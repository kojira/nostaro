use anyhow::Result;
use chrono::{DateTime, Utc};
use nostr_sdk::prelude::*;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::cache::CacheDb;
use crate::client;
use crate::config::NostaroConfig;
use crate::keys;
use crate::outln;
use crate::output;

/// Resolve who reacted: the npub, the cached display name (if any) and whether
/// it is the local user. Reactions by the local user are never name-resolved,
/// they are always shown as "you".
fn reactor_identity(
    reaction: &Event,
    own_pubkey: PublicKey,
    cache: Option<&CacheDb>,
) -> (String, Option<String>, bool) {
    let npub = reaction.pubkey.to_bech32().unwrap_or_default();
    if reaction.pubkey == own_pubkey {
        return (npub, None, true);
    }
    let name = cache
        .and_then(|cache| cache.get_profile(&reaction.pubkey.to_hex()).ok().flatten())
        .and_then(|profile| profile.display_name.or(profile.name))
        .filter(|name| !name.is_empty());
    (npub, name, false)
}

fn reaction_emoji(reaction: &Event) -> String {
    if reaction.content.is_empty() {
        "+".to_string()
    } else {
        reaction.content.clone()
    }
}

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

async fn fetch_and_cache_profiles(
    nostr_client: &Client,
    pubkeys: Vec<PublicKey>,
    cache: &CacheDb,
) -> Result<()> {
    let missing_pubkeys: Vec<PublicKey> = pubkeys
        .into_iter()
        .filter(|pk| cache.get_profile(&pk.to_hex()).ok().flatten().is_none())
        .collect();

    if missing_pubkeys.is_empty() {
        return Ok(());
    }

    let filter = Filter::new()
        .kind(Kind::Metadata)
        .authors(missing_pubkeys)
        .limit(500);

    let events = nostr_client
        .fetch_events(filter, Duration::from_secs(10))
        .await?;

    for event in events {
        if let Ok(metadata) = Metadata::from_json(&event.content) {
            let _ = cache.store_profile(
                &event.pubkey.to_hex(),
                metadata.name.as_deref(),
                metadata.display_name.as_deref(),
                metadata.about.as_deref(),
                metadata.picture.as_deref(),
            );
        }
    }

    Ok(())
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

    // Batch-fetch kind:0 profiles for reactors not yet in cache
    if with_reactions && !reactions_by_event.is_empty() {
        if let Ok(cache) = CacheDb::open() {
            let all_reactor_pubkeys: Vec<PublicKey> = reactions_by_event
                .values()
                .flatten()
                .map(|e| e.pubkey)
                .collect::<HashSet<PublicKey>>()
                .into_iter()
                .collect();
            let _ = fetch_and_cache_profiles(&nostr_client, all_reactor_pubkeys, &cache).await;
        }
    }

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

    // An empty timeline is still a result: --out gets an empty listing rather
    // than no file at all, so the body is emitted in every case.
    if all_events.is_empty() {
        println!("No notes found.");
    }

    let own_pubkey = keys.public_key();

    if output::is_json() {
        let mut notes = Vec::with_capacity(all_events.len());
        for event in &all_events {
            let reactions: Vec<serde_json::Value> = reactions_by_event
                .get(&event.id)
                .map(|reactions| {
                    reactions
                        .iter()
                        .map(|reaction| {
                            let (npub, name, is_self) =
                                reactor_identity(reaction, own_pubkey, cache.as_ref());
                            serde_json::json!({
                                "emoji": reaction_emoji(reaction),
                                "npub": npub,
                                "name": name,
                                "is_self": is_self,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();

            notes.push(serde_json::json!({
                "event": serde_json::to_value(event)?,
                "following": following_set.contains(&event.pubkey),
                "is_self": event.pubkey == own_pubkey,
                "reactions": reactions,
            }));
        }
        output::write_json(&serde_json::json!({
            "count": notes.len(),
            "notes": notes,
        }))?;

        if !all_events.is_empty() {
            println!("\nShowing {} note(s).", all_events.len());
        }
        nostr_client.disconnect().await;
        return Ok(());
    }

    output::open_body()?;
    for event in &all_events {
        let npub = event.pubkey.to_bech32()?;
        let short_npub = &npub;
        let is_following = following_set.contains(&event.pubkey);
        let is_self = event.pubkey == own_pubkey;

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
        outln!("[{}]{} {}", short_npub, label, datetime)?;
        outln!("{}", event.content)?;
        outln!("  id: {}", note_id)?;

        if with_reactions {
            if let Some(reactions) = reactions_by_event.get(&event.id) {
                let mut counts: HashMap<String, (usize, Vec<String>)> = HashMap::new();

                for reaction in reactions {
                    let (reactor_npub, name, is_self) =
                        reactor_identity(reaction, own_pubkey, cache.as_ref());
                    let reactor_label = if is_self {
                        format!("you({})", reactor_npub)
                    } else {
                        match name {
                            Some(name) => format!("{}({})", name, reactor_npub),
                            None => reactor_npub,
                        }
                    };
                    let entry = counts
                        .entry(reaction_emoji(reaction))
                        .or_insert_with(|| (0, Vec::new()));
                    entry.0 += 1;
                    entry.1.push(reactor_label);
                }

                if !counts.is_empty() {
                    let mut parts = Vec::new();

                    for (emoji, (count, names)) in &counts {
                        parts.push(format!("{} x{} ({})", emoji, count, names.join(", ")));
                    }

                    outln!("  Reactions: {}", parts.join(", "))?;
                }
            }
        }

        outln!("{}", "-".repeat(60))?;
    }

    if !all_events.is_empty() {
        println!("\nShowing {} note(s).", all_events.len());
    }

    nostr_client.disconnect().await;
    Ok(())
}
