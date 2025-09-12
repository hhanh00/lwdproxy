use serde::Deserialize;

#[derive(Clone, Deserialize, Debug, Default)]
pub struct Config {
    pub db_path: String,
    pub origin: Vec<String>,
    pub bind_address: String,
    pub port: u16,
    pub min_height: u32,
    pub tls: bool,
    pub cert_path: String,
    pub key_path: String,
}
