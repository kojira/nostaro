use anyhow::Result;
use chrono::{DateTime, Utc};
use nostr_sdk::prelude::*;

use crate::client;
use crate::config::NostaroConfig;
use crate::keys;
use crate::outln;
use crate::output;

pub async fn run(query: &str, limit: usize) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    println!("Searching for \"{}\"...\n", query);

    let events = client::search_notes(&nostr_client, query, limit).await?;

    // No match is still a result: --out gets an empty listing rather than no
    // file at all, so the body is emitted in every case.
    if events.is_empty() {
        println!("No notes found matching \"{}\".", query);
    }

    if output::is_json() {
        let notes: Result<Vec<serde_json::Value>> = events
            .iter()
            .map(|event| Ok(serde_json::to_value(event)?))
            .collect();
        output::write_json(&serde_json::json!({
            "count": events.len(),
            "events": notes?,
        }))?;
    } else {
        output::open_body()?;
        for event in &events {
            let npub = event.pubkey.to_bech32()?;
            let short_npub = &npub;
            let timestamp = event.created_at.as_u64() as i64;
            let datetime = DateTime::<Utc>::from_timestamp(timestamp, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "unknown".to_string());

            outln!("[{}] {}", short_npub, datetime)?;
            outln!("{}", event.content)?;
            let note_id = event.id.to_bech32()?;
            outln!("  id: {}", note_id)?;
            outln!("{}", "-".repeat(60))?;
        }
    }

    if !events.is_empty() {
        println!("\nFound {} note(s).", events.len());
    }

    nostr_client.disconnect().await;
    Ok(())
}
