use anyhow::Result;

use crate::client;
use crate::config::NostaroConfig;
use crate::keys;

pub async fn run(message: &str) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    println!("Publishing note...");
    client::post_note(&nostr_client, message).await?;
    println!("Note published successfully!");

    Ok(())
}
