use anyhow::{bail, Result};
use nostr_sdk::prelude::*;

use crate::cache::CacheDb;
use crate::client;
use crate::config::NostaroConfig;
use crate::keys;

pub async fn show(pubkey_str: Option<&str>) -> Result<()> {
    let config = NostaroConfig::load()?;
    let own_keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&own_keys, &config).await?;

    let pubkey = match pubkey_str {
        Some(pk) => PublicKey::parse(pk)?,
        None => own_keys.public_key(),
    };

    let npub = pubkey.to_bech32()?;
    println!("Fetching profile for {}...\n", &npub[..12.min(npub.len())]);

    let metadata = client::fetch_profile(&nostr_client, &pubkey).await?;

    if let Some(ref metadata) = metadata {
        if let Some(ref name) = metadata.name {
            println!("Name:         {}", name);
        }
        if let Some(ref display_name) = metadata.display_name {
            println!("Display Name: {}", display_name);
        }
        if let Some(ref about) = metadata.about {
            println!("About:        {}", about);
        }
        if let Some(ref picture) = metadata.picture {
            println!("Picture:      {}", picture);
        }
        if let Some(ref banner) = metadata.banner {
            println!("Banner:       {}", banner);
        }
        if let Some(ref website) = metadata.website {
            println!("Website:      {}", website);
        }
        if let Some(ref lud16) = metadata.lud16 {
            println!("Lud16:        {}", lud16);
        }
        if let Some(ref nip05) = metadata.nip05 {
            println!("NIP-05:       {}", nip05);
        }

        // Cache the profile
        if let Ok(cache) = CacheDb::open() {
            let _ = cache.store_profile(
                &pubkey.to_hex(),
                metadata.name.as_deref(),
                metadata.display_name.as_deref(),
                metadata.about.as_deref(),
                metadata.picture.as_ref().map(|u| u.as_str()),
            );
        }
    } else {
        println!("No profile metadata found.");
    }
    println!("Npub:         {}", npub);

    let nprofile = Nip19Profile::new(pubkey, Vec::<String>::new())?;
    println!("Nprofile:     {}", nprofile.to_bech32()?);

    Ok(())
}

pub async fn set(
    name: Option<&str>,
    display_name: Option<&str>,
    about: Option<&str>,
    picture: Option<&str>,
    lud16: Option<&str>,
    lud06: Option<&str>,
    nip05: Option<&str>,
    banner: Option<&str>,
    website: Option<&str>,
) -> Result<()> {
    if name.is_none()
        && display_name.is_none()
        && about.is_none()
        && picture.is_none()
        && lud16.is_none()
        && lud06.is_none()
        && nip05.is_none()
        && banner.is_none()
        && website.is_none()
    {
        bail!("At least one field must be specified (--name, --display-name, --about, --picture, --lud16, --lud06, --nip05, --banner, --website)");
    }

    let config = NostaroConfig::load()?;
    let own_keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&own_keys, &config).await?;

    let mut metadata = client::fetch_profile(&nostr_client, &own_keys.public_key())
        .await?
        .unwrap_or_else(Metadata::new);

    if let Some(v) = name {
        metadata = metadata.name(v);
    }
    if let Some(v) = display_name {
        metadata = metadata.display_name(v);
    }
    if let Some(v) = about {
        metadata = metadata.about(v);
    }
    if let Some(v) = picture {
        let url = Url::parse(v)?;
        metadata = metadata.picture(url);
    }
    if let Some(v) = lud16 {
        metadata = metadata.lud16(v);
    }
    if let Some(v) = lud06 {
        metadata = metadata.lud06(v);
    }
    if let Some(v) = nip05 {
        metadata = metadata.nip05(v);
    }
    if let Some(v) = banner {
        let url = Url::parse(v)?;
        metadata = metadata.banner(url);
    }
    if let Some(v) = website {
        let url = Url::parse(v)?;
        metadata = metadata.website(url);
    }

    println!("Setting profile metadata...");
    client::set_metadata(&nostr_client, &metadata).await?;
    println!("Profile updated successfully!");

    Ok(())
}
