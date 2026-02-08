use anyhow::Result;
use nostr_sdk::prelude::*;
use std::collections::HashMap;

use crate::client;
use crate::config::NostaroConfig;
use crate::keys;

pub async fn run(webhook_url: &str, npub_str: Option<&str>) -> Result<()> {
    let config = NostaroConfig::load()?;
    let own_keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&own_keys, &config).await?;

    let target_pubkey = match npub_str {
        Some(pk) => PublicKey::parse(pk)?,
        None => own_keys.public_key(),
    };

    let own_pubkey = own_keys.public_key();
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

    let mut name_cache: HashMap<PublicKey, String> = HashMap::new();
    let http_client = reqwest::Client::new();

    let mut notifications = nostr_client.notifications();
    while let Ok(notification) = notifications.recv().await {
        if let RelayPoolNotification::Event { event, .. } = notification {
            if event.pubkey == own_pubkey {
                continue;
            }

            let sender_name =
                get_display_name(&nostr_client, &event.pubkey, &mut name_cache).await;

            let note_id = event.id.to_bech32()?;

            let message = match event.kind {
                Kind::TextNote => {
                    let has_e_tag = event.tags.iter().any(|t| {
                        matches!(t.as_standardized(), Some(TagStandard::Event { .. }))
                    });
                    let label = if has_e_tag { "リプライ" } else { "メンション" };
                    let content = truncate(&event.content, 200);
                    format!("📩 **{}** from {}\n> {}\n🔗 {}", label, sender_name, content, note_id)
                }
                Kind::Reaction => {
                    let emoji = if event.content.is_empty() {
                        "👍"
                    } else {
                        &event.content
                    };
                    let target_note = event
                        .tags
                        .iter()
                        .find_map(|t| {
                            if let Some(TagStandard::Event { event_id, .. }) = t.as_standardized()
                            {
                                event_id.to_bech32().ok()
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| "unknown".to_string());
                    format!(
                        "⚡ **リアクション** from {}\nEmoji: {} → {}\n🔗 {}",
                        sender_name, emoji, target_note, note_id
                    )
                }
                _ => continue,
            };

            println!("[{}] {}", chrono::Local::now().format("%H:%M:%S"), message);

            if let Err(e) = send_discord_webhook(&http_client, webhook_url, &message).await {
                eprintln!("Webhook error: {}", e);
            }
        }
    }

    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{}...", truncated)
    }
}

async fn get_display_name(
    nostr_client: &Client,
    pubkey: &PublicKey,
    cache: &mut HashMap<PublicKey, String>,
) -> String {
    if let Some(name) = cache.get(pubkey) {
        return name.clone();
    }

    let npub = pubkey.to_bech32().unwrap_or_else(|_| pubkey.to_hex());
    let short_npub = format!("{}...", &npub[..12.min(npub.len())]);

    let display = match client::fetch_profile(nostr_client, pubkey).await {
        Ok(Some(metadata)) => {
            if let Some(ref dn) = metadata.display_name {
                format!("{}({})", dn, short_npub)
            } else if let Some(ref name) = metadata.name {
                format!("{}({})", name, short_npub)
            } else {
                short_npub
            }
        }
        _ => short_npub,
    };

    cache.insert(*pubkey, display.clone());
    display
}

async fn send_discord_webhook(
    client: &reqwest::Client,
    webhook_url: &str,
    content: &str,
) -> Result<()> {
    let body = serde_json::json!({
        "content": content,
    });

    let resp = client.post(webhook_url).json(&body).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Discord webhook failed ({}): {}", status, body);
    }

    Ok(())
}
