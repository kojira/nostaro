use anyhow::{bail, Result};
use nostr_sdk::prelude::*;

use crate::client;
use crate::config::NostaroConfig;
use crate::keys;

pub async fn run(kind: u16, tags: Vec<String>, content: &str) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let mut parsed_tags = Vec::new();
    for tag_str in &tags {
        let parts: Vec<String> = tag_str.split(',').map(|s| s.to_string()).collect();
        if parts.len() < 2 {
            bail!("Invalid tag format: '{}'. Expected 'key,value[,value...]'", tag_str);
        }
        parsed_tags.push(Tag::parse(parts)?);
    }

    println!("Publishing kind:{} event...", kind);
    let builder = EventBuilder::new(Kind::from(kind), content).tags(parsed_tags);
    let output = nostr_client.send_event_builder(builder).await?;
    println!("Event published! ID: {}", output.id().to_hex());

    Ok(())
}
