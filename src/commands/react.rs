use anyhow::{anyhow, Result};
use nostr_sdk::prelude::*;

use crate::client;
use crate::config::NostaroConfig;
use crate::keys;

pub async fn run(event_id_str: &str, reaction: &str) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let event_id = EventId::parse(event_id_str)
        .or_else(|_| EventId::from_bech32(event_id_str))?;

    let target_event = client::fetch_event_by_id(&nostr_client, &event_id)
        .await?
        .ok_or_else(|| anyhow!("Event not found: {}", event_id_str))?;

    let tags = vec![
        Tag::event(event_id),
        Tag::public_key(target_event.pubkey),
        Tag::custom(
            TagKind::Custom("k".into()),
            vec![target_event.kind.as_u16().to_string()],
        ),
    ];

    let builder = EventBuilder::new(Kind::Reaction, reaction).tags(tags);
    nostr_client.send_event_builder(builder).await?;

    println!(
        "Reacted with '{}' to event {}",
        reaction,
        &event_id.to_hex()[..8]
    );

    Ok(())
}
