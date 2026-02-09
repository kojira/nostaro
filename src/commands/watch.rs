use anyhow::Result;
use nostr_sdk::prelude::*;
use std::collections::HashMap;

use crate::client;
use crate::config::NostaroConfig;
use crate::keys;
use crate::utils::resolve_pubkey;

pub async fn run(webhook_url: &str, npub_str: Option<&str>, channel_id: Option<&str>) -> Result<()> {
    let config = NostaroConfig::load()?;
    let own_keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&own_keys, &config).await?;

    let own_pubkey = own_keys.public_key();

    // Channel watch mode
    let watching_channel = channel_id.map(|s| s.to_string());

    if let Some(ref ch_id) = watching_channel {
        println!("Watching NIP-28 channel: {}...", &ch_id[..16.min(ch_id.len())]);
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
        println!(
            "Watching for events targeting {}...",
            &target_npub[..20.min(target_npub.len())]
        );
        println!("Webhook: {}", webhook_url);
        println!("Press Ctrl+C to stop.\n");

        let filter = Filter::new()
            .pubkey(target_pubkey)
            .kinds(vec![Kind::TextNote, Kind::Reaction])
            .since(Timestamp::now());

        nostr_client.subscribe(filter, None).await?;
    }

    let mut profile_cache: HashMap<PublicKey, (String, Option<String>)> = HashMap::new();
    let http_client = reqwest::Client::new();

    let mut notifications = nostr_client.notifications();
    while let Ok(notification) = notifications.recv().await {
        if let RelayPoolNotification::Event { event, .. } = notification {
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
                            if let Some(TagStandard::Event { event_id, marker, .. }) = t.as_standardized() {
                                let marker_match = marker.as_ref().map_or(false, |m| *m == Marker::Root);
                                event_id.to_hex() == *ch_id && marker_match
                            } else {
                                false
                            }
                        });
                        if !belongs {
                            continue;
                        }
                        let npub_str = event.pubkey.to_bech32()?;
                        let msg = format!("**{}**\nnpub: {}\nnote: {}\n\n{}", sender_name, npub_str, note_id, event.content);
                        msg
                    } else {
                        continue;
                    }
                }
                Kind::TextNote => {
                    let has_e_tag = event.tags.iter().any(|t| {
                        matches!(t.as_standardized(), Some(TagStandard::Event { .. }))
                    });
                    let label = if has_e_tag { "リプライ" } else { "メンション" };
                    format!("📩 **{}** from {}\n> {}\n🔗 {}", label, sender_name, event.content, note_id)
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
                        original_note_str = orig_id.to_bech32().unwrap_or_else(|_| "unknown".to_string());

                        // Fetch the original post
                        let filter = Filter::new().id(orig_id).kind(Kind::TextNote).limit(1);
                        if let Ok(events) = nostr_client
                            .fetch_events(filter, std::time::Duration::from_secs(5))
                            .await
                        {
                            if let Some(orig_event) = events.first() {
                                let content: String = orig_event.content.chars().take(200).collect();
                                let ellipsis = if orig_event.content.chars().count() > 200 { "..." } else { "" };
                                original_content_line = format!("\n\n> {}{}", content, ellipsis);
                            }
                        }
                    }

                    format!(
                        "**{}** reacted {}\nnpub: {}{}\nnote: {}",
                        sender_name, emoji, npub_str, original_content_line, original_note_str
                    )
                }
                _ => continue,
            };

            println!("[{}] {}", chrono::Local::now().format("%H:%M:%S"), message);

            if let Err(e) = send_discord_webhook(&http_client, webhook_url, &message, &sender_name, sender_avatar.as_deref()).await {
                eprintln!("Webhook error: {}", e);
            }
        }
    }

    Ok(())
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
            let display = if let Some(ref dn) = metadata.display_name {
                if !dn.is_empty() { dn.clone() } else if let Some(ref name) = metadata.name { name.clone() } else { npub.clone() }
            } else if let Some(ref name) = metadata.name {
                if !name.is_empty() { name.clone() } else { npub.clone() }
            } else {
                npub.clone()
            };
            let picture = metadata.picture.map(|u| u.to_string()).filter(|s| !s.is_empty());
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
        body["avatar_url"] = serde_json::Value::String(url.to_string());
    }

    let resp = client.post(webhook_url).json(&body).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Discord webhook failed ({}): {}", status, body);
    }

    Ok(())
}
