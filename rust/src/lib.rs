use thiserror::Error;
use tonic::transport::Channel;

use crate::rpc::compact_tx_streamer_client::CompactTxStreamerClient;

pub mod config;
pub mod api;
#[path ="cash.z.wallet.sdk.rpc.rs"]
pub mod rpc;
pub mod db;
pub mod server;
mod frb_generated;

pub type Client = CompactTxStreamerClient<Channel>;

#[derive(Error, Debug)]
pub enum SyncError {
    #[error("Reorg")]
    Reorg,
    #[error(transparent)]
    Db(#[from] heed::Error),
    #[error(transparent)]
    Tonic(#[from] tonic::Status),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}