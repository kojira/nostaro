use anyhow::{bail, Result};
use nostr_sdk::prelude::*;
use std::time::Duration;

use crate::config::NostaroConfig;

pub async fn create_client(keys: &Keys, config: &NostaroConfig) -> Result<Client> {
    create_client_with_relay_list(keys, &config.active_relays()).await
}

/// Build a client connected only to the given relays, ignoring the config's relay list.
///
/// Used by `watch --relay <url>` so callers (e.g. OpenCrab) can pin the exact relay set
/// without it being overridden by whatever is in the account's config.toml.
pub async fn create_client_with_relay_list(keys: &Keys, relay_urls: &[String]) -> Result<Client> {
    let client = Client::builder().signer(keys.clone()).build();

    for relay in relay_urls {
        client.add_relay(relay).await?;
    }

    client.connect().await;

    Ok(client)
}

/// Turn the per-relay outcome of a publish into warnings, or an error when the
/// event reached nobody.
///
/// `send_event_builder` answers `Ok` even when **every** relay rejected the
/// event — the refusals only appear in `Output::failed` — so without this check
/// nostaro reports success for an event nobody stored. That is a realistic
/// outcome for a large kind:3, since relays enforce event-size and tag-count
/// limits.
pub fn check_publish_output<T>(output: &Output<T>) -> Result<()>
where
    T: std::fmt::Debug,
{
    for (relay, reason) in &output.failed {
        eprintln!("Warning: {} rejected the event: {}", relay, reason);
    }

    if output.success.is_empty() {
        if output.failed.is_empty() {
            bail!("no relay accepted the event (no relay answered)");
        }
        let reasons: Vec<String> = output
            .failed
            .iter()
            .map(|(relay, reason)| format!("{}: {}", relay, reason))
            .collect();
        bail!("no relay accepted the event ({})", reasons.join("; "));
    }

    Ok(())
}

/// Publish an event and confirm at least one relay accepted it.
pub async fn publish(client: &Client, builder: EventBuilder) -> Result<Output<EventId>> {
    let output = client.send_event_builder(builder).await?;
    check_publish_output(&output)?;
    Ok(output)
}

pub async fn post_note(client: &Client, content: &str) -> Result<()> {
    let builder = EventBuilder::text_note(content);
    publish(client, builder).await?;
    Ok(())
}

pub async fn reply_note(client: &Client, reply_to: &Event, content: &str) -> Result<()> {
    let reply_id_hex = reply_to.id.to_hex();
    let tags = vec![
        Tag::parse(["e", &reply_id_hex, "", "reply"])?,
        Tag::public_key(reply_to.pubkey),
    ];
    let builder = EventBuilder::text_note(content).tags(tags);
    publish(client, builder).await?;
    Ok(())
}

pub async fn repost_event(client: &Client, event: &Event) -> Result<()> {
    let builder = EventBuilder::repost(event, None);
    publish(client, builder).await?;
    Ok(())
}

/// The filter behind the global timeline: the newest kind:1, **with no author
/// constraint at all**.
///
/// Pure — it takes a limit and builds a filter, it talks to no relay.
pub fn global_timeline_filter(limit: usize) -> Filter {
    Filter::new().kind(Kind::TextNote).limit(limit)
}

/// The global timeline is built from a limit and nothing else: no
/// `&[PublicKey]`, no `&Client`, no `async`. This pins that shape — narrowing
/// the global feed back to a follow set means `global_timeline_filter` has to
/// take authors, and this line stops compiling. If you are here because of that
/// error, you are turning "what is happening on the relay" back into "what the
/// people I already follow said", which is the gap #10 exists to close.
const _: fn(usize) -> Filter = global_timeline_filter;

pub async fn fetch_timeline(client: &Client, limit: usize) -> Result<Vec<Event>> {
    let events = client
        .fetch_events(global_timeline_filter(limit), Duration::from_secs(10))
        .await?;
    let mut events: Vec<Event> = events.into_iter().collect();
    events.sort_by_key(|e| std::cmp::Reverse(e.created_at));
    Ok(events)
}

/// The filter behind the follow-based timeline: the newest kind:1 **from these
/// authors**. The counterpart of [`global_timeline_filter`].
pub fn timeline_filter_for_authors(authors: &[PublicKey], limit: usize) -> Filter {
    Filter::new()
        .kind(Kind::TextNote)
        .authors(authors.to_vec())
        .limit(limit)
}

