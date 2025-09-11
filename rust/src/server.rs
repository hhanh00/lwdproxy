#![allow(unused_variables)]

use anyhow::Result;
use futures_util::TryStreamExt as _;
use heed::{byteorder::BE, types::*, Database, Env, EnvOpenOptions};
use prost::Message;
use tokio::{
    runtime::Builder,
    sync::{mpsc, Mutex},
    task::LocalSet,
};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{
    transport::{Channel, ClientTlsConfig, Endpoint, Server},
    Request, Response, Status, Streaming,
};
use tracing::info;
use zcash_protocol::consensus::{BranchId, Network, NetworkUpgrade, Parameters};

use crate::{
    api::init::config,
    rpc::{
        compact_tx_streamer_client::CompactTxStreamerClient,
        compact_tx_streamer_server::{CompactTxStreamer, CompactTxStreamerServer},
        *,
    },
    Client, SyncError,
};

macro_rules! env {
    ($self:ident) => {{
        let state = $self.state.lock().await;
        state.env.clone()
    }};
}

macro_rules! rtxn {
    ($env:ident) => {{
        let rtxn = $env.read_txn().map_err(into_status)?;
        let blocks_table: Database<U32<BE>, Bytes> = $env
            .open_database(&rtxn, None)
            .map_err(into_status)?
            .unwrap();
        (blocks_table, rtxn)
    }};
}

#[derive(Clone)]
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

    pub async fn rewind(&self) -> Result<()> {
        let mut wtxn = self.env.write_txn()?;
        let blocks_table: Database<U32<BE>, Bytes> = self.env.create_database(&mut wtxn, None)?;
        if let Some((last, _)) = blocks_table.last(&mut wtxn)? {
            // drop last 100 blocks, a reorg will never be longer than that
            let height = last.saturating_sub(100);
            blocks_table.delete_range(&mut wtxn, &(height..=last))?;
        }
        wtxn.commit()?;
        Ok(())
    }

    pub async fn sync(&self) -> Result<(), SyncError> {
        let min_height = config().min_height;
        let network = self.network;
        let mut client = CompactTxStreamerClient::new(self.channel.clone());
        let id = client
            .get_latest_block(Request::new(ChainSpec {}))
            .await?
            .into_inner();
        let end = id.height as u32;
        let (start, mut prevhash) = {
            let rtxn = self.env.read_txn()?;
            let blocks_table: Database<U32<BE>, Bytes> =
                self.env.open_database(&rtxn, None)?.unwrap();
            let (max_height, data) = blocks_table
                .last(&rtxn)?
                .map(|(h, hash)| (h + 1, Some(hash.to_vec())))
                .unwrap_or_else(|| {
                    let activation_height: u32 = network
                        .activation_height(NetworkUpgrade::Sapling)
                        .unwrap()
                        .into();
                    (activation_height.max(min_height), None)
                });
            let prevhash = data.map(|d| {
                let block = CompactBlock::decode(&*d).unwrap();
                block.hash
            });
            (max_height, prevhash)
        };

        if start > end {
            return Ok(());
        }
        info!("Processing from {start} to {end}");

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
        let mut wtxn = self.env.write_txn()?;
        // Database can outlive the wtxn in lmdb
        let blocks_table: Database<U32<BE>, Bytes> = self.env.create_database(&mut wtxn, None)?;
        loop {
            tokio::select! {
                block = blocks.message() => {
                    if let Some(block) = block? {
                        h = block.height as u32;
                        let block_prevhash = block.prev_hash.clone();
                        if let Some(prevhash) = &prevhash {
                            if *prevhash != block_prevhash {
                                info!("REORG {h} exp {}, got {}", hex::encode(prevhash), hex::encode(block_prevhash));
                                return Err(SyncError::Reorg);
                            }
                        }
                        prevhash = Some(block.hash.clone());
                        if h % 100_000 == 0 { info!("Syncing block @{h}"); }
                        blocks_table.put(&mut wtxn, &h, block.encode_to_vec().as_slice())?;
                    }
                    else {
                        break;
                    }
                }
                _ = report_timer.tick() => {
                    info!("Checkpoint @{h}");
                    wtxn.commit()?;
                    wtxn = self.env.write_txn()?;
                }
            }
        }
        wtxn.commit()?;
        info!("Checkpoint @{h}");

        Ok(())
    }

    pub fn run(&self) -> Result<()> {
        let state = self.clone();
        tokio::task::spawn_blocking(move || -> Result<(), anyhow::Error> {
            loop {
                let runtime = Builder::new_current_thread().enable_time().build().unwrap();
                runtime.block_on(async {
                    let result = state.sync().await;
                    match result {
                        Ok(r) => Ok(()),
                        Err(SyncError::Reorg) => state.rewind().await,
                        Err(SyncError::Db(e)) => Err(anyhow::Error::new(e)),
                        Err(SyncError::Tonic(e)) => Err(anyhow::Error::new(e)),
                        Err(SyncError::Other(e)) => Err(e),
                    }
                })?;
                std::thread::sleep(std::time::Duration::from_secs(15));
            }
        });
        Ok(())
    }
}

