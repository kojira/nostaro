use anyhow::Result;
use chrono::{DateTime, Utc};
use nostr_sdk::prelude::*;

use crate::client;
use crate::config::NostaroConfig;
use crate::keys;

pub async fn send(npub_str: &str, message: &str) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let receiver = PublicKey::parse(npub_str)?;

    println!("Sending DM...");
    client::send_dm(&nostr_client, receiver, message).await?;

    let npub = receiver.to_bech32()?;
    println!("DM sent to {}!", &npub[..12.min(npub.len())]);

    Ok(())
}

pub async fn read(npub_filter: Option<&str>) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let filter_pubkey = match npub_filter {
        Some(pk) => Some(PublicKey::parse(pk)?),
        None => None,
    };

    println!("Fetching DMs...\n");

    // Fetch both NIP-17 gift wraps and NIP-04 encrypted DMs
    let gift_wraps = client::fetch_gift_wraps(&nostr_client, &keys.public_key(), 20).await?;
    let nip04_dms = client::fetch_nip04_dms(&nostr_client, &keys.public_key(), 20).await?;

    if gift_wraps.is_empty() && nip04_dms.is_empty() {
        println!("No direct messages found.");
        return Ok(());
    }

    let mut count = 0;

    // Process NIP-17 gift-wrapped DMs
    for gw in &gift_wraps {
        match nostr_client.unwrap_gift_wrap(gw).await {
            Ok(unwrapped) => {
                if let Some(ref filter_pk) = filter_pubkey {
                    if &unwrapped.sender != filter_pk {
                        continue;
                    }
                }

                let sender_npub = unwrapped.sender.to_bech32()?;
                let short_sender = &sender_npub[..12.min(sender_npub.len())];
                let timestamp = unwrapped.rumor.created_at.as_u64() as i64;
                let datetime = DateTime::<Utc>::from_timestamp(timestamp, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                println!("[{}] {} [NIP-17]", short_sender, datetime);
                println!("{}", unwrapped.rumor.content);
                println!("{}", "-".repeat(60));
                count += 1;
            }
            Err(_) => continue,
        }
    }

    // Process NIP-04 encrypted DMs
    for dm in &nip04_dms {
        // Determine the other party (sender if we received, recipient if we sent)
        let other_party = if dm.pubkey == keys.public_key() {
            // We sent this message, get recipient from 'p' tag
            dm.tags.iter()
                .find_map(|tag| {
                    if let Some(TagStandard::PublicKey { public_key, .. }) = tag.as_standardized() {
                        Some(*public_key)
                    } else {
                        None
                    }
                })
        } else {
            // We received this message, sender is the author
            Some(dm.pubkey)
        };

        if let Some(other_pk) = other_party {
            // Apply filter if specified
            if let Some(ref filter_pk) = filter_pubkey {
                if &other_pk != filter_pk {
                    continue;
                }
            }

            // Decrypt NIP-04 message
            match nostr_sdk::nips::nip04::decrypt(keys.secret_key(), &other_pk, &dm.content) {
                Ok(decrypted_content) => {
                    let other_npub = other_pk.to_bech32()?;
                    let short_other = &other_npub[..12.min(other_npub.len())];
                    let timestamp = dm.created_at.as_u64() as i64;
                    let datetime = DateTime::<Utc>::from_timestamp(timestamp, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                        .unwrap_or_else(|| "unknown".to_string());

                    let direction = if dm.pubkey == keys.public_key() {
                        format!("To {}", short_other)
                    } else {
                        format!("From {}", short_other)
                    };

                    println!("[{}] {} [NIP-04]", direction, datetime);
                    println!("{}", decrypted_content);
                    println!("{}", "-".repeat(60));
                    count += 1;
                }
                Err(_) => continue,
            }
        }
    }

    if count == 0 {
        println!("No messages could be decrypted.");
    } else {
        println!("\n{} message(s).", count);
    }

    Ok(())
}