pub async fn fetch_timeline_for_authors(
    client: &Client,
    authors: &[PublicKey],
    limit: usize,
) -> Result<Vec<Event>> {
    let filter = timeline_filter_for_authors(authors, limit);
    let events = client.fetch_events(filter, Duration::from_secs(10)).await?;
    let mut events: Vec<Event> = events.into_iter().collect();
    events.sort_by_key(|e| std::cmp::Reverse(e.created_at));
    Ok(events)
}

pub async fn search_notes(client: &Client, query: &str, limit: usize) -> Result<Vec<Event>> {
    let filter = Filter::new()
        .kind(Kind::TextNote)
        .search(query)
        .limit(limit);
    let events = client.fetch_events(filter, Duration::from_secs(10)).await?;
    let mut events: Vec<Event> = events.into_iter().collect();
    events.sort_by_key(|e| std::cmp::Reverse(e.created_at));
    Ok(events)
}

pub async fn fetch_profile(client: &Client, pubkey: &PublicKey) -> Result<Option<Metadata>> {
    fetch_profile_with_timeout(client, pubkey, Duration::from_secs(10)).await
}

pub async fn fetch_profile_with_timeout(
    client: &Client,
    pubkey: &PublicKey,
    timeout: Duration,
) -> Result<Option<Metadata>> {
    let metadata = client.fetch_metadata(*pubkey, timeout).await?;
    Ok(metadata)
}

pub async fn set_metadata(client: &Client, metadata: &Metadata) -> Result<()> {
    let output = client.set_metadata(metadata).await?;
    check_publish_output(&output)?;
    Ok(())
}

