use anyhow::{bail, Result};
use nostr_sdk::prelude::*;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use crate::client;
use crate::config::NostaroConfig;
use crate::keys;
use crate::utils::resolve_pubkey;

/// How stale an event can be (relative to now) before watch drops it as a replay.
const MAX_EVENT_AGE_SECS: u64 = 300;
/// Cap on remembered event IDs before the oldest are evicted, to bound memory growth.
const MAX_SEEN_EVENTS: usize = 1000;
/// Timeout for the best-effort author-name lookup in JSON mode. Kept short (unlike the
/// 10s default used elsewhere) so one slow/unresponsive relay can't stall the whole
/// real-time event stream; author_name is optional in the output schema.
const AUTHOR_NAME_FETCH_TIMEOUT: Duration = Duration::from_secs(3);

/// Tracks recently-seen event IDs to drop duplicates/replays, and rejects events older
/// than `MAX_EVENT_AGE_SECS` (relays sometimes replay old events on resubscribe).
/// Shared by both the Discord-webhook loop and the `--json` loop so a fix to this logic
/// only needs to be made once.
struct EventDeduplicator {
    seen: HashSet<EventId>,
    order: VecDeque<EventId>,
}

impl EventDeduplicator {
    fn new() -> Self {
        Self {
            seen: HashSet::new(),
            order: VecDeque::new(),
        }
    }

    /// Returns true if `event` is fresh and unseen and should be processed.
    fn accept(&mut self, event: &Event) -> bool {
        let now = chrono::Utc::now().timestamp() as u64;
        let created_at = event.created_at.as_u64();
        if now > created_at && now - created_at > MAX_EVENT_AGE_SECS {
            eprintln!(
                "Skipping old event: {} (created_at: {})",
                event.id, created_at
            );
            return false;
        }

        if self.seen.contains(&event.id) {
            return false;
        }
        if self.seen.len() >= MAX_SEEN_EVENTS {
            if let Some(oldest) = self.order.pop_front() {
                self.seen.remove(&oldest);
            }
        }
        self.seen.insert(event.id);
        self.order.push_back(event.id);
        true
    }
}

