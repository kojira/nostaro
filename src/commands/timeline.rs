use anyhow::Result;
use chrono::{DateTime, Utc};
use nostr_sdk::prelude::*;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::cache::CacheDb;
use crate::client;
use crate::config::NostaroConfig;
use crate::keys;

async fn fetch_reactions(
    nostr_client: &Client,
    event_ids: Vec<EventId>,
) -> Result<HashMap<EventId, Vec<Event>>> {
    let filter = Filter::new()
        .kind(Kind::Reaction)
        .events(event_ids)
        .limit(1000);
    let reaction_events = nostr_client
        .fetch_events(filter, Duration::from_secs(10))
        .await?;

    let mut reactions_by_event: HashMap<EventId, Vec<Event>> = HashMap::new();

    for reaction in reaction_events {
        let related_event_ids: Vec<EventId> = reaction
            .tags
            .iter()
            .filter_map(|tag: &Tag| match tag.as_standardized() {
                Some(TagStandard::Event { event_id, .. }) => Some(*event_id),
                _ => None,
            })
            .collect();

        for event_id in related_event_ids {
            reactions_by_event
                .entry(event_id)
                .or_default()
                .push(reaction.clone());
        }
    }

    Ok(reactions_by_event)
}

pub async fn run(limit: usize, with_reactions: bool) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    println!("Fetching timeline...\n");

    let contacts = client::fetch_contacts(&nostr_client, &keys.public_key()).await?;
    let following_set: HashSet<PublicKey> = contacts.iter().copied().collect();

    let mut authors = contacts.clone();
    authors.push(keys.public_key());

    let mut all_events = Vec::new();

    if !authors.is_empty() {
        let followed_events =
            client::fetch_timeline_for_authors(&nostr_client, &authors, limit).await?;
        all_events.extend(followed_events);
    }

    if all_events.len() < limit {
        let global_events = client::fetch_timeline(&nostr_client, limit).await?;
        let seen: HashSet<EventId> = all_events.iter().map(|e| e.id).collect();
        for event in global_events {
            if !seen.contains(&event.id) {
                all_events.push(event);
            }
        }
    }

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

    let reactions_by_event = if with_reactions {
        let event_ids: Vec<EventId> = all_events.iter().map(|e| e.id).collect();
        fetch_reactions(&nostr_client, event_ids).await?
    } else {
        HashMap::new()
    };
    let cache = CacheDb::open().ok();

    // Cache events
    if let Ok(cache) = CacheDb::open() {
        for event in &all_events {
            let tags_json = serde_json::to_string(&event.tags).unwrap_or_default();
            let _ = cache.store_event(
                &event.id.to_hex(),
                &event.pubkey.to_hex(),
                event.kind.as_u16(),
                &event.content,
                event.created_at.as_u64() as i64,
                &tags_json,
                &event.as_json(),
            );
        }
    }

    if all_events.is_empty() {
        println!("No notes found.");
        nostr_client.disconnect().await;
        return Ok(());
    }

    for event in &all_events {
        let npub = event.pubkey.to_bech32()?;
        let short_npub = &npub;
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

        let note_id = event.id.to_bech32()?;
        println!("[{}]{} {}", short_npub, label, datetime);
        println!("{}", event.content);
        println!("  id: {}", note_id);

        if with_reactions {
            if let Some(reactions) = reactions_by_event.get(&event.id) {
                let own_pubkey = keys.public_key();
                let mut counts: HashMap<String, (usize, Vec<String>)> = HashMap::new();

                for reaction in reactions {
                    let emoji = if reaction.content.is_empty() {
                        "+".to_string()
                    } else {
                        reaction.content.clone()
                    };
                    let reactor_npub = reaction.pubkey.to_bech32().unwrap_or_default();
                    let reactor_name = if reaction.pubkey == own_pubkey {
                        format!("you({})", reactor_npub)
                    } else {
                        let display_name = cache
                            .as_ref()
                            .and_then(|cache| cache.get_profile(&reaction.pubkey.to_hex()).ok().flatten())
                            .and_then(|profile| profile.display_name.or(profile.name))
                            .filter(|name| !name.is_empty());
                        match display_name {
                            Some(name) => format!("{}({})", name, reactor_npub),
                            None => reactor_npub,
                        }
                    };
                    let entry = counts.entry(emoji).or_insert_with(|| (0, Vec::new()));
                    entry.0 += 1;
                    entry.1.push(reactor_name);
                }

                if !counts.is_empty() {
                    let mut parts = Vec::new();

                    for (emoji, (count, names)) in &counts {
                        parts.push(format!("{} x{} ({})", emoji, count, names.join(", ")));
                    }

                    println!("  Reactions: {}", parts.join(", "));
                }
            }
        }

        println!("{}", "-".repeat(60));
    }

    println!("\nShowing {} note(s).", all_events.len());

    nostr_client.disconnect().await;
    Ok(())
}
