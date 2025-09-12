use thiserror::Error;
use tonic::transport::Channel;

use crate::rpc::compact_tx_streamer_client::CompactTxStreamerClient;

pub mod api;
#[path ="cash.z.wallet.sdk.rpc.rs"]
pub mod rpc;
pub mod db;
pub mod server;

pub type Client = CompactTxStreamerClient<Channel>;
pub use api::config::Config;

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