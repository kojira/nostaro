use anyhow::Result;
use chrono::{DateTime, Utc};
use nostr_sdk::prelude::*;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::cache::CacheDb;
use crate::client;
use crate::config::NostaroConfig;
use crate::keys;
use crate::outln;
use crate::output;

/// Resolve who reacted: the npub, the cached display name (if any) and whether
/// it is the local user. Reactions by the local user are never name-resolved,
/// they are always shown as "you".
fn reactor_identity(
    reaction: &Event,
    own_pubkey: PublicKey,
    cache: Option<&CacheDb>,
) -> (String, Option<String>, bool) {
    let npub = reaction.pubkey.to_bech32().unwrap_or_default();
    if reaction.pubkey == own_pubkey {
        return (npub, None, true);
    }
    let name = cache
        .and_then(|cache| cache.get_profile(&reaction.pubkey.to_hex()).ok().flatten())
        .and_then(|profile| profile.display_name.or(profile.name))
        .filter(|name| !name.is_empty());
    (npub, name, false)
}

fn reaction_emoji(reaction: &Event) -> String {
    if reaction.content.is_empty() {
        "+".to_string()
    } else {
        reaction.content.clone()
    }
}

async fn fetch_reactions(
    nostr_client: &Client,
    event_ids: Vec<EventId>,
) -> Result<HashMap<EventId, Vec<Event>>> {
    let filter = Filter::new()
        .kind(Kind::Reaction)
        .events(event_ids)
        .limit(1000);
    let reaction_events = nostr_client
        .fetch_events(filter, Duration::from_secs(10))
        .await?;

    let mut reactions_by_event: HashMap<EventId, Vec<Event>> = HashMap::new();

    for reaction in reaction_events {
        let related_event_ids: Vec<EventId> = reaction
            .tags
            .iter()
            .filter_map(|tag: &Tag| match tag.as_standardized() {
                Some(TagStandard::Event { event_id, .. }) => Some(*event_id),
                _ => None,
            })
            .collect();

        for event_id in related_event_ids {
            reactions_by_event
                .entry(event_id)
                .or_default()
                .push(reaction.clone());
        }
    }

    Ok(reactions_by_event)
}

/// One filter for *all* the missing profiles: kind:0 with every pubkey in a
/// single `authors` list, i.e. a shape a single read can be assembled from, no
/// matter how many people are involved.
///
/// Pure — it takes pubkeys and builds a filter, it talks to no relay.
fn profile_batch_filter(pubkeys: Vec<PublicKey>) -> Filter {
    Filter::new()
        .kind(Kind::Metadata)
        .authors(pubkeys)
        .limit(500)
}

/// Pins the *argument type* — a filter is built from a whole list of pubkeys,
/// not from one. Narrowing this to a single `PublicKey` stops the line
/// compiling.
///
/// That is all it does. It does **not** guarantee the number of round trips:
/// `profile_batch_filter` is pure, so nothing stops a caller from looping over
/// it a pubkey at a time. The one-read property lives in
/// `fetch_and_cache_profiles`, which calls `fetch_events` never more than once
/// — and not at all when the cache already has everyone — and is held by
/// review rather than by the type system.
///
/// Contrast `follow.rs`'s `describe`: the difference is *granularity*, not
/// purity (both functions are pure). `describe` is the whole step from pubkeys
/// to output, and `Entry` has no name field, so a caller that read kind:0
/// anyway would have nowhere to put the result — the shape rules the read out
/// end to end. This function is only the filter-building part of a fetch whose
/// `async fn` one level up does hold a `&Client`; it can rule nothing out
/// beyond its own signature.
///
/// Keep the batch shape anyway: on a global timeline the reactors are
/// strangers, so *every* one of them misses the cache, and a per-reactor lookup
/// is the 979 round trips #8/#9 removed.
const _: fn(Vec<PublicKey>) -> Filter = profile_batch_filter;

async fn fetch_and_cache_profiles(
    nostr_client: &Client,
    pubkeys: Vec<PublicKey>,
    cache: &CacheDb,
) -> Result<()> {
    let missing_pubkeys: Vec<PublicKey> = pubkeys
        .into_iter()
        .filter(|pk| cache.get_profile(&pk.to_hex()).ok().flatten().is_none())
        .collect();

    if missing_pubkeys.is_empty() {
        return Ok(());
    }

    let filter = profile_batch_filter(missing_pubkeys);

    let events = nostr_client
        .fetch_events(filter, Duration::from_secs(10))
        .await?;

    for event in events {
        if let Ok(metadata) = Metadata::from_json(&event.content) {
            let _ = cache.store_profile(
                &event.pubkey.to_hex(),
                metadata.name.as_deref(),
                metadata.display_name.as_deref(),
                metadata.about.as_deref(),
                metadata.picture.as_deref(),
            );
        }
    }

    Ok(())
}

