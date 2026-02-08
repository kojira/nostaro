use anyhow::Result;
use chrono::{DateTime, Utc};
use nostr_sdk::prelude::*;

use crate::client;
use crate::config::NostaroConfig;
use crate::keys;

pub async fn run(query: &str, limit: usize) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    println!("Searching for \"{}\"...\n", query);

    let events = client::search_notes(&nostr_client, query, limit).await?;

    if events.is_empty() {
        println!("No notes found matching \"{}\".", query);
        return Ok(());
    }

    for event in &events {
        let npub = event.pubkey.to_bech32()?;
        let short_npub = &npub[..12.min(npub.len())];
        let timestamp = event.created_at.as_u64() as i64;
        let datetime = DateTime::<Utc>::from_timestamp(timestamp, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "unknown".to_string());

        println!("[{}] {}", short_npub, datetime);
        println!("{}", event.content);
        println!("{}", "-".repeat(60));
    }

    println!("\nFound {} note(s).", events.len());

    Ok(())
}