/// Resolve a display name from kind:0 metadata: prefer display_name, fall back to name,
/// treating empty strings as absent. Shared by the Discord and JSON name-lookup paths.
fn resolve_display_name(metadata: &Metadata) -> Option<String> {
    metadata
        .display_name
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| metadata.name.clone().filter(|s| !s.is_empty()))
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    webhook_url: Option<&str>,
    npub_str: Option<&str>,
    channel_id: Option<&str>,
    keywords: &[String],
    extra_kinds: &[u16],
    mention_only: bool,
    authors: &[String],
    relays: &[String],
    json_output: bool,
) -> Result<()> {
    if !json_output && webhook_url.is_none() {
        bail!("--webhook is required unless --json is specified");
    }
    if json_output && channel_id.is_some() {
        bail!("--channel is not supported together with --json");
    }
    if json_output && npub_str.is_some() {
        bail!("--npub is not supported together with --json");
    }

    let config = NostaroConfig::load()?;
    let own_keys = keys::keys_from_config(&config)?;

    let author_pubkeys: Vec<PublicKey> = authors
        .iter()
        .map(|a| resolve_pubkey(a))
        .collect::<Result<Vec<_>>>()?;

    let nostr_client = if relays.is_empty() {
        client::create_client(&own_keys, &config).await?
    } else {
        client::create_client_with_relay_list(&own_keys, relays).await?
    };

    let own_pubkey = own_keys.public_key();

    if json_output {
        return watch_json(&nostr_client, keywords, extra_kinds, &author_pubkeys).await;
    }
    let webhook_url = webhook_url.expect("checked above");

    // Channel watch mode
    let watching_channel = channel_id.map(|s| s.to_string());

    if let Some(ref ch_id) = watching_channel {
        println!(
            "Watching NIP-28 channel: {}...",
            &ch_id[..16.min(ch_id.len())]
        );
        println!("Webhook: {}", webhook_url);
        println!("Press Ctrl+C to stop.\n");

        let channel_event_id = EventId::from_hex(ch_id)?;
        let filter = Filter::new()
            .kind(Kind::ChannelMessage)
            .event(channel_event_id)
            .since(Timestamp::now());

        nostr_client.subscribe(filter, None).await?;
    }

    // Mention/reply/reaction watch mode (skip if only channel is specified)
    let watching_mentions = channel_id.is_none() || npub_str.is_some();
    if watching_mentions {
        let target_pubkey = match npub_str {
            Some(pk) => resolve_pubkey(pk)?,
            None => own_keys.public_key(),
        };

        let target_npub = target_pubkey.to_bech32()?;
        println!("Watching for events targeting {}...", &target_npub);
        println!("Webhook: {}", webhook_url);
        println!("Press Ctrl+C to stop.\n");

        if extra_kinds.is_empty() {
            // Default behavior: kind:1 + kind:7 with p-tag filter
            let filter = Filter::new()
                .pubkey(target_pubkey)
                .kinds(vec![Kind::TextNote, Kind::Reaction])
                .since(Timestamp::now());
            nostr_client.subscribe(filter, None).await?;
        } else {
            // Custom kinds subscription
            let kinds_vec: Vec<Kind> = extra_kinds.iter().map(|&k| Kind::from(k)).collect();
            let mut filter = Filter::new().kinds(kinds_vec).since(Timestamp::now());
            if mention_only {
                filter = filter.pubkey(target_pubkey);
            }
            nostr_client.subscribe(filter, None).await?;
            println!(
                "Custom kinds: {:?}, mention_only: {}",
                extra_kinds, mention_only
            );
        }
    }

    // Keyword watch mode (local matching on existing relays)
    if !keywords.is_empty() {
        // Subscribe to all kind:1 events since now; keyword matching is done locally
        let filter = Filter::new().kind(Kind::TextNote).since(Timestamp::now());
        nostr_client.subscribe(filter, None).await?;
        for keyword in keywords {
            println!("Watching keyword: {}", keyword);
        }
    }

    let mut profile_cache: HashMap<PublicKey, (String, Option<String>)> = HashMap::new();
    let http_client = reqwest::Client::new();
    let mut dedup = EventDeduplicator::new();

    let mut notifications = nostr_client.notifications();
    while let Ok(notification) = notifications.recv().await {
        if let RelayPoolNotification::Event { event, .. } = notification {
            if !dedup.accept(&event) {
                continue;
            }

            if !author_pubkeys.is_empty() && !author_pubkeys.contains(&event.pubkey) {
                continue;
            }

            if event.pubkey == own_pubkey && event.kind != Kind::ChannelMessage {
                continue;
            }

            let (sender_name, sender_avatar) =
                get_profile_info(&nostr_client, &event.pubkey, &mut profile_cache).await;

            let note_id = event.id.to_bech32()?;

            let message = match event.kind {
                Kind::ChannelMessage => {
                    if let Some(ref ch_id) = watching_channel {
                        // Check if this message belongs to the watched channel
                        let belongs = event.tags.iter().any(|t| {
                            if let Some(TagStandard::Event {
                                event_id, marker, ..
                            }) = t.as_standardized()
                            {
                                let marker_match =
                                    marker.as_ref().is_some_and(|m| *m == Marker::Root);
                                event_id.to_hex() == *ch_id && marker_match
                            } else {
                                false
                            }
                        });
                        if !belongs {
                            continue;
                        }
                        let npub_str = event.pubkey.to_bech32()?;
                        let msg = format!(
                            "**{}**\nnpub: {}\nnote: {}\n\n{}",
                            sender_name, npub_str, note_id, event.content
                        );
                        msg
                    } else {
                        continue;
                    }
                }
                Kind::TextNote => {
                    let is_mention_or_reply = watching_mentions && (
                        !mention_only || event.tags.iter().any(|t| {
                            matches!(t.as_standardized(), Some(TagStandard::PublicKey { public_key, .. }) if *public_key == own_pubkey)
                        })
                    );

                    if is_mention_or_reply {
                        let has_e_tag = event.tags.iter().any(|t| {
                            matches!(t.as_standardized(), Some(TagStandard::Event { .. }))
                        });
                        let label = if has_e_tag {
                            "リプライ"
                        } else {
                            "メンション"
                        };
                        format!(
                            "📩 **{}** from {}\n> {}\n🔗 {}",
                            label, sender_name, event.content, note_id
                        )
                    } else if !keywords.is_empty() {
                        let matched_keyword = keywords
                            .iter()
                            .find(|kw| event.content.to_lowercase().contains(&kw.to_lowercase()));
                        if let Some(kw) = matched_keyword {
                            format!(
                                "🔍 **keyword match: {}**\n{}\n> {}\nnote: {}",
                                kw, sender_name, event.content, note_id
                            )
                        } else {
                            continue;
                        }
                    } else {
                        continue;
                    }
                }
                Kind::Reaction => {
                    let emoji = if event.content.is_empty() {
                        "👍"
                    } else {
                        &event.content
                    };
                    let npub_str = event.pubkey.to_bech32()?;

                    // Get the original event ID from e tag
                    let original_event_id = event.tags.iter().find_map(|t| {
                        if let Some(TagStandard::Event { event_id, .. }) = t.as_standardized() {
                            Some(*event_id)
                        } else {
                            None
                        }
                    });

                    let mut original_content_line = String::new();
                    let mut original_note_str = "unknown".to_string();

                    if let Some(orig_id) = original_event_id {
                        original_note_str = orig_id
                            .to_bech32()
                            .unwrap_or_else(|_| "unknown".to_string());

                        // Fetch the original post
                        let filter = Filter::new().id(orig_id).kind(Kind::TextNote).limit(1);
                        if let Ok(events) = nostr_client
                            .fetch_events(filter, std::time::Duration::from_secs(5))
                            .await
                        {
                            if let Some(orig_event) = events.first() {
                                let content: String =
                                    orig_event.content.chars().take(200).collect();
                                let ellipsis = if orig_event.content.chars().count() > 200 {
                                    "..."
                                } else {
                                    ""
                                };
                                original_content_line = format!("\n\n> {}{}", content, ellipsis);
                            }
                        }
                    }

                    format!(
                        "**{}** reacted {}\nnpub: {}{}\nnote: {}\nreaction_note: {}",
                        sender_name,
                        emoji,
                        npub_str,
                        original_content_line,
                        original_note_str,
                        note_id
                    )
                }
                k if k == Kind::from(9735u16) => {
                    // Zap Receipt (NIP-57)
                    let npub_str_val = event
                        .pubkey
                        .to_bech32()
                        .unwrap_or_else(|_| event.pubkey.to_hex());

                    // Parse description tag for zapper info
                    let description_json = event.tags.iter().find_map(|t| {
                        if t.kind() == TagKind::custom("description") {
                            t.content().map(|s| s.to_string())
                        } else {
                            None
                        }
                    });

                    let (zap_message, zapper_npub) = if let Some(desc_json) = description_json {
                        if let Ok(zap_request) =
                            serde_json::from_str::<serde_json::Value>(&desc_json)
                        {
                            let content = zap_request["content"].as_str().unwrap_or("").to_string();
                            let zapper_npub = if let Some(pk_hex) = zap_request["pubkey"].as_str() {
                                PublicKey::from_hex(pk_hex)
                                    .ok()
                                    .and_then(|pk| pk.to_bech32().ok())
                                    .unwrap_or_else(|| pk_hex.to_string())
                            } else {
                                npub_str_val.clone()
                            };
                            (content, zapper_npub)
                        } else {
                            (String::new(), npub_str_val.clone())
                        }
                    } else {
                        (String::new(), npub_str_val.clone())
                    };

                    let has_bolt11 = event
                        .tags
                        .iter()
                        .any(|t| t.kind() == TagKind::custom("bolt11"));

                    if has_bolt11 {
                        if zap_message.is_empty() {
                            format!("⚡ Zap受信！\nfrom: {}\nnote: {}", zapper_npub, note_id)
                        } else {
                            format!(
                                "⚡ Zap受信！\nfrom: {}\nメッセージ: {}\nnote: {}",
                                zapper_npub, zap_message, note_id
                            )
                        }
                    } else {
                        continue;
                    }
                }
                k if extra_kinds.contains(&k.as_u16()) => {
                    // Generic custom kind notification
                    let npub_str_val = event
                        .pubkey
                        .to_bech32()
                        .unwrap_or_else(|_| event.pubkey.to_hex());
                    let content_preview: String = event.content.chars().take(500).collect();
                    let ellipsis = if event.content.chars().count() > 500 {
                        "..."
                    } else {
                        ""
                    };
                    format!(
                        "📡 kind:{} from {}\n> {}{}\nnote: {}",
                        k.as_u16(),
                        npub_str_val,
                        content_preview,
                        ellipsis,
                        note_id
                    )
                }
                _ => continue,
            };

            println!("[{}] {}", chrono::Local::now().format("%H:%M:%S"), message);

            if let Err(e) = send_discord_webhook(
                &http_client,
                webhook_url,
                &message,
                &sender_name,
                sender_avatar.as_deref(),
            )
            .await
            {
                eprintln!("Webhook error: {}", e);
            }
        }
    }

    Ok(())
}

