#![allow(unused_variables)]
use std::sync::Arc;

use anyhow::Result;
use flutter_rust_bridge::frb;
use heed::{
    byteorder::BE,
    types::*,
    Database, Env, EnvOpenOptions,
};
use prost::Message;
use tokio::sync::Mutex;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{
    transport::{Channel, ClientTlsConfig, Endpoint, Server},
    Request, Response, Status,
};
use tracing::info;
use zcash_protocol::consensus::{Network, NetworkUpgrade, Parameters};

use crate::{
    api::init::config,
    rpc::{
        compact_tx_streamer_client::CompactTxStreamerClient,
        compact_tx_streamer_server::{CompactTxStreamer, CompactTxStreamerServer},
        *,
    },
    Client,
};

pub struct LightwalletDState {
    pub network: Network,
    pub env: Env,
    pub channel: Channel,
}

impl LightwalletDState {
    pub async fn new() -> Result<Self> {
        let c = config();
        // db connection
        let env = unsafe {
            EnvOpenOptions::new()
                .map_size(32 * 1024 * 1024 * 1024)
                .open(c.db_path.clone())?
        };
        // grpc origin load balanced channel
        let endpoints: Vec<_> = c
            .origin
            .iter()
            .map(|url| {
                let tls_config = ClientTlsConfig::new().with_enabled_roots();
                Endpoint::from_shared(url.to_string())
                    .unwrap_or_else(|_| panic!("Invalid endpoint: {url}"))
                    .tls_config(tls_config)
                    .expect("TLS cannot be enabled")
            })
            .collect();
        let channel = Channel::balance_list(endpoints.into_iter());

        let state = LightwalletDState {
            network: Network::MainNetwork,
            env,
            channel,
        };
        Ok(state)
    }
}

pub struct LightwalletD {
    pub state: Arc<Mutex<LightwalletDState>>,
}

impl LightwalletD {
    pub async fn new() -> Result<Self> {
        let state = LightwalletDState::new().await?;
        Ok(Self {
            state: Arc::new(Mutex::new(state)),
        })
    }

    pub async fn client(&self) -> Result<Client> {
        let state = self.state.lock().await;
        let client = CompactTxStreamerClient::new(state.channel.clone());
        Ok(client)
    }

    pub async fn sync(&self) -> Result<()> {
        let network = {
            let state = self.state.lock().await;
            state.network
        };
        info!("Network: {:?}", network);
        let mut client = self.client().await?;
        let id = client
            .get_latest_block(Request::new(ChainSpec {}))
            .await?
            .into_inner();
        let end = id.height as u32;
        let start = {
            let state = self.state.lock().await;
            let rtxn = state.env.read_txn()?;
            let blocks_table: Database<U32<BE>, Bytes> =
                state.env.open_database(&rtxn, None)?.unwrap();
            let max_height = blocks_table
                .last(&rtxn)?
                .map(|(h, _)| h + 1)
                .unwrap_or_else(|| {
                    network
                        .activation_height(NetworkUpgrade::Sapling)
                        .unwrap()
                        .into()
                });
            max_height
        };

        info!("{start} {end}");
        if start > end {
            info!("Database is up to date");
            return Ok(());
        }

        let range = BlockRange {
            start: Some(BlockId {
                height: start as u64,
                hash: vec![],
            }),
            end: Some(BlockId {
                height: end as u64,
                hash: vec![],
            }),
            spam_filter_threshold: 0,
        };
        let mut report_timer = tokio::time::interval(std::time::Duration::from_secs(15));

        let mut blocks = client
            .get_block_range(Request::new(range))
            .await?
            .into_inner();
        let mut h = 0u32;
        let state = self.state.lock().await;
        let mut wtxn = state.env.write_txn()?;
        // Database can outlive the wtxn in lmdb
        let blocks_table: Database<U32<BE>, Bytes> = state.env.create_database(&mut wtxn, None)?;
        loop {
            tokio::select! {
                block = blocks.message() => {
                    if let Some(block) = block? {
                        h = block.height as u32;
                        if h % 100_000 == 0 { info!("Syncing block @{h}"); }
                        blocks_table.put(&mut wtxn, &h, block.encode_to_vec().as_slice())?;
                    }
                    else {
                        break;
                    }
                }
                _ = report_timer.tick() => {
                    info!("Commit @{h}");
                    wtxn.commit()?;
                    wtxn = state.env.write_txn()?;
                }
            }
        }
        wtxn.commit()?;

        Ok(())
    }
}