pub async fn fetch_contacts(client: &Client, pubkey: &PublicKey) -> Result<Vec<PublicKey>> {
    let filter = Filter::new()
        .kind(Kind::ContactList)
        .author(*pubkey)
        .limit(1);

    let events = client.fetch_events(filter, Duration::from_secs(10)).await?;

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

pub async fn fetch_followers(client: &Client, pubkey: &PublicKey) -> Result<Vec<PublicKey>> {
    let filter = Filter::new().kind(Kind::ContactList).pubkey(*pubkey);

    let events: Vec<Event> = client
        .fetch_events(filter, Duration::from_secs(15))
        .await?
        .into_iter()
        .collect();

    // Deduplicate by author: keep only the latest ContactList per author
    let mut latest: std::collections::HashMap<PublicKey, Timestamp> =
        std::collections::HashMap::new();
    for event in &events {
        let entry = latest.entry(event.pubkey).or_insert(event.created_at);
        if event.created_at > *entry {
            *entry = event.created_at;
        }
    }

    // Collect only authors whose latest ContactList still contains the target pubkey
    let mut followers = Vec::new();
    for event in &events {
        if Some(&event.created_at) == latest.get(&event.pubkey) {
            let has_target = event.tags.iter().any(|tag: &Tag| {
                matches!(tag.as_standardized(), Some(TagStandard::PublicKey { public_key, .. }) if *public_key == *pubkey)
            });
            if has_target {
                followers.push(event.pubkey);
            }
        }
    }

    Ok(followers)
}

pub async fn publish_contact_list(client: &Client, contacts: &[PublicKey]) -> Result<()> {
    let mut tags = Vec::new();
    for contact in contacts {
        tags.push(Tag::public_key(*contact));
    }

    let builder = EventBuilder::new(Kind::ContactList, "").tags(tags);
    publish(client, builder).await?;

    Ok(())
}

pub async fn fetch_event_by_id(client: &Client, event_id: &EventId) -> Result<Option<Event>> {
    let filter = Filter::new().id(*event_id);

    let events = client.fetch_events(filter, Duration::from_secs(10)).await?;

    Ok(events.into_iter().next())
}

pub async fn send_dm(client: &Client, receiver: PublicKey, message: &str) -> Result<()> {
    let output = client.send_private_msg(receiver, message, []).await?;
    check_publish_output(&output)?;
    Ok(())
}

pub async fn send_dm_nip04(
    client: &Client,
    keys: &Keys,
    receiver: PublicKey,
    message: &str,
) -> Result<()> {
    use nostr_sdk::nips::nip04;

    let encrypted = nip04::encrypt(keys.secret_key(), &receiver, message)?;
    let tags = vec![Tag::public_key(receiver)];
    let builder = EventBuilder::new(Kind::EncryptedDirectMessage, encrypted).tags(tags);
    publish(client, builder).await?;
    Ok(())
}

pub async fn fetch_gift_wraps(
    client: &Client,
    pubkey: &PublicKey,
    limit: usize,
) -> Result<Vec<Event>> {
    let filter = Filter::new()
        .kind(Kind::GiftWrap)
        .pubkey(*pubkey)
        .limit(limit);
    let events = client.fetch_events(filter, Duration::from_secs(15)).await?;
    Ok(events.into_iter().collect())
}

pub async fn fetch_nip04_dms(
    client: &Client,
    pubkey: &PublicKey,
    limit: usize,
) -> Result<Vec<Event>> {
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
    all_events.extend(sent);

    let received = client
        .fetch_events(filter_received, Duration::from_secs(10))
        .await?;
    all_events.extend(received);

    // Remove duplicates and sort by timestamp
    all_events.sort_by_key(|e| std::cmp::Reverse(e.created_at));
    all_events.dedup_by(|a, b| a.id == b.id);

    Ok(all_events)
}

pub async fn fetch_channels(client: &Client, limit: usize) -> Result<Vec<Event>> {
    let filter = Filter::new().kind(Kind::ChannelCreation).limit(limit);
    let events = client.fetch_events(filter, Duration::from_secs(10)).await?;
    let mut events: Vec<Event> = events.into_iter().collect();
    events.sort_by_key(|e| std::cmp::Reverse(e.created_at));
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
    let events = client.fetch_events(filter, Duration::from_secs(10)).await?;
    let mut events: Vec<Event> = events.into_iter().collect();
    events.sort_by_key(|e| e.created_at);
    Ok(events)
}

pub async fn create_channel(client: &Client, content: &str) -> Result<EventId> {
    let builder = EventBuilder::new(Kind::ChannelCreation, content);
    let output = publish(client, builder).await?;
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
    publish(client, builder).await?;
    Ok(())
}

pub async fn post_channel_message(
    client: &Client,
    channel_id: &EventId,
    content: &str,
) -> Result<()> {
    let ch_hex = channel_id.to_hex();
    let tags = vec![Tag::parse(["e", &ch_hex, "", "root"])?];
    let builder = EventBuilder::new(Kind::ChannelMessage, content).tags(tags);
    publish(client, builder).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of the global timeline: no author constraint, so the
    /// relay is free to answer with anyone — including people the user does not
    /// follow.
    #[test]
    fn global_timeline_filter_does_not_narrow_by_author() {
        let filter = global_timeline_filter(20);
        assert!(
            filter.authors.is_none(),
            "the global timeline must not carry an author list: {:?}",
            filter.authors
        );
    }

    /// Contrast with the follow-based timeline, which does carry authors. Same
    /// kind, same limit — the author list is the only difference.
    #[test]
    fn follow_timeline_filter_does_narrow_by_author() {
        let authors: Vec<PublicKey> = (0..3).map(|_| Keys::generate().public_key()).collect();
        let filter = timeline_filter_for_authors(&authors, 20);

        let carried = filter
            .authors
            .expect("the follow timeline filters by author");
        assert_eq!(carried.len(), authors.len());
        for author in &authors {
            assert!(carried.contains(author));
        }
        assert_eq!(filter.kinds, global_timeline_filter(20).kinds);
        assert_eq!(filter.limit, global_timeline_filter(20).limit);
    }

    /// kind:1 only. Other kinds are not part of #10 and must not leak in.
    #[test]
    fn global_timeline_filter_is_text_notes_only() {
        let kinds = global_timeline_filter(20)
            .kinds
            .expect("the global timeline is restricted to one kind");
        assert_eq!(kinds.len(), 1);
        assert!(kinds.contains(&Kind::TextNote));
    }

    #[test]
    fn global_timeline_filter_carries_the_requested_limit() {
        for limit in [1usize, 20, 50, 500] {
            assert_eq!(global_timeline_filter(limit).limit, Some(limit));
        }
    }

    /// kind + limit and nothing else. No since/until/search/ids/tag query: the
    /// goal is "the newest N notes, whoever wrote them", and every extra
    /// dimension is a filter option nobody asked for.
    #[test]
    fn global_timeline_filter_adds_no_other_constraints() {
        let filter = global_timeline_filter(20);
        assert!(filter.ids.is_none());
        assert!(filter.search.is_none());
        assert!(filter.since.is_none());
        assert!(filter.until.is_none());
        assert!(filter.generic_tags.is_empty());
    }
}
