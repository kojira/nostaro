use anyhow::Result;
use chrono::{DateTime, Utc};
use nostr_sdk::prelude::*;
use std::collections::HashSet;

use crate::client;
use crate::config::NostaroConfig;
use crate::keys;

pub async fn run(limit: usize) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    println!("Fetching timeline...\n");

    // Get follow list for prioritization
    let contacts = client::fetch_contacts(&nostr_client, &keys.public_key()).await?;
    let following_set: HashSet<PublicKey> = contacts.iter().copied().collect();

    // Include self in the "followed" set
    let mut authors = contacts.clone();
    authors.push(keys.public_key());

    let mut all_events = Vec::new();

    // Fetch notes from followed users
    if !authors.is_empty() {
        let followed_events =
            client::fetch_timeline_for_authors(&nostr_client, &authors, limit).await?;
        all_events.extend(followed_events);
    }

    // If we don't have enough from followed users, fetch global
    if all_events.len() < limit {
        let global_events = client::fetch_timeline(&nostr_client, limit).await?;
        let seen: HashSet<EventId> = all_events.iter().map(|e| e.id).collect();
        for event in global_events {
            if !seen.contains(&event.id) {
                all_events.push(event);
            }
        }
    }

    // Sort: followed users first, then by timestamp
    all_events.sort_by(|a, b| {
        let a_following = following_set.contains(&a.pubkey) || a.pubkey == keys.public_key();
        let b_following = following_set.contains(&b.pubkey) || b.pubkey == keys.public_key();
        match (a_following, b_following) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => b.created_at.cmp(&a.created_at),
        }
    });

    all_events.truncate(limit);

    if all_events.is_empty() {
        println!("No notes found.");
        return Ok(());
    }

    for event in &all_events {
        let npub = event.pubkey.to_bech32()?;
        let short_npub = &npub[..12.min(npub.len())];
        let is_following = following_set.contains(&event.pubkey);
        let is_self = event.pubkey == keys.public_key();

        let label = if is_self {
            " [you]"
        } else if is_following {
            " [following]"
        } else {
            ""
        };

        let timestamp = event.created_at.as_u64() as i64;
        let datetime = DateTime::<Utc>::from_timestamp(timestamp, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "unknown".to_string());

        println!("[{}]{} {}", short_npub, label, datetime);
        println!("{}", event.content);
        println!("{}", "-".repeat(60));
    }

    println!("\nShowing {} note(s).", all_events.len());

    Ok(())
}