pub type GRPCResult<T> = Result<T, Status>;

#[tonic::async_trait]
impl CompactTxStreamer for LightwalletD {
    /// Return the height of the tip of the best chain
    async fn get_latest_block(&self, request: Request<ChainSpec>) -> GRPCResult<Response<BlockId>> {
        todo!()
    }
    /// Return the compact block corresponding to the given block identifier
    async fn get_block(&self, request: Request<BlockId>) -> GRPCResult<Response<CompactBlock>> {
        todo!()
    }
    /// Server streaming response type for the GetBlockRange method.
    type GetBlockRangeStream = ReceiverStream<GRPCResult<CompactBlock>>;

    /// Return a list of consecutive compact blocks
    async fn get_block_range(
        &self,
        request: Request<BlockRange>,
    ) -> GRPCResult<Response<Self::GetBlockRangeStream>> {
        todo!()
    }
    /// Return the requested full (not compact) transaction (as from zcashd)
    async fn get_transaction(
        &self,
        request: Request<TxFilter>,
    ) -> GRPCResult<Response<RawTransaction>> {
        todo!()
    }
    /// Submit the given transaction to the Zcash network
    async fn send_transaction(
        &self,
        request: Request<RawTransaction>,
    ) -> GRPCResult<Response<SendResponse>> {
        todo!()
    }
    /// Server streaming response type for the GetTaddressTxids method.
    type GetTaddressTxidsStream = ReceiverStream<GRPCResult<RawTransaction>>;

    /// Return the txids corresponding to the given t-address within the given block range
    async fn get_taddress_txids(
        &self,
        request: Request<TransparentAddressBlockFilter>,
    ) -> GRPCResult<Response<Self::GetTaddressTxidsStream>> {
        todo!()
    }
    async fn get_taddress_balance(
        &self,
        request: Request<AddressList>,
    ) -> GRPCResult<Response<Balance>> {
        todo!()
    }
    async fn get_taddress_balance_stream(
        &self,
        request: Request<tonic::Streaming<Address>>,
    ) -> GRPCResult<Response<Balance>> {
        todo!()
    }
    /// Server streaming response type for the GetMempoolTx method.
    type GetMempoolTxStream = ReceiverStream<GRPCResult<CompactTx>>;

    /// Return the compact transactions currently in the mempool{ todo!() } the GRPCResults
    /// can be a few seconds out of date. If the Exclude list is empty, return
    /// all transactions{ todo!() } otherwise return all *except* those in the Exclude list
    /// (if any){ todo!() } this allows the client to avoid receiving transactions that it
    /// already has (from an earlier call to this rpc). The transaction IDs in the
    /// Exclude list can be shortened to any number of bytes to make the request
    /// more bandwidth-efficient{ todo!() } if two or more transactions in the mempool
    /// match a shortened txid, they are all sent (none is excluded). Transactions
    /// in the exclude list that don't exist in the mempool are ignored.
    async fn get_mempool_tx(
        &self,
        request: Request<Exclude>,
    ) -> GRPCResult<Response<Self::GetMempoolTxStream>> {
        todo!()
    }
    /// Server streaming response type for the GetMempoolStream method.
    type GetMempoolStreamStream = ReceiverStream<GRPCResult<RawTransaction>>;

    /// Return a stream of current Mempool transactions. This will keep the output stream open while
    /// there are mempool transactions. It will close the returned stream when a new block is mined.
    async fn get_mempool_stream(
        &self,
        request: Request<Empty>,
    ) -> GRPCResult<Response<Self::GetMempoolStreamStream>> {
        todo!()
    }
    /// GetTreeState returns the note commitment tree state corresponding to the given block.
    /// See section 3.7 of the Zcash protocol specification. It returns several other useful
    /// values also (even though they can be obtained using GetBlock).
    /// The block can be specified by either height or hash.
    async fn get_tree_state(&self, request: Request<BlockId>) -> GRPCResult<Response<TreeState>> {
        todo!()
    }
    async fn get_address_utxos(
        &self,
        request: Request<GetAddressUtxosArg>,
    ) -> GRPCResult<Response<GetAddressUtxosReplyList>> {
        todo!()
    }
    /// Server streaming response type for the GetAddressUtxosStream method.
    type GetAddressUtxosStreamStream = ReceiverStream<GRPCResult<GetAddressUtxosReply>>;

