use anyhow::{bail, Result};

use crate::config::NostaroConfig;

pub async fn add(url: &str) -> Result<()> {
    let mut config = NostaroConfig::load()?;

    if config.relays.contains(&url.to_string()) {
        println!("Relay {} is already configured.", url);
        return Ok(());
    }

    config.relays.push(url.to_string());
    config.save()?;
    println!("Added relay: {}", url);

    Ok(())
}

pub async fn remove(url: &str) -> Result<()> {
    let mut config = NostaroConfig::load()?;

    let original_len = config.relays.len();
    config.relays.retain(|r| r != url);

    if config.relays.len() == original_len {
        bail!("Relay {} is not in the configuration.", url);
    }

    config.save()?;
    println!("Removed relay: {}", url);

    Ok(())
}

pub async fn list() -> Result<()> {
    let config = NostaroConfig::load()?;

    let active = config.active_relays();

    if active.is_empty() {
        println!("No relays configured. Add one with: nostaro relay add <url>");
        return Ok(());
    }

    println!("Active relays:");
    for relay in &active {
        let is_default = config.default_relays.contains(relay);
        let label = if is_default && config.relays.is_empty() {
            " (default)"
        } else {
            ""
        };
        println!("  - {}{}", relay, label);
    }

    Ok(())
}