pub struct LightwalletD {
    pub state: Mutex<LightwalletDState>,
    pub tx_job: mpsc::Sender<RangeJob>,
}

impl LightwalletD {
    pub async fn build() -> Result<Self> {
        const CONCURRENT_RANGE_JOBS: usize = 16;

        let state = LightwalletDState::new().await?;
        let mut tx_child_jobs = vec![];
        for i in 0..CONCURRENT_RANGE_JOBS {
            let (tx_child_job, mut rx_child_job) = mpsc::channel::<RangeJob>(4);
            // dedicated ROTxn worker thread because LMDB read txns are not Send
            // We cannot send the CompactBlock results back to the grpc worker thread
            // The tonic grpc worker thread may be busy waiting for the client to receive
            // and yielding to other grpc requests
            // there is an await point between each block send
            // We use a thread pool because otherwise we wouldn't be able to process
            // multiple GetBlockRange concurrently
            std::thread::spawn(move || {
                let local = LocalSet::new();
                tokio::runtime::Builder::new_current_thread()
                    .build()
                    .unwrap()
                    .block_on(local.run_until(async move {
                        while let Some(job) = rx_child_job.recv().await {
                            let RangeJob {
                                env,
                                start,
                                end,
                                tx,
                            } = job;
                            let (blocks_table, rtxn) = rtxn!(env);
                            let blocks = blocks_table.range(&rtxn, &(start..=end))?;
                            for block in blocks {
                                let (h, data) = block?;
                                let block = CompactBlock::decode(data)?;
                                let _ = tx.send(Ok(block)).await;
                            }
                        }
                        info!("Range job worker stopped");
                        Ok::<_, anyhow::Error>(())
                    }))
                    .unwrap();
            });
            tx_child_jobs.push(tx_child_job);
        }
        let (tx_job, mut rx_job) = mpsc::channel::<RangeJob>(CONCURRENT_RANGE_JOBS);
        tokio::spawn(async move {
            let mut i = 0;
            while let Some(job) = rx_job.recv().await {
                let tx_child_job = &tx_child_jobs[i % tx_child_jobs.len()];
                let _ = tx_child_job.send(job).await;
                i += 1;
            }
            Ok::<_, anyhow::Error>(())
        });

        state.run()?; // Run the autosync task

        Ok(Self {
            state: Mutex::new(state),
            tx_job,
        })
    }

    pub async fn client(&self) -> Result<Client> {
        let state = self.state.lock().await;
        let client = CompactTxStreamerClient::new(state.channel.clone());
        Ok(client)
    }
}

pub type GRPCResult<T> = Result<T, Status>;

fn into_status(e: impl std::error::Error) -> Status {
    Status::internal(e.to_string())
}

pub struct RangeJob {
    env: Env,
    start: u32,
    end: u32,
    tx: mpsc::Sender<GRPCResult<CompactBlock>>,
}

pub async fn forward<T, R>(
    lwd: &LightwalletD,
    request: Request<T>,
    f: impl AsyncFnOnce(Client, Request<T>) -> Result<Response<R>>,
) -> GRPCResult<Response<R>> {
    let r = async {
        let client = lwd.client().await?;
        let tx = f(client, request).await?;
        Ok::<_, anyhow::Error>(tx)
    };
    r.await.map_err(|e| into_status(e.root_cause()))
}

macro_rules! forward {
    ($self:ident, $request:ident, $method:ident) => {{
        forward($self, $request, async |mut client, request| {
            let result = client.$method(request).await?;
            Ok::<_, anyhow::Error>(result)
        })
        .await
    }};
}

pub async fn forward_stream<T, R>(
    lwd: &LightwalletD,
    request: Request<T>,
    f: impl AsyncFnOnce(Client, Request<T>) -> Result<Response<Streaming<R>>>,
) -> GRPCResult<Response<ReceiverStream<GRPCResult<R>>>> {
    let rx = async move {
        let (tx, rx) = tokio::sync::mpsc::channel::<GRPCResult<R>>(4);
        let client = lwd.client().await?;
        let mut rep = f(client, request).await?.into_inner();
        while let Some(rtx) = rep.message().await? {
            let _ = tx.send(Ok(rtx)).await;
        }
        Ok::<_, anyhow::Error>(rx)
    };
    let rx = rx.await.map_err(|e| into_status(e.root_cause()))?;
    Ok(Response::new(ReceiverStream::new(rx)))
}