    async fn get_address_utxos_stream(
        &self,
        request: Request<GetAddressUtxosArg>,
    ) -> GRPCResult<Response<Self::GetAddressUtxosStreamStream>> {
        todo!()
    }
    /// Return information about this lightwalletd instance and the blockchain
    async fn get_lightd_info(&self, request: Request<Empty>) -> GRPCResult<Response<LightdInfo>> {
        todo!()
    }
    /// Testing-only, requires lightwalletd --ping-very-insecure (do not enable in production)
    async fn ping(&self, request: Request<Duration>) -> GRPCResult<Response<PingResponse>> {
        todo!()
    }
}

#[frb]
pub async fn start_server() -> Result<()> {
    let c = config();
    let addr = format!("{}:{}", c.bind_address, c.port).parse()?;
    let lwd = LightwalletD::new().await?;
    lwd.sync().await?;

    info!("GreeterServer listening on {}", addr);

    Server::builder()
        .add_service(CompactTxStreamerServer::new(lwd))
        .serve(addr)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    // use super::*;
    // use heed::{
    //     byteorder::LE,
    //     types::{Bytes, U32},
    //     Database, EnvOpenOptions,
    // };
    // use sqlx::{
    //     sqlite::{SqliteConnectOptions, SqliteRow},
    //     Row, SqlitePool,
    // };
    // use tokio_stream::StreamExt;

    #[tokio::test]
    async fn migrate() -> anyhow::Result<()> {
        // let lwd = LightwalletD::new().await?;
        // lwd.sync().await?;

        // let blocks_table: TableDefinition<u32, &[u8]> = TableDefinition::new("blocks");
        // let odb = Database::create("zec.redb")?;
        // let wtxn = odb.begin_write()?;
        // let mut blocks = wtxn.open_table(blocks_table)?;
        // let options = SqliteConnectOptions::new().filename("lwd.db");
        // let idb = SqlitePool::connect_with(options).await?;
        // let mut idb = idb.acquire().await?;
        // let mut data = sqlx::query("SELECT height, block FROM compact_blocks ORDER BY height")
        // .map(|r: SqliteRow| {
        //     let height: u32 = r.get(0);
        //     let block: Vec<u8> = r.get(1);
        //     (height, block)
        // })
        // .fetch(&mut *idb);

        // while let Some(data) = data.next().await {
        //     let (height, block) = data?;
        //     if height % 10000 == 0 {
        //         println!("{height}");
        //     }
        //     blocks.insert(height, &block.as_slice())?;
        // }
        Ok(())
    }

    #[tokio::test]
    async fn migrate2() -> anyhow::Result<()> {
        // let env = unsafe { EnvOpenOptions::new()
        //     .map_size(20*1024*1024*1024)
        //     .open("zec.mdb")? };
        // let mut wtxn = env.write_txn()?;
        // let blocks_table: Database<U32<BE>, Bytes> = env.create_database(&mut wtxn, None)?;
        // let options = SqliteConnectOptions::new().filename("/Volumes/External2TB/lwd.db");
        // let idb = SqlitePool::connect_with(options).await?;
        // let mut idb = idb.acquire().await?;
        // let mut data = sqlx::query("SELECT height, block FROM compact_blocks ORDER BY height")
        // .map(|r: SqliteRow| {
        //     let height: u32 = r.get(0);
        //     let block: Vec<u8> = r.get(1);
        //     (height, block)
        // })
        // .fetch(&mut *idb);

        // while let Some(data) = data.next().await {
        //     let (height, block) = data?;
        //     if height % 10000 == 0 {
        //         println!("{height}");
        //     }
        //     blocks_table.put(&mut wtxn, &height, &block.as_slice())?;
        // }
        // wtxn.commit()?;
        Ok(())
    }
}
