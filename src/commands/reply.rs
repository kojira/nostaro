use anyhow::{anyhow, Result};
use nostr_sdk::prelude::*;

use crate::client;
use crate::commands::emit;
use crate::config::NostaroConfig;
use crate::keys;

pub async fn run(note_id: &str, message: &str, json: bool) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let event_id = EventId::parse(note_id).or_else(|_| EventId::from_bech32(note_id))?;

    let target_event = client::fetch_event_by_id(&nostr_client, &event_id)
        .await?
        .ok_or_else(|| anyhow!("Event not found: {}", note_id))?;

    emit::status(json, &format!("Replying to {}...", &event_id.to_hex()[..8]));
    let published_id = client::reply_note(&nostr_client, &target_event, message).await?;
    emit::status(json, "Reply published successfully!");
    // Issue #18: additive machine-readable id (text `Event id:` line or JSON).
    emit::emit_event_id(json, &published_id);

    nostr_client.disconnect().await;
    Ok(())
}
