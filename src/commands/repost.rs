use anyhow::{anyhow, Result};
use nostr_sdk::prelude::*;

use crate::client;
use crate::config::NostaroConfig;
use crate::keys;

pub async fn run(note_id: &str) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let event_id = EventId::parse(note_id)?;

    let target_event = client::fetch_event_by_id(&nostr_client, &event_id)
        .await?
        .ok_or_else(|| anyhow!("Event not found: {}", note_id))?;

    println!("Reposting {}...", &event_id.to_hex()[..8]);
    client::repost_event(&nostr_client, &target_event).await?;
    println!("Reposted successfully!");

    Ok(())
}