/// Matched event schema emitted on stdout by `watch --json`, one line per event (JSONL).
#[derive(Serialize)]
struct JsonEvent {
    id: String,
    pubkey: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    npub: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    author_name: Option<String>,
    created_at: u64,
    kind: u16,
    content: String,
    tags: Tags,
}

/// Generic, output-agnostic watch loop for consumers like OpenCrab: subscribes to the
/// given kinds (defaulting to kind:1) filtered by author/keyword, and prints one JSON
/// object per matched event to stdout (JSONL). All progress/diagnostic output goes to
/// stderr so stdout stays pure JSONL.
async fn watch_json(
    nostr_client: &Client,
    keywords: &[String],
    extra_kinds: &[u16],
    author_pubkeys: &[PublicKey],
) -> Result<()> {
    let kinds_to_use: Vec<Kind> = if extra_kinds.is_empty() {
        vec![Kind::TextNote]
    } else {
        extra_kinds.iter().map(|&k| Kind::from(k)).collect()
    };

    eprintln!("Watching (JSON mode), kinds: {:?}", extra_kinds);
    if !author_pubkeys.is_empty() {
        eprintln!("Authors: {}", author_pubkeys.len());
    }
    if !keywords.is_empty() {
        eprintln!("Keywords: {:?}", keywords);
    }
    eprintln!("Press Ctrl+C to stop.\n");

    let mut filter = Filter::new().kinds(kinds_to_use).since(Timestamp::now());
    if !author_pubkeys.is_empty() {
        filter = filter.authors(author_pubkeys.to_vec());
    }
    nostr_client.subscribe(filter, None).await?;

    let mut dedup = EventDeduplicator::new();
    let mut author_name_cache: HashMap<PublicKey, Option<String>> = HashMap::new();

    let mut notifications = nostr_client.notifications();
    while let Ok(notification) = notifications.recv().await {
        if let RelayPoolNotification::Event { event, .. } = notification {
            if !dedup.accept(&event) {
                continue;
            }

            if !author_pubkeys.is_empty() && !author_pubkeys.contains(&event.pubkey) {
                continue;
            }

            if !keywords.is_empty() {
                let matched = keywords
                    .iter()
                    .any(|kw| event.content.to_lowercase().contains(&kw.to_lowercase()));
                if !matched {
                    continue;
                }
            }

            match build_json_event(nostr_client, &event, &mut author_name_cache).await {
                Ok(line) => println!("{}", line),
                Err(e) => eprintln!("Failed to serialize event {}: {}", event.id, e),
            }
        }
    }

    Ok(())
}

