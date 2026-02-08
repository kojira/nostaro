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

pub async fn reply_note(client: &Client, reply_to: &Event, content: &str) -> Result<()> {
    let reply_id_hex = reply_to.id.to_hex();
    let tags = vec![
        Tag::parse(["e", &reply_id_hex, "", "reply"])?,
        Tag::public_key(reply_to.pubkey),
    ];
    let builder = EventBuilder::text_note(content).tags(tags);
    client.send_event_builder(builder).await?;
    Ok(())
}

pub async fn repost_event(client: &Client, event: &Event) -> Result<()> {
    let builder = EventBuilder::repost(event, None);
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

pub async fn search_notes(client: &Client, query: &str, limit: usize) -> Result<Vec<Event>> {
    let filter = Filter::new()
        .kind(Kind::TextNote)
        .search(query)
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

pub async fn send_dm(client: &Client, receiver: PublicKey, message: &str) -> Result<()> {
    client
        .send_private_msg(receiver, message, [])
        .await?;
    Ok(())
}

pub async fn send_dm_nip04(client: &Client, keys: &Keys, receiver: PublicKey, message: &str) -> Result<()> {
    use nostr_sdk::nips::nip04;

    let encrypted = nip04::encrypt(keys.secret_key(), &receiver, message)?;
    let tags = vec![Tag::public_key(receiver)];
    let builder = EventBuilder::new(Kind::EncryptedDirectMessage, encrypted).tags(tags);
    client.send_event_builder(builder).await?;
    Ok(())
}

pub async fn fetch_gift_wraps(client: &Client, pubkey: &PublicKey, limit: usize) -> Result<Vec<Event>> {
    let filter = Filter::new()
        .kind(Kind::GiftWrap)
        .pubkey(*pubkey)
        .limit(limit);
    let events = client
        .fetch_events(filter, Duration::from_secs(15))
        .await?;
    Ok(events.into_iter().collect())
}

pub async fn fetch_nip04_dms(client: &Client, pubkey: &PublicKey, limit: usize) -> Result<Vec<Event>> {
    // Fetch DMs where user is author or recipient
    let filter_sent = Filter::new()
        .kind(Kind::EncryptedDirectMessage)
        .author(*pubkey)
        .limit(limit);

    let filter_received = Filter::new()
        .kind(Kind::EncryptedDirectMessage)
        .pubkey(*pubkey)
        .limit(limit);

    let mut all_events = Vec::new();

    let sent = client
        .fetch_events(filter_sent, Duration::from_secs(10))
        .await?;
    all_events.extend(sent.into_iter());

    let received = client
        .fetch_events(filter_received, Duration::from_secs(10))
        .await?;
    all_events.extend(received.into_iter());

    // Remove duplicates and sort by timestamp
    all_events.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    all_events.dedup_by(|a, b| a.id == b.id);

    Ok(all_events)
}

pub async fn fetch_channels(client: &Client, limit: usize) -> Result<Vec<Event>> {
    let filter = Filter::new()
        .kind(Kind::ChannelCreation)
        .limit(limit);
    let events = client
        .fetch_events(filter, Duration::from_secs(10))
        .await?;
    let mut events: Vec<Event> = events.into_iter().collect();
    events.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(events)
}

pub async fn fetch_channel_messages(
    client: &Client,
    channel_id: &EventId,
    limit: usize,
) -> Result<Vec<Event>> {
    let filter = Filter::new()
        .kind(Kind::ChannelMessage)
        .event(*channel_id)
        .limit(limit);
    let events = client
        .fetch_events(filter, Duration::from_secs(10))
        .await?;
    let mut events: Vec<Event> = events.into_iter().collect();
    events.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(events)
}

pub async fn create_channel(client: &Client, content: &str) -> Result<EventId> {
    let builder = EventBuilder::new(Kind::ChannelCreation, content);
    let output = client.send_event_builder(builder).await?;
    Ok(*output.id())
}

pub async fn edit_channel(
    client: &Client,
    channel_id: &EventId,
    content: &str,
    relay_url: &str,
) -> Result<()> {
    let ch_hex = channel_id.to_hex();
    let tags = vec![Tag::parse(["e", &ch_hex, relay_url])?];
    let builder = EventBuilder::new(Kind::ChannelMetadata, content).tags(tags);
    client.send_event_builder(builder).await?;
    Ok(())
}

pub async fn post_channel_message(
    client: &Client,
    channel_id: &EventId,
    content: &str,
) -> Result<()> {
    let ch_hex = channel_id.to_hex();
    let tags = vec![
        Tag::parse(["e", &ch_hex, "", "root"])?,
    ];
    let builder = EventBuilder::new(Kind::ChannelMessage, content).tags(tags);
    client.send_event_builder(builder).await?;
    Ok(())
}
