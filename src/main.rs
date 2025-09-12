use anyhow::Result;
use figment::{providers::{Format as _, Json}, Figment};
use lwdproxy_rs::{api::init::{init_app, set_config}, Config, server::start_server};
use tracing::info;
use lwdproxy_rs::api::init::CONFIG;

#[tokio::main]
async fn main() -> Result<()> {
    init_app();

    let figment = Figment::new()
        .join(Json::file("App.json"));
    let config: Config = figment.extract()?;
    set_config(config);

    info!("{:?}", &CONFIG);
    start_server().await?;

    Ok(())
}