async fn build_json_event(
    nostr_client: &Client,
    event: &Event,
    author_name_cache: &mut HashMap<PublicKey, Option<String>>,
) -> Result<String> {
    let author_name = get_author_name(nostr_client, &event.pubkey, author_name_cache).await;

    let json_event = JsonEvent {
        id: event.id.to_hex(),
        pubkey: event.pubkey.to_hex(),
        npub: event.pubkey.to_bech32().ok(),
        note_id: event.id.to_bech32().ok(),
        author_name,
        created_at: event.created_at.as_u64(),
        kind: event.kind.as_u16(),
        content: event.content.clone(),
        tags: event.tags.clone(),
    };

    Ok(serde_json::to_string(&json_event)?)
}

async fn get_author_name(
    nostr_client: &Client,
    pubkey: &PublicKey,
    cache: &mut HashMap<PublicKey, Option<String>>,
) -> Option<String> {
    if let Some(name) = cache.get(pubkey) {
        return name.clone();
    }

    let name =
        match client::fetch_profile_with_timeout(nostr_client, pubkey, AUTHOR_NAME_FETCH_TIMEOUT)
            .await
        {
            Ok(Some(metadata)) => resolve_display_name(&metadata),
            _ => None,
        };

    cache.insert(*pubkey, name.clone());
    name
}

async fn get_profile_info(
    nostr_client: &Client,
    pubkey: &PublicKey,
    cache: &mut HashMap<PublicKey, (String, Option<String>)>,
) -> (String, Option<String>) {
    if let Some(info) = cache.get(pubkey) {
        return info.clone();
    }

    let npub = pubkey.to_bech32().unwrap_or_else(|_| pubkey.to_hex());

    let info = match client::fetch_profile(nostr_client, pubkey).await {
        Ok(Some(metadata)) => {
            let display = resolve_display_name(&metadata).unwrap_or_else(|| npub.clone());
            let picture = metadata
                .picture
                .map(|u| u.to_string())
                .filter(|s| !s.is_empty());
            (display, picture)
        }
        _ => (npub, None),
    };

    cache.insert(*pubkey, info.clone());
    info
}

async fn send_discord_webhook(
    client: &reqwest::Client,
    webhook_url: &str,
    content: &str,
    username: &str,
    avatar_url: Option<&str>,
) -> Result<()> {
    let content = if content.chars().count() > 2000 {
        format!("{}...", content.chars().take(1997).collect::<String>())
    } else {
        content.to_string()
    };

    let mut body = serde_json::json!({
        "content": content,
        "username": username,
    });

    if let Some(url) = avatar_url {
        if url.starts_with("http://") || url.starts_with("https://") {
            body["avatar_url"] = serde_json::Value::String(url.to_string());
        }
    }

    let resp = client.post(webhook_url).json(&body).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Discord webhook failed ({}): {}", status, body);
    }

    Ok(())
}