/// The `--out-format json` document.
///
/// Both `timeline` and `timeline --global` go through here, so the two produce
/// the same shape — a caller does not have to branch on which one it ran.
/// `following` is still answered for the global feed (from the one kind:3 read
/// the command already does), so an agent looking at a stranger's note can tell
/// whether it already follows them.
///
/// Pure — it renders what has already been fetched and talks to no relay; the
/// cache is only read for names that are already there.
fn to_json(
    events: &[Event],
    following_set: &HashSet<PublicKey>,
    own_pubkey: PublicKey,
    reactions_by_event: &HashMap<EventId, Vec<Event>>,
    cache: Option<&CacheDb>,
) -> Result<serde_json::Value> {
    let mut notes = Vec::with_capacity(events.len());
    for event in events {
        let reactions: Vec<serde_json::Value> = reactions_by_event
            .get(&event.id)
            .map(|reactions| {
                reactions
                    .iter()
                    .map(|reaction| {
                        let (npub, name, is_self) = reactor_identity(reaction, own_pubkey, cache);
                        serde_json::json!({
                            "emoji": reaction_emoji(reaction),
                            "npub": npub,
                            "name": name,
                            "is_self": is_self,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        notes.push(serde_json::json!({
            "event": serde_json::to_value(event)?,
            "following": following_set.contains(&event.pubkey),
            "is_self": event.pubkey == own_pubkey,
            "reactions": reactions,
        }));
    }

    Ok(serde_json::json!({
        "count": notes.len(),
        "notes": notes,
    }))
}

/// Fetch the newest kind:1 written by the people the user follows (plus the
/// user), topped up with the relay-wide feed when the follow set is too quiet
/// to fill `limit`.
async fn fetch_following(
    nostr_client: &Client,
    authors: &[PublicKey],
    limit: usize,
) -> Result<Vec<Event>> {
    let mut all_events = Vec::new();

    if !authors.is_empty() {
        let followed_events =
            client::fetch_timeline_for_authors(nostr_client, authors, limit).await?;
        all_events.extend(followed_events);
    }

    if all_events.len() < limit {
        let global_events = client::fetch_timeline(nostr_client, limit).await?;
        let seen: HashSet<EventId> = all_events.iter().map(|e| e.id).collect();
        for event in global_events {
            if !seen.contains(&event.id) {
                all_events.push(event);
            }
        }
    }

    Ok(all_events)
}

/// Which feed `timeline` is showing.
///
/// A `bool` would work, and that is what this was: `run(limit, with_reactions,
/// global)` put two `bool`s side by side, where swapping them still compiles
/// and every test still passes. A type the caller has to name cannot be
/// swapped with `with_reactions` by accident. This is the same argument the
/// rest of #10 makes with `const _` — the internal representation, not a new
/// option: the CLI surface is still the `--global` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineScope {
    /// The people you follow (plus you), topped up from the relay when the
    /// follow set is too quiet to fill `limit`.
    Following,
    /// The relay's newest kind:1, whoever wrote them.
    Global,
}

/// [`TimelineScope::Global`] swaps the author filter off — `timeline` stays the
/// follow-based view it has always been, `timeline --global` is "what is on the
/// relay right now, whoever wrote it". Everything after the fetch (caching,
/// reactions, rendering, the JSON document) is shared, so the two views cannot
/// drift apart.
pub async fn run(limit: usize, with_reactions: bool, scope: TimelineScope) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    match scope {
        TimelineScope::Global => println!("Fetching global timeline...\n"),
        TimelineScope::Following => println!("Fetching timeline...\n"),
    }

    // Read once in both modes. This is a single kind:3 — constant in the number
    // of follows — and it is only used to label who you already follow, never
    // expanded into a profile lookup per author.
    let contacts = client::fetch_contacts(&nostr_client, &keys.public_key()).await?;
    let following_set: HashSet<PublicKey> = contacts.iter().copied().collect();

    let mut all_events = match scope {
        // One author-less filter, so the cost does not grow with the size of
        // the follow set.
        TimelineScope::Global => client::fetch_timeline(&nostr_client, limit).await?,
        TimelineScope::Following => {
            let mut authors = contacts.clone();
            authors.push(keys.public_key());
            fetch_following(&nostr_client, &authors, limit).await?
        }
    };

    match scope {
        // Nobody is privileged in the global feed: newest first, full stop.
        // `fetch_timeline` already returns newest-first, so this changes
        // nothing — it is here to say, at the point where the follow-based
        // branch applies its ordering, that the global branch deliberately does
        // not go through that comparator.
        TimelineScope::Global => {
            all_events.sort_by_key(|event| std::cmp::Reverse(event.created_at))
        }
        TimelineScope::Following => all_events.sort_by(|a, b| {
            let a_following = following_set.contains(&a.pubkey) || a.pubkey == keys.public_key();
            let b_following = following_set.contains(&b.pubkey) || b.pubkey == keys.public_key();
            match (a_following, b_following) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => b.created_at.cmp(&a.created_at),
            }
        }),
    }

    all_events.truncate(limit);

    let reactions_by_event = if with_reactions {
        let event_ids: Vec<EventId> = all_events.iter().map(|e| e.id).collect();
        fetch_reactions(&nostr_client, event_ids).await?
    } else {
        HashMap::new()
    };

    // Batch-fetch kind:0 profiles for reactors not yet in cache
    if with_reactions && !reactions_by_event.is_empty() {
        if let Ok(cache) = CacheDb::open() {
            let all_reactor_pubkeys: Vec<PublicKey> = reactions_by_event
                .values()
                .flatten()
                .map(|e| e.pubkey)
                .collect::<HashSet<PublicKey>>()
                .into_iter()
                .collect();
            let _ = fetch_and_cache_profiles(&nostr_client, all_reactor_pubkeys, &cache).await;
        }
    }

    let cache = CacheDb::open().ok();

    // Cache events
    if let Ok(cache) = CacheDb::open() {
        for event in &all_events {
            let tags_json = serde_json::to_string(&event.tags).unwrap_or_default();
            let _ = cache.store_event(
                &event.id.to_hex(),
                &event.pubkey.to_hex(),
                event.kind.as_u16(),
                &event.content,
                event.created_at.as_u64() as i64,
                &tags_json,
                &event.as_json(),
            );
        }
    }

    // An empty timeline is still a result: --out gets an empty listing rather
    // than no file at all, so the body is emitted in every case.
    if all_events.is_empty() {
        println!("No notes found.");
    }

    let own_pubkey = keys.public_key();

    if output::is_json() {
        output::write_json(&to_json(
            &all_events,
            &following_set,
            own_pubkey,
            &reactions_by_event,
            cache.as_ref(),
        )?)?;

        if !all_events.is_empty() {
            println!("\nShowing {} note(s).", all_events.len());
        }
        nostr_client.disconnect().await;
        return Ok(());
    }

    output::open_body()?;
    for event in &all_events {
        let npub = event.pubkey.to_bech32()?;
        let short_npub = &npub;
        let is_following = following_set.contains(&event.pubkey);
        let is_self = event.pubkey == own_pubkey;

        let label = if is_self {
            " [you]"
        } else if is_following {
            " [following]"
        } else {
            ""
        };

        let timestamp = event.created_at.as_u64() as i64;
        let datetime = DateTime::<Utc>::from_timestamp(timestamp, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let note_id = event.id.to_bech32()?;
        outln!("[{}]{} {}", short_npub, label, datetime)?;
        outln!("{}", event.content)?;
        outln!("  id: {}", note_id)?;

        if with_reactions {
            if let Some(reactions) = reactions_by_event.get(&event.id) {
                let mut counts: HashMap<String, (usize, Vec<String>)> = HashMap::new();

                for reaction in reactions {
                    let (reactor_npub, name, is_self) =
                        reactor_identity(reaction, own_pubkey, cache.as_ref());
                    let reactor_label = if is_self {
                        format!("you({})", reactor_npub)
                    } else {
                        match name {
                            Some(name) => format!("{}({})", name, reactor_npub),
                            None => reactor_npub,
                        }
                    };
                    let entry = counts
                        .entry(reaction_emoji(reaction))
                        .or_insert_with(|| (0, Vec::new()));
                    entry.0 += 1;
                    entry.1.push(reactor_label);
                }

                if !counts.is_empty() {
                    let mut parts = Vec::new();

                    for (emoji, (count, names)) in &counts {
                        parts.push(format!("{} x{} ({})", emoji, count, names.join(", ")));
                    }

                    outln!("  Reactions: {}", parts.join(", "))?;
                }
            }
        }

        outln!("{}", "-".repeat(60))?;
    }

    if !all_events.is_empty() {
        println!("\nShowing {} note(s).", all_events.len());
    }

    nostr_client.disconnect().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(keys: &Keys, content: &str) -> Event {
        EventBuilder::text_note(content)
            .sign_with_keys(keys)
            .unwrap()
    }

    /// The key set of every note in the document, which is what a caller has to
    /// be able to rely on.
    fn note_key_sets(document: &serde_json::Value) -> Vec<Vec<String>> {
        document["notes"]
            .as_array()
            .expect("notes is an array")
            .iter()
            .map(|note| {
                note.as_object()
                    .expect("a note is an object")
                    .keys()
                    .cloned()
                    .collect()
            })
            .collect()
    }

    /// One filter holds everyone: 500 strangers ride in a single `authors`
    /// list, so the batch does not have to be split up as it grows.
    ///
    /// What this does *not* assert is how many times the filter is sent —
    /// that `fetch_and_cache_profiles` calls `fetch_events` once is not fixed
    /// here (see the note on `profile_batch_filter`). This covers the shape the
    /// one read is built from.
    #[test]
    fn profiles_are_resolved_in_a_single_batched_filter() {
        let pubkeys: Vec<PublicKey> = (0..500).map(|_| Keys::generate().public_key()).collect();
        let filter = profile_batch_filter(pubkeys.clone());

        let authors = filter.authors.expect("the batch names its authors");
        assert_eq!(
            authors.len(),
            pubkeys.len(),
            "every pubkey rides in the same filter"
        );
        let kinds = filter.kinds.expect("the batch is restricted to one kind");
        assert_eq!(kinds.len(), 1);
        assert!(kinds.contains(&Kind::Metadata));
    }

    /// Note authors are never name-resolved, so the kind:0 path is not reached
    /// for them at all (only reactors under `--with-reactions` are): `to_json`
    /// renders bare events and the text body prints npubs.
    #[test]
    fn the_json_body_carries_no_profile_name_for_note_authors() {
        let author = Keys::generate();
        let events = vec![note(&author, "hello from a stranger")];
        let document = to_json(
            &events,
            &HashSet::new(),
            Keys::generate().public_key(),
            &HashMap::new(),
            None,
        )
        .unwrap();

        assert_eq!(
            note_key_sets(&document),
            vec![vec!["event", "following", "is_self", "reactions"]],
            "a note is the raw event plus follow/self/reactions — no resolved name"
        );
        assert_eq!(
            document["notes"][0]["event"],
            serde_json::to_value(&events[0]).unwrap(),
            "the embedded event is the event verbatim — nothing is added to it"
        );
    }

    /// `timeline` and `timeline --global` render through the same `to_json`, so
    /// a caller reads one shape either way — even though what comes back
    /// differs: the follow-based view is people you follow, the global view is
    /// whoever posted.
    #[test]
    fn global_and_follow_based_json_share_one_shape() {
        let me = Keys::generate();
        let followed = Keys::generate();
        let stranger = Keys::generate();
        let following: HashSet<PublicKey> = [followed.public_key()].into_iter().collect();

        // What `timeline` sees: the follow set plus yourself.
        let follow_based = to_json(
            &[note(&me, "mine"), note(&followed, "someone i follow")],
            &following,
            me.public_key(),
            &HashMap::new(),
            None,
        )
        .unwrap();

        // What `timeline --global` sees: the same follow set is still known (one
        // kind:3 read), but the notes come from anyone.
        let global = to_json(
            &[
                note(&stranger, "someone i do not follow"),
                note(&followed, "someone i follow"),
            ],
            &following,
            me.public_key(),
            &HashMap::new(),
            None,
        )
        .unwrap();

        let expected = vec![
            vec!["event", "following", "is_self", "reactions"],
            vec!["event", "following", "is_self", "reactions"],
        ];
        for document in [&follow_based, &global] {
            assert_eq!(document["count"], 2);
            assert_eq!(note_key_sets(document), expected);
        }

        assert_eq!(follow_based["notes"][0]["is_self"], true);
        assert_eq!(follow_based["notes"][1]["following"], true);

        assert_eq!(
            global["notes"][0]["following"], false,
            "a stranger's note is reported as not-followed, not dropped"
        );
        assert_eq!(global["notes"][0]["is_self"], false);
        assert_eq!(
            global["notes"][1]["following"], true,
            "the global feed still tells you who you already follow"
        );
    }

    /// An empty global feed is still a document: `count: 0` with an empty array,
    /// never a missing `notes` key.
    #[test]
    fn json_of_an_empty_timeline_is_still_a_document() {
        let document = to_json(
            &[],
            &HashSet::new(),
            Keys::generate().public_key(),
            &HashMap::new(),
            None,
        )
        .unwrap();
        assert_eq!(document["count"], 0);
        assert_eq!(document["notes"].as_array().unwrap().len(), 0);
    }
}
