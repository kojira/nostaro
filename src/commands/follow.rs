use anyhow::Result;
use nostr_sdk::prelude::*;

use crate::client;
use crate::config::NostaroConfig;
use crate::keys;
use crate::utils::resolve_pubkey;

pub async fn follow(pubkey_str: &str) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let pubkey = resolve_pubkey(pubkey_str)?;

    let mut contacts = client::fetch_contacts(&nostr_client, &keys.public_key()).await?;

    if contacts.contains(&pubkey) {
        println!("Already following {}", pubkey.to_bech32()?);
        return Ok(());
    }

    contacts.push(pubkey);

    client::publish_contact_list(&nostr_client, &contacts).await?;
    println!("Now following {}", pubkey.to_bech32()?);

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
        return Ok(());
    }

    contacts.retain(|&p| p != pubkey);

    client::publish_contact_list(&nostr_client, &contacts).await?;
    println!("Unfollowed {}", pubkey.to_bech32()?);

    Ok(())
}

pub async fn following() -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let contacts = client::fetch_contacts(&nostr_client, &keys.public_key()).await?;

    if contacts.is_empty() {
        println!("You're not following anyone yet.");
        return Ok(());
    }

    println!("Following {} user(s):", contacts.len());
    for contact in contacts {
        let npub = contact.to_bech32()?;
        if let Ok(Some(metadata)) = client::fetch_profile(&nostr_client, &contact).await {
            let name = metadata.name.unwrap_or_else(|| "Unknown".to_string());
            println!("  {} ({})", name, npub);
        } else {
            println!("  {}", npub);
        }
    }

    Ok(())
}
