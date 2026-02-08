use anyhow::Result;
use nostr_sdk::prelude::*;
use std::time::Duration;

use crate::config::NostaroConfig;

pub async fn create_client(keys: &Keys, config: &NostaroConfig) -> Result<Client> {
    let client = Client::builder().signer(keys.clone()).build();

    for relay in config.active_relays() {
        client.add_relay(&relay).await?;
    }

    client.connect().await;

    Ok(client)
}

pub async fn post_note(client: &Client, content: &str) -> Result<()> {
    let builder = EventBuilder::text_note(content);
    client.send_event_builder(builder).await?;
    Ok(())
}

pub async fn fetch_timeline(client: &Client, limit: usize) -> Result<Vec<Event>> {
    let filter = Filter::new().kind(Kind::TextNote).limit(limit);
    let events = client
        .fetch_events(filter, Duration::from_secs(10))
        .await?;
    let mut events: Vec<Event> = events.into_iter().collect();
    events.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(events)
}

pub async fn fetch_timeline_for_authors(
    client: &Client,
    authors: &[PublicKey],
    limit: usize,
) -> Result<Vec<Event>> {
    let filter = Filter::new()
        .kind(Kind::TextNote)
        .authors(authors.to_vec())
        .limit(limit);
    let events = client
        .fetch_events(filter, Duration::from_secs(10))
        .await?;
    let mut events: Vec<Event> = events.into_iter().collect();
    events.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(events)
}

pub async fn fetch_profile(client: &Client, pubkey: &PublicKey) -> Result<Option<Metadata>> {
    let metadata = client
        .fetch_metadata(*pubkey, Duration::from_secs(10))
        .await?;
    Ok(metadata)
}

pub async fn set_metadata(client: &Client, metadata: &Metadata) -> Result<()> {
    client.set_metadata(metadata).await?;
    Ok(())
}

pub async fn fetch_contacts(client: &Client, pubkey: &PublicKey) -> Result<Vec<PublicKey>> {
    let filter = Filter::new()
        .kind(Kind::ContactList)
        .author(*pubkey)
        .limit(1);

    let events = client
        .fetch_events(filter, Duration::from_secs(10))
        .await?;

    if let Some(event) = events.into_iter().next() {
        let mut contacts = Vec::new();
        for tag in event.tags {
            if let Some(TagStandard::PublicKey { public_key, .. }) = tag.as_standardized() {
                contacts.push(*public_key);
            }
        }
        Ok(contacts)
    } else {
        Ok(Vec::new())
    }
}

pub async fn publish_contact_list(client: &Client, contacts: &[PublicKey]) -> Result<()> {
    let mut tags = Vec::new();
    for contact in contacts {
        tags.push(Tag::public_key(*contact));
    }

    let builder = EventBuilder::new(Kind::ContactList, "").tags(tags);
    client.send_event_builder(builder).await?;

    Ok(())
}

pub async fn fetch_event_by_id(client: &Client, event_id: &EventId) -> Result<Option<Event>> {
    let filter = Filter::new().id(*event_id);

    let events = client
        .fetch_events(filter, Duration::from_secs(10))
        .await?;

    Ok(events.into_iter().next())
}
