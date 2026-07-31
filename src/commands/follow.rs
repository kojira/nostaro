use anyhow::Result;
use nostr_sdk::prelude::*;

use crate::client;
use crate::config::NostaroConfig;
use crate::keys;
use crate::outln;
use crate::output;
use crate::utils::resolve_pubkey;

/// One entry of a following/followers listing: the npub, the hex pubkey (what
/// a kind:3 `p` tag needs) and the display name, `None` when no kind:0 could be
/// fetched for that user.
struct Entry {
    npub: String,
    hex: String,
    name: Option<String>,
}

async fn describe(nostr_client: &Client, pubkeys: &[PublicKey]) -> Result<Vec<Entry>> {
    let mut entries = Vec::with_capacity(pubkeys.len());
    for pubkey in pubkeys {
        let name = match client::fetch_profile(nostr_client, pubkey).await {
            Ok(Some(metadata)) => Some(metadata.name.unwrap_or_else(|| "Unknown".to_string())),
            _ => None,
        };
        entries.push(Entry {
            npub: pubkey.to_bech32()?,
            hex: pubkey.to_hex(),
            name,
        });
    }
    Ok(entries)
}

/// Emit the listing: one line per user, or a JSON document when the caller
/// asked for `--out-format json`.
fn emit(entries: &[Entry]) -> Result<()> {
    if output::is_json() {
        let users: Vec<serde_json::Value> = entries
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "npub": entry.npub,
                    "hex": entry.hex,
                    "name": entry.name,
                })
            })
            .collect();
        return output::write_json(&serde_json::json!({
            "count": users.len(),
            "users": users,
        }));
    }

    output::open_body()?;
    for entry in entries {
        match &entry.name {
            Some(name) => outln!("  {} ({})", name, entry.npub)?,
            None => outln!("  {}", entry.npub)?,
        }
    }
    Ok(())
}

pub async fn follow(pubkey_str: &str) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let pubkey = resolve_pubkey(pubkey_str)?;

    let mut contacts = client::fetch_contacts(&nostr_client, &keys.public_key()).await?;

    if contacts.contains(&pubkey) {
        println!("Already following {}", pubkey.to_bech32()?);
        nostr_client.disconnect().await;
        return Ok(());
    }

    contacts.push(pubkey);

    client::publish_contact_list(&nostr_client, &contacts).await?;
    println!("Now following {}", pubkey.to_bech32()?);

    nostr_client.disconnect().await;
    Ok(())
}

pub async fn unfollow(pubkey_str: &str) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let pubkey = resolve_pubkey(pubkey_str)?;

    let mut contacts = client::fetch_contacts(&nostr_client, &keys.public_key()).await?;

    if !contacts.contains(&pubkey) {
        println!("Not following {}", pubkey.to_bech32()?);
        nostr_client.disconnect().await;
        return Ok(());
    }

    contacts.retain(|&p| p != pubkey);

    client::publish_contact_list(&nostr_client, &contacts).await?;
    println!("Unfollowed {}", pubkey.to_bech32()?);

    nostr_client.disconnect().await;
    Ok(())
}

pub async fn following(npub_str: Option<&str>) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let target_pubkey = match npub_str {
        Some(s) => resolve_pubkey(s)?,
        None => keys.public_key(),
    };

    let contacts = client::fetch_contacts(&nostr_client, &target_pubkey).await?;

    if contacts.is_empty() {
        if npub_str.is_some() {
            println!("{} is not following anyone.", target_pubkey.to_bech32()?);
        } else {
            println!("You're not following anyone yet.");
        }
        // An empty result is still a result: --out gets an empty listing.
        emit(&[])?;
        nostr_client.disconnect().await;
        return Ok(());
    }

    println!("Following {} user(s):", contacts.len());
    let entries = describe(&nostr_client, &contacts).await?;
    emit(&entries)?;

    nostr_client.disconnect().await;
    Ok(())
}

pub async fn followers(npub_str: Option<&str>) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let target_pubkey = match npub_str {
        Some(s) => resolve_pubkey(s)?,
        None => keys.public_key(),
    };

    let follower_list = client::fetch_followers(&nostr_client, &target_pubkey).await?;

    if follower_list.is_empty() {
        if npub_str.is_some() {
            println!("No followers found for {}.", target_pubkey.to_bech32()?);
        } else {
            println!("No followers found.");
        }
        emit(&[])?;
        nostr_client.disconnect().await;
        return Ok(());
    }

    println!("{} follower(s):", follower_list.len());
    let entries = describe(&nostr_client, &follower_list).await?;
    emit(&entries)?;

    nostr_client.disconnect().await;
    Ok(())
}
