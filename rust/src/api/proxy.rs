use anyhow::Result;
use flutter_rust_bridge::frb;
use tracing::info;

#[frb]
pub async fn start_proxy(origin: &str) -> Result<()> {
    info!("Start proxy to {origin}");
    Ok(())
}
