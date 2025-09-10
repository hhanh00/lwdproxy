use anyhow::Result;
use figment::{providers::{Format as _, Json}, Figment};
use lfrb::{api::init::{init_app, set_config}, config::Config, server::start_server};
use tracing::info;
use lfrb::api::init::CONFIG;
use zcash_protocol::consensus::{Network, Parameters};

#[tokio::main]
async fn main() -> Result<()> {
    init_app();

    let figment = Figment::new()
        .join(Json::file("App.json"));
    let config: Config = figment.extract()?;
    set_config(config);

    info!("{:?}", &CONFIG);
    let orchard_height: u32 = Network::MainNetwork.activation_height(zcash_protocol::consensus::NetworkUpgrade::Nu5).unwrap().into();
    info!("{orchard_height}");

    // let lwd = LightwalletD::new().await?;
    // lwd.sync().await?;

    start_server().await?;

    Ok(())
}
