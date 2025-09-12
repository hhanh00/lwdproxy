use anyhow::Result;
use figment::{providers::{Format as _, Json}, Figment};
use flutter_rust_bridge::frb;
use tracing::info;

use crate::{api::init::{set_config, CONFIG}, Config, server::start_server};

#[frb(sync)]
pub fn get_default_config() -> Result<Config> {
    let figment = Figment::new()
        .join(Json::file("App.json"));
    let config: Config = figment.extract()?;
    Ok(config)
}

#[frb]
pub async fn start_proxy(config: &Config) -> Result<()> {
    set_config(config.clone());
    tokio::task::spawn_blocking(move || {
        info!("Starting server with config: {:?}", CONFIG);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(start_server()).unwrap();
    });
    Ok(())
}
