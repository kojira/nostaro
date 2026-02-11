use anyhow::Result;
use chrono::{DateTime, Utc};
use nostr_sdk::prelude::*;

use crate::client;
use crate::config::NostaroConfig;
use crate::keys;

pub async fn create(name: &str, about: Option<&str>, picture: Option<&str>) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let mut meta = serde_json::json!({ "name": name });
    if let Some(a) = about {
        meta["about"] = serde_json::json!(a);
    }
    if let Some(p) = picture {
        meta["picture"] = serde_json::json!(p);
    }
    let content = serde_json::to_string(&meta)?;

    println!("Creating channel...");
    let event_id = client::create_channel(&nostr_client, &content).await?;
    println!("Channel created! ID: {}", event_id.to_hex());

    Ok(())
}

pub async fn edit(
    channel_id_str: &str,
    name: &str,
    about: Option<&str>,
    picture: Option<&str>,
) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let channel_id = EventId::parse(channel_id_str)?;

    let mut meta = serde_json::json!({ "name": name });
    if let Some(a) = about {
        meta["about"] = serde_json::json!(a);
    }
    if let Some(p) = picture {
        meta["picture"] = serde_json::json!(p);
    }
    let content = serde_json::to_string(&meta)?;

    let relay_url = config
        .active_relays()
        .first()
        .cloned()
        .unwrap_or_default();

    println!("Updating channel metadata...");
    client::edit_channel(&nostr_client, &channel_id, &content, &relay_url).await?;
    println!("Channel metadata updated!");

    Ok(())
}

pub async fn list() -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    println!("Fetching channels...\n");
    let channels = client::fetch_channels(&nostr_client, 20).await?;

    if channels.is_empty() {
        println!("No channels found.");
        return Ok(());
    }

    for ch in &channels {
        let id_hex = ch.id.to_hex();
        let short_id = &id_hex;

        if let Ok(meta) = serde_json::from_str::<serde_json::Value>(&ch.content) {
            let name = meta
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("Unnamed");
            let about = meta.get("about").and_then(|v| v.as_str()).unwrap_or("");
            println!("[{}] {}", short_id, name);
            if !about.is_empty() {
                println!("  {}", about);
            }
        } else {
            println!("[{}] {}", short_id, ch.content);
        }
    }

    println!("\n{} channel(s) found.", channels.len());
    Ok(())
}

pub async fn read(channel_id_str: &str) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let channel_id = EventId::parse(channel_id_str)?;

    println!("Fetching channel messages...\n");
    let messages = client::fetch_channel_messages(&nostr_client, &channel_id, 30).await?;

    if messages.is_empty() {
        println!("No messages in this channel.");
        return Ok(());
    }

    for msg in &messages {
        let npub = msg.pubkey.to_bech32()?;
        let short_npub = &npub;
        let timestamp = msg.created_at.as_u64() as i64;
        let datetime = DateTime::<Utc>::from_timestamp(timestamp, 0)
            .map(|dt| dt.format("%H:%M:%S").to_string())
            .unwrap_or_else(|| "??:??:??".to_string());

        println!("[{}] {}: {}", datetime, short_npub, msg.content);
    }

    println!("\n{} message(s).", messages.len());
    Ok(())
}

pub async fn post(channel_id_str: &str, message: &str) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let channel_id = EventId::parse(channel_id_str)?;

    println!("Posting to channel...");
    client::post_channel_message(&nostr_client, &channel_id, message).await?;
    println!("Message posted successfully!");

    Ok(())
}