macro_rules! forward_stream {
    ($self:ident, $request:ident, $method:ident) => {{
        forward_stream($self, $request, async |mut client, request| {
            let result = client.$method(request).await?;
            Ok::<_, anyhow::Error>(result)
        })
        .await
    }};
}

#[tonic::async_trait]
impl CompactTxStreamer for LightwalletD {
    /// Return the height of the tip of the best chain
    async fn get_latest_block(&self, request: Request<ChainSpec>) -> GRPCResult<Response<BlockId>> {
        let env = env!(self);
        let (blocks_table, rtxn) = rtxn!(env);
        let last_block = blocks_table
            .last(&rtxn)
            .map_err(into_status)?
            .map(|(h, data)| {
                // Decode the block to get its hash (cannot fail as it was encoded before storing)
                let block = CompactBlock::decode(data).map_err(into_status).unwrap();
                BlockId {
                    height: h as u64,
                    hash: block.hash.clone(),
                }
            })
            .unwrap_or_default();
        Ok(Response::new(last_block))
    }
    /// Return the compact block corresponding to the given block identifier
    async fn get_block(&self, request: Request<BlockId>) -> GRPCResult<Response<CompactBlock>> {
        let request = request.into_inner();
        let env = env!(self);
        let (blocks_table, rtxn) = rtxn!(env);
        let block_data = blocks_table
            .get(&rtxn, &(request.height as u32))
            .map_err(into_status)?;
        let block_data = block_data.ok_or_else(|| Status::not_found("Block not found"))?;
        let block = CompactBlock::decode(block_data).unwrap();
        Ok(Response::new(block))
    }
    /// Server streaming response type for the GetBlockRange method.
    type GetBlockRangeStream = ReceiverStream<GRPCResult<CompactBlock>>;

    /// Return a list of consecutive compact blocks
    async fn get_block_range(
        &self,
        request: Request<BlockRange>,
    ) -> GRPCResult<Response<Self::GetBlockRangeStream>> {
        let request = request.into_inner();
        let start = request
            .start
            .ok_or_else(|| Status::invalid_argument("Start block is required"))?;
        let end = request
            .end
            .ok_or_else(|| Status::invalid_argument("End block is required"))?;
        let start = start.height as u32;
        let end = end.height as u32;

        let (tx, rx) = tokio::sync::mpsc::channel::<GRPCResult<CompactBlock>>(4);
        let env = env!(self);
        let tx_job = self.tx_job.clone();
        tokio::spawn(async move {
            let job = RangeJob {
                env,
                start,
                end,
                tx,
            };
            let _ = tx_job.send(job).await;
            Ok::<_, anyhow::Error>(())
        });
        let recv = ReceiverStream::new(rx);
        Ok(Response::new(recv))
    }
    /// Return the requested full (not compact) transaction (as from zcashd)
    async fn get_transaction(
        &self,
        request: Request<TxFilter>,
    ) -> GRPCResult<Response<RawTransaction>> {
        forward!(self, request, get_transaction)
    }
    /// Submit the given transaction to the Zcash network
    async fn send_transaction(
        &self,
        request: Request<RawTransaction>,
    ) -> GRPCResult<Response<SendResponse>> {
        forward!(self, request, send_transaction)
    }
    /// Server streaming response type for the GetTaddressTxids method.
    type GetTaddressTxidsStream = ReceiverStream<GRPCResult<RawTransaction>>;

    /// Return the txids corresponding to the given t-address within the given block range
    async fn get_taddress_txids(
        &self,
        request: Request<TransparentAddressBlockFilter>,
    ) -> GRPCResult<Response<Self::GetTaddressTxidsStream>> {
        forward_stream!(self, request, get_taddress_txids)
    }
    async fn get_taddress_balance(
        &self,
        request: Request<AddressList>,
    ) -> GRPCResult<Response<Balance>> {
        forward!(self, request, get_taddress_balance)
    }
    async fn get_taddress_balance_stream(
        &self,
        request: Request<tonic::Streaming<Address>>,
    ) -> GRPCResult<Response<Balance>> {
        let request = request.into_inner();
        let request: Vec<_> = request.try_collect().await?;
        let request = tokio_stream::iter(request);
        let req = Request::new(request);
        let mut client = self
            .client()
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        let rep = client.get_taddress_balance_stream(req).await?;
        Ok(rep)
    }
    /// Server streaming response type for the GetMempoolTx method.
    type GetMempoolTxStream = ReceiverStream<GRPCResult<CompactTx>>;

