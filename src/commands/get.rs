use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use nostr_sdk::prelude::*;
use std::time::Duration;

use crate::client;
use crate::config::NostaroConfig;
use crate::keys;

pub async fn run(event_id_str: &str) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    // Parse event ID: hex, note1 bech32, or nevent1 bech32
    let (event_id, relay_hints) = if event_id_str.starts_with("nevent1") {
        let nip19_event = Nip19Event::from_bech32(event_id_str)?;
        (nip19_event.event_id, nip19_event.relays)
    } else {
        let id = EventId::parse(event_id_str)
            .or_else(|_| EventId::from_bech32(event_id_str))?;
        (id, vec![])
    };

    // Add relay hints if present
    for relay in &relay_hints {
        let _ = nostr_client.add_relay(relay).await;
    }
    if !relay_hints.is_empty() {
        nostr_client.connect().await;
    }

    // Fetch the event
    let event = client::fetch_event_by_id(&nostr_client, &event_id)
        .await?
        .ok_or_else(|| anyhow!("Event not found: {}", event_id_str))?;

    // Display event details
    println!("Event ID:    {}", event.id.to_hex());
    println!("Author:      {}", event.pubkey.to_bech32()?);

    let timestamp = event.created_at.as_u64() as i64;
    let datetime = DateTime::<Utc>::from_timestamp(timestamp, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("Created at:  {}", datetime);

    println!("Kind:        {}", event.kind.as_u16());
    println!("Content:     {}", event.content);

    // Fetch reactions
    let reaction_filter = Filter::new()
        .kind(Kind::Reaction)
        .event(event_id)
        .limit(100);
    let reactions = nostr_client
        .fetch_events(reaction_filter, Duration::from_secs(10))
        .await?;
    let reaction_count = reactions.into_iter().count();
    println!("Reactions:   {}", reaction_count);

    nostr_client.disconnect().await;
    Ok(())
}
