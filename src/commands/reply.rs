use anyhow::{anyhow, Result};
use nostr_sdk::prelude::*;

use crate::client;
use crate::config::NostaroConfig;
use crate::keys;

pub async fn run(note_id: &str, message: &str) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let event_id = EventId::parse(note_id)
        .or_else(|_| EventId::from_bech32(note_id))?;

    let target_event = client::fetch_event_by_id(&nostr_client, &event_id)
        .await?
        .ok_or_else(|| anyhow!("Event not found: {}", note_id))?;

    println!("Replying to {}...", &event_id.to_hex()[..8]);
    client::reply_note(&nostr_client, &target_event, message).await?;
    println!("Reply published successfully!");

    Ok(())
}