    /// Return the compact transactions currently in the mempool the GRPCResults
    /// can be a few seconds out of date. If the Exclude list is empty, return
    /// all transactions otherwise return all *except* those in the Exclude list
    /// (if any) this allows the client to avoid receiving transactions that it
    /// already has (from an earlier call to this rpc). The transaction IDs in the
    /// Exclude list can be shortened to any number of bytes to make the request
    /// more bandwidth-efficient if two or more transactions in the mempool
    /// match a shortened txid, they are all sent (none is excluded). Transactions
    /// in the exclude list that don't exist in the mempool are ignored.
    async fn get_mempool_tx(
        &self,
        request: Request<Exclude>,
    ) -> GRPCResult<Response<Self::GetMempoolTxStream>> {
        forward_stream!(self, request, get_mempool_tx)
    }
    /// Server streaming response type for the GetMempoolStream method.
    type GetMempoolStreamStream = ReceiverStream<GRPCResult<RawTransaction>>;

    /// Return a stream of current Mempool transactions. This will keep the output stream open while
    /// there are mempool transactions. It will close the returned stream when a new block is mined.
    async fn get_mempool_stream(
        &self,
        request: Request<Empty>,
    ) -> GRPCResult<Response<Self::GetMempoolStreamStream>> {
        forward_stream!(self, request, get_mempool_stream)
    }
    /// GetTreeState returns the note commitment tree state corresponding to the given block.
    /// See section 3.7 of the Zcash protocol specification. It returns several other useful
    /// values also (even though they can be obtained using GetBlock).
    /// The block can be specified by either height or hash.
    async fn get_tree_state(&self, request: Request<BlockId>) -> GRPCResult<Response<TreeState>> {
        forward!(self, request, get_tree_state)
    }
    async fn get_address_utxos(
        &self,
        request: Request<GetAddressUtxosArg>,
    ) -> GRPCResult<Response<GetAddressUtxosReplyList>> {
        forward!(self, request, get_address_utxos)
    }
    /// Server streaming response type for the GetAddressUtxosStream method.
    type GetAddressUtxosStreamStream = ReceiverStream<GRPCResult<GetAddressUtxosReply>>;

    async fn get_address_utxos_stream(
        &self,
        request: Request<GetAddressUtxosArg>,
    ) -> GRPCResult<Response<Self::GetAddressUtxosStreamStream>> {
        forward_stream!(self, request, get_address_utxos_stream)
    }
    /// Return information about this lightwalletd instance and the blockchain
    async fn get_lightd_info(&self, request: Request<Empty>) -> GRPCResult<Response<LightdInfo>> {
        let sapling_activation_height: u64 = Network::MainNetwork
            .activation_height(NetworkUpgrade::Sapling)
            .unwrap()
            .into();
        let consensus_branch_id: u32 = BranchId::Nu6.into();
        let env = env!(self);
        let (blocks_table, rtxn) = rtxn!(env);
        let latest_height = blocks_table
            .last(&rtxn)
            .map_err(into_status)?
            .map(|(h, _)| h)
            .unwrap_or_default();
        let info = LightdInfo {
            version: "LWD Proxy 1.0.0".to_string(),
            vendor: "hanh".to_string(),
            taddr_support: true,
            chain_name: "main".to_string(),
            sapling_activation_height,
            consensus_branch_id: hex::encode(consensus_branch_id.to_le_bytes()),
            block_height: latest_height as u64,
            git_commit: "".to_string(),
            branch: "".to_string(),
            estimated_height: latest_height as u64,
            build_date: "".to_string(),
            build_user: "".to_string(),
            zcashd_build: "".to_string(),
            zcashd_subversion: "".to_string(),
        };
        Ok(Response::new(info))
    }
    /// Testing-only, requires lightwalletd --ping-very-insecure (do not enable in production)
    async fn ping(&self, request: Request<Duration>) -> GRPCResult<Response<PingResponse>> {
        Ok(Response::new(PingResponse { entry: 0, exit: 0 }))
    }
}

pub async fn start_server() -> Result<()> {
    let c = config();
    let addr = format!("{}:{}", c.bind_address, c.port).parse()?;
    let lwd = LightwalletD::build().await?;

    info!("GreeterServer listening on {}", addr);

    let local = LocalSet::new();
    local
        .run_until(async move {
            Server::builder()
                .add_service(CompactTxStreamerServer::new(lwd))
                .serve(addr)
                .await
        })
        .await?;

    Ok(())
}
