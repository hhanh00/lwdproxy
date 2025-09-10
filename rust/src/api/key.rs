use anyhow::Result;
use flutter_rust_bridge::frb;
use orchard::keys::{FullViewingKey, SpendingKey};
use zcash_address::unified::{Encoding, Ufvk};
use zcash_protocol::consensus::NetworkType;

#[frb]
pub async fn derive_key(idx: u32) -> Result<String> {
    let mut k = [0u8; 32];
    k[0..4].copy_from_slice(&idx.to_le_bytes());
    let sk = SpendingKey::from_bytes(k).unwrap();
    let fvk = FullViewingKey::from(&sk);
    let ofvk = zcash_address::unified::Fvk::Orchard(fvk.to_bytes());
    let ufvk = Ufvk::try_from_items(vec![ofvk])?;
    let key = ufvk.encode(&NetworkType::Main);
    Ok(key)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_derive_key() {
        let k = super::derive_key(1).await.unwrap();
        println!("{k}");
    }
}
