use anyhow::{bail, Result};
use nostr_sdk::prelude::*;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use crate::client;
use crate::config::NostaroConfig;
use crate::keys;
use crate::utils::resolve_pubkey;

/// How stale an event can be (relative to now) before watch drops it as a replay.
const MAX_EVENT_AGE_SECS: u64 = 300;
/// Cap on remembered event IDs before the oldest are evicted, to bound memory growth.
const MAX_SEEN_EVENTS: usize = 1000;
/// Timeout for the best-effort author-name lookup in JSON mode. Kept short (unlike the
/// 10s default used elsewhere) so one slow/unresponsive relay can't stall the whole
/// real-time event stream; author_name is optional in the output schema.
const AUTHOR_NAME_FETCH_TIMEOUT: Duration = Duration::from_secs(3);

/// Tracks recently-seen event IDs to drop duplicates/replays, and rejects events older
/// than `MAX_EVENT_AGE_SECS` (relays sometimes replay old events on resubscribe).
/// Shared by both the Discord-webhook loop and the `--json` loop so a fix to this logic
/// only needs to be made once.
struct EventDeduplicator {
    seen: HashSet<EventId>,
    order: VecDeque<EventId>,
}

impl EventDeduplicator {
    fn new() -> Self {
        Self {
            seen: HashSet::new(),
            order: VecDeque::new(),
        }
    }

    /// Returns true if `event` is fresh and unseen and should be processed.
    fn accept(&mut self, event: &Event) -> bool {
        let now = chrono::Utc::now().timestamp() as u64;
        let created_at = event.created_at.as_u64();
        if now > created_at && now - created_at > MAX_EVENT_AGE_SECS {
            eprintln!(
                "Skipping old event: {} (created_at: {})",
                event.id, created_at
            );
            return false;
        }

        if self.seen.contains(&event.id) {
            return false;
        }
        if self.seen.len() >= MAX_SEEN_EVENTS {
            if let Some(oldest) = self.order.pop_front() {
                self.seen.remove(&oldest);
            }
        }
        self.seen.insert(event.id);
        self.order.push_back(event.id);
        true
    }
}

/// True if `event` carries a `p` tag pointing at `pubkey`, i.e. the event mentions or
/// replies to that pubkey. Shared by the Discord-webhook loop and the `--json` loop so
/// both agree on what "targets me" means.
fn mentions_pubkey(event: &Event, pubkey: &PublicKey) -> bool {
    event.tags.iter().any(|t| {
        matches!(t.as_standardized(), Some(TagStandard::PublicKey { public_key, .. }) if public_key == pubkey)
    })
}

/// The first of `keywords` contained in `content` (case-insensitive). An empty keyword
/// list never matches.
fn matched_keyword(content: &str, keywords: &[String]) -> Option<String> {
    let lowered = content.to_lowercase();
    keywords
        .iter()
        .find(|kw| lowered.contains(&kw.to_lowercase()))
        .cloned()
}

/// Why an event was kept. Only affects how the webhook path labels it; the `--json` path
/// emits the raw event either way.
#[derive(Debug, PartialEq, Eq)]
enum MatchReason {
    /// Carries a `p` tag for the watched pubkey (mention, reply, reaction, zap...).
    Mention,
    /// Content matched this keyword.
    Keyword(String),
    /// Written by one of the `--author`s.
    Author,
    /// No narrowing condition was configured, so everything of the watched kinds passes.
    Unfiltered,
}

/// How the mention / keyword / author conditions are combined (`--match`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum MatchMode {
    /// Keep an event that satisfies **any** condition (default): mentions of us, keyword
    /// hits and posts by watched authors all come through.
    #[default]
    Any,
    /// Keep only events that satisfy **every** configured condition, e.g.
    /// `--author X --keyword foo --match all` = posts by X that also contain "foo".
    All,
}

/// What `watch` subscribes to and which received events it keeps.
///
/// Built once by [`build_watch_filter`] from the command-line arguments (`--npub`,
/// `--kind`, `--keyword`, `--author`, `--mention-only`, `--match`) and used by **both**
/// output modes: `--json` and the Discord webhook differ only in where a matched event
/// goes, never in what is matched.
struct WatchFilter {
    /// Pubkey watched via `p` tags — `--npub` or, by default, our own.
    target_pubkey: PublicKey,
    /// Our own pubkey, so we never echo our own posts back at ourselves.
    own_pubkey: PublicKey,
    /// Kinds to subscribe to. `--kind` when given, otherwise kind:1 + kind:7 so mentions,
    /// replies and reactions all arrive.
    kinds: Vec<Kind>,
    /// Kinds the keyword condition applies to. Explicit `--kind`s are taken at face
    /// value; with the implicit default we only match keywords against kind:1, because
    /// the content of a kind:7 reaction is an emoji, not prose.
    keyword_kinds: Vec<Kind>,
    /// Whether the `p`-tag condition is in play (`--mention-only`, the default). Forced
    /// off under `--only-follows`: the p-tag subscription is a firehose anyone can inject
    /// into, so it is never opened when we are pinning to the follow set.
    mention_only: bool,
    keywords: Vec<String>,
    /// The effective author set the relay filters and the local check use. When
    /// `--only-follows` is on this is `static_authors` unioned with the live follow list
    /// and is refreshed by [`WatchFilter::set_follows`] as the kind:3 list changes.
    authors: Vec<PublicKey>,
    /// The authors given explicitly on the command line (`--author`). Kept apart from
    /// `authors` so the follow list can be merged in (and re-merged on update) without
    /// losing the static ones.
    static_authors: Vec<PublicKey>,
    /// `--only-follows`: pin the watch to the author's own follow list (kind:3). Acts as a
    /// hard gate — a non-followee never passes, whatever else it matches.
    only_follows: bool,
    /// Whether the conditions above are OR'd (`any`) or AND'd (`all`).
    match_mode: MatchMode,
}

/// The single place a [`WatchFilter`] is built. Both output modes call this with the same
/// arguments, so neither can drift away from the other by growing a parameter alone.
///
/// `watching_mentions` is false when only a `--channel` is being watched: there is no
/// mention target then, so the `p`-tag condition is dropped.
#[allow(clippy::too_many_arguments)]
fn build_watch_filter(
    own_pubkey: PublicKey,
    target_pubkey: PublicKey,
    extra_kinds: &[u16],
    mention_only: bool,
    watching_mentions: bool,
    keywords: &[String],
    authors: &[PublicKey],
    match_mode: MatchMode,
    only_follows: bool,
) -> WatchFilter {
    let (kinds, keyword_kinds) = if extra_kinds.is_empty() {
        (vec![Kind::TextNote, Kind::Reaction], vec![Kind::TextNote])
    } else {
        let kinds: Vec<Kind> = extra_kinds.iter().map(|&k| Kind::from(k)).collect();
        (kinds.clone(), kinds)
    };
    WatchFilter {
        target_pubkey,
        own_pubkey,
        kinds,
        keyword_kinds,
        // The follow set replaces the mention subscription under --only-follows: opening a
        // p-tag firehose would defeat the point (anyone can p-tag you).
        mention_only: mention_only && watching_mentions && !only_follows,
        keywords: keywords.to_vec(),
        authors: authors.to_vec(),
        static_authors: authors.to_vec(),
        only_follows,
        match_mode,
    }
}

/// Whether mentions are watched at all. False only when `--channel` is the sole thing
/// being watched: there is no mention target then. `--json` rejects `--channel`, so this
/// is always true in JSON mode — both output modes reach [`build_watch_filter`] through
/// the same derivation.
fn watching_mentions(channel_id: Option<&str>, npub_str: Option<&str>) -> bool {
    channel_id.is_none() || npub_str.is_some()
}

/// Whether to open the general (non-channel) subscriptions. Watching a channel alone
/// needs none: the channel subscription supplies the events and `--author` is applied to
/// them locally.
fn opens_general_subscriptions(watching_mentions: bool, keywords: &[String]) -> bool {
    watching_mentions || !keywords.is_empty()
}

/// The effective `--mention-only` value. `--no-mention-only` turns the p-tag condition
/// off; without it the condition is on. Lives here (rather than inline in `main`) so the
/// derivation is testable and cannot silently change.
pub fn effective_mention_only(mention_only: bool, no_mention_only: bool) -> bool {
    mention_only && !no_mention_only
}

impl WatchFilter {
    /// True when at least one narrowing condition (p tag / keyword / author) is set.
    fn is_narrowed(&self) -> bool {
        self.mention_only || !self.keywords.is_empty() || !self.authors.is_empty()
    }

    /// Whether the keyword condition can apply to this event's kind at all.
    fn keyword_kind(&self, event: &Event) -> bool {
        self.keyword_kinds.contains(&event.kind)
    }

    /// Replace the live follow set (from a kind:3 list), recomputing the effective author
    /// set as `static_authors ∪ follows`. Only meaningful under `--only-follows`; the
    /// dedup keeps `--author` entries from being listed twice.
    fn set_follows(&mut self, follows: &[PublicKey]) {
        let mut merged = self.static_authors.clone();
        for pk in follows {
            if !merged.contains(pk) {
                merged.push(*pk);
            }
        }
        self.authors = merged;
    }

    /// `--author` as a plain membership test. Used on its own for NIP-28 channel
    /// messages, which are selected by the channel subscription but must still honour
    /// `--author`. Under `--only-follows` an empty author set means "allow no one" (the
    /// follow list is empty), never "allow everyone".
    fn author_allowed(&self, event: &Event) -> bool {
        if self.only_follows {
            return self.authors.contains(&event.pubkey);
        }
        self.authors.is_empty() || self.authors.contains(&event.pubkey)
    }

    /// Our own event coming back to us. Dropped in both output modes — unless the user
    /// explicitly asked to watch themselves with `--author`.
    fn is_own_echo(&self, event: &Event) -> bool {
        event.pubkey == self.own_pubkey && !self.authors.contains(&self.own_pubkey)
    }

    /// The relay subscriptions needed to see every event `match_event` could keep.
    ///
    /// - `--match any`: one subscription per condition, so the relay delivers the union.
    /// - `--match all`: a single subscription combining the conditions a relay can
    ///   evaluate (kinds + `p` tag + authors). Keywords are never part of it — relays
    ///   cannot filter by content — so they stay a local check in `match_event`, but they
    ///   do narrow the subscribed kinds, since no other kind could satisfy the AND.
    ///
    /// With no condition at all we subscribe to the bare kinds, which is what
    /// `--no-mention-only` without any other flag explicitly asks for.
    fn subscriptions(&self, since: Timestamp) -> Vec<Filter> {
        // --only-follows narrows *every* subscription to the follow set at the relay (the
        // "line" the issue wants events dropped before): no p-tag firehose, no bare-kind
        // firehose. With an empty follow set nothing is subscribed at all, so nothing
        // streams — the safe direction, never a fall-back to a firehose. Keywords stay a
        // local check (relays can't filter content); in --match all they still narrow the
        // subscribed kinds.
        if self.only_follows {
            if self.authors.is_empty() {
                return Vec::new();
            }
            let kinds = if self.match_mode == MatchMode::All && !self.keywords.is_empty() {
                self.keyword_kinds.clone()
            } else {
                self.kinds.clone()
            };
            return vec![Filter::new()
                .kinds(kinds)
                .authors(self.authors.clone())
                .since(since)];
        }

        if self.match_mode == MatchMode::All {
            let kinds = if self.keywords.is_empty() {
                self.kinds.clone()
            } else {
                self.keyword_kinds.clone()
            };
            let mut filter = Filter::new().kinds(kinds).since(since);
            if self.mention_only {
                filter = filter.pubkey(self.target_pubkey);
            }
            if !self.authors.is_empty() {
                filter = filter.authors(self.authors.clone());
            }
            return vec![filter];
        }

        let mut filters = Vec::new();

        if self.mention_only {
            filters.push(
                Filter::new()
                    .kinds(self.kinds.clone())
                    .pubkey(self.target_pubkey)
                    .since(since),
            );
        }
        if !self.keywords.is_empty() {
            // Relays cannot filter by content, so keywords need their own subscription
            // over the watched kinds, matched locally in `match_event`.
            filters.push(Filter::new().kinds(self.keyword_kinds.clone()).since(since));
        }
        if !self.authors.is_empty() {
            filters.push(
                Filter::new()
                    .kinds(self.kinds.clone())
                    .authors(self.authors.clone())
                    .since(since),
            );
        }
        if filters.is_empty() {
            filters.push(Filter::new().kinds(self.kinds.clone()).since(since));
        }

        filters
    }

    /// Whether to keep `event`, and why. Mirrors `subscriptions` so a relay that
    /// over-delivers (or another subscription's traffic) is narrowed back down locally.
    /// With no condition configured everything is kept, in both match modes.
    fn match_event(&self, event: &Event) -> Option<MatchReason> {
        if self.is_own_echo(event) {
            return None;
        }
        // --only-follows is a hard gate mirroring the authors filter on every
        // subscription: an event from a non-followee never passes, whatever else it
        // matches. An empty follow set therefore drops everything (not a firehose).
        if self.only_follows && !self.authors.contains(&event.pubkey) {
            return None;
        }
        if !self.is_narrowed() {
            return Some(MatchReason::Unfiltered);
        }
        match self.match_mode {
            MatchMode::Any => self.match_any(event),
            MatchMode::All => self.match_all(event),
        }
    }

    /// OR: the first condition that holds decides.
    fn match_any(&self, event: &Event) -> Option<MatchReason> {
        if self.mention_only && mentions_pubkey(event, &self.target_pubkey) {
            return Some(MatchReason::Mention);
        }
        if self.keyword_kind(event) {
            if let Some(kw) = matched_keyword(&event.content, &self.keywords) {
                return Some(MatchReason::Keyword(kw));
            }
        }
        if self.authors.contains(&event.pubkey) {
            return Some(MatchReason::Author);
        }
        None
    }

    /// AND: every configured condition must hold. The reported reason follows the same
    /// precedence as `match_any` so the webhook labels events identically in both modes.
    fn match_all(&self, event: &Event) -> Option<MatchReason> {
        if self.mention_only && !mentions_pubkey(event, &self.target_pubkey) {
            return None;
        }
        let keyword = if self.keywords.is_empty() {
            None
        } else {
            if !self.keyword_kind(event) {
                return None;
            }
            Some(matched_keyword(&event.content, &self.keywords)?)
        };
        if !self.author_allowed(event) {
            return None;
        }

        if self.mention_only {
            Some(MatchReason::Mention)
        } else if let Some(kw) = keyword {
            Some(MatchReason::Keyword(kw))
        } else {
            Some(MatchReason::Author)
        }
    }
}

/// Emoji and label a kind:1 webhook notification gets, by why it matched. Keyword hits
/// are formatted separately (their message includes the matched keyword); everything else
/// comes through here, so a post picked up by `--author` is not mislabelled a mention.
fn text_note_label(reason: Option<&MatchReason>, has_e_tag: bool) -> (&'static str, &'static str) {
    match reason {
        Some(MatchReason::Author) | Some(MatchReason::Unfiltered) => ("📝", "投稿"),
        _ if has_e_tag => ("📩", "リプライ"),
        _ => ("📩", "メンション"),
    }
}

/// Resolve a display name from kind:0 metadata: prefer display_name, fall back to name,
/// treating empty strings as absent. Shared by the Discord and JSON name-lookup paths.
fn resolve_display_name(metadata: &Metadata) -> Option<String> {
    metadata
        .display_name
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| metadata.name.clone().filter(|s| !s.is_empty()))
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    webhook_url: Option<&str>,
    npub_str: Option<&str>,
    channel_id: Option<&str>,
    keywords: &[String],
    extra_kinds: &[u16],
    mention_only: bool,
    authors: &[String],
    relays: &[String],
    json_output: bool,
    match_mode: MatchMode,
    only_follows: bool,
) -> Result<()> {
    if !json_output && webhook_url.is_none() {
        bail!("--webhook is required unless --json is specified");
    }
    if json_output && channel_id.is_some() {
        bail!("--channel is not supported together with --json");
    }
    if only_follows && npub_str.is_some() {
        // --only-follows pins to *your own* follow list; watching someone else's mentions
        // via --npub would need their kind:3, which is a different feature. Fail loudly
        // rather than quietly ignoring one of the two flags.
        bail!("--only-follows cannot be combined with --npub: it always uses your own follow list");
    }

    let config = NostaroConfig::load()?;
    let own_keys = keys::keys_from_config(&config)?;

    let author_pubkeys: Vec<PublicKey> = authors
        .iter()
        .map(|a| resolve_pubkey(a))
        .collect::<Result<Vec<_>>>()?;

    let nostr_client = if relays.is_empty() {
        client::create_client(&own_keys, &config).await?
    } else {
        client::create_client_with_relay_list(&own_keys, relays).await?
    };

    let own_pubkey = own_keys.public_key();
    // `--npub` picks the watched pubkey; without it we watch our own. Same for both
    // output modes.
    let target_pubkey = match npub_str {
        Some(pk) => resolve_pubkey(pk)?,
        None => own_pubkey,
    };
    // Computed before the --json branch so both modes build the filter from exactly the
    // same inputs.
    let watching_mentions = watching_mentions(channel_id, npub_str);

    let mut watch_filter = build_watch_filter(
        own_pubkey,
        target_pubkey,
        extra_kinds,
        mention_only,
        watching_mentions,
        keywords,
        &author_pubkeys,
        match_mode,
        only_follows,
    );

    // --only-follows: seed the author set from the live kind:3 follow list, fail loudly if
    // there is none, and subscribe to future replacements so the set tracks updates. Both
    // output modes go through this, then handle the incoming kind:3 in their own loop via
    // `maybe_apply_follow_update`.
    if only_follows {
        let follows = client::fetch_contacts(&nostr_client, &own_pubkey).await?;
        if follows.is_empty() {
            bail!(
                "--only-follows: no kind:3 follow list found for your pubkey ({}). \
                 Publish a follow list first; refusing to fall back to passing everything.",
                own_pubkey
                    .to_bech32()
                    .unwrap_or_else(|_| own_pubkey.to_hex())
            );
        }
        watch_filter.set_follows(&follows);
        nostr_client
            .subscribe_with_id(
                follows_update_subscription_id(),
                Filter::new()
                    .kind(Kind::ContactList)
                    .author(own_pubkey)
                    .since(Timestamp::now()),
                None,
            )
            .await?;
        eprintln!(
            "--only-follows: pinned to {} followed authors (kind:3 updates tracked)",
            watch_filter.authors.len()
        );
    }

    if json_output {
        return watch_json(&nostr_client, &mut watch_filter, own_pubkey).await;
    }
    let webhook_url = webhook_url.expect("checked above");

    // Channel watch mode
    let watching_channel = channel_id.map(|s| s.to_string());
    let general_watch =
        opens_general_subscriptions(watching_mentions, keywords) || watch_filter.only_follows;

    println!("Webhook: {}", webhook_url);

    if let Some(ref ch_id) = watching_channel {
        println!(
            "Watching NIP-28 channel: {}...",
            &ch_id[..16.min(ch_id.len())]
        );

        let channel_event_id = EventId::from_hex(ch_id)?;
        let filter = Filter::new()
            .kind(Kind::ChannelMessage)
            .event(channel_event_id)
            .since(Timestamp::now());

        nostr_client.subscribe(filter, None).await?;
    }

    // Follow-narrowed subscriptions are tracked by id so `--only-follows` can replace them
    // when the kind:3 list changes; empty for the ordinary (static) subscriptions.
    let mut active_ids: Vec<SubscriptionId> = Vec::new();
    if general_watch {
        describe_filter(&watch_filter)?;
        active_ids = subscribe_watch(&nostr_client, &watch_filter, Timestamp::now()).await?;
    }
    println!("Press Ctrl+C to stop.\n");

    let mut profile_cache: HashMap<PublicKey, (String, Option<String>)> = HashMap::new();
    let http_client = reqwest::Client::new();
    let mut dedup = EventDeduplicator::new();

    let mut notifications = nostr_client.notifications();
    while let Ok(notification) = notifications.recv().await {
        if let RelayPoolNotification::Event { event, .. } = notification {
            if !dedup.accept(&event) {
                continue;
            }

            // A replacement of our own follow list re-narrows the subscriptions; the event
            // itself is not a watched notification, so stop processing it here.
            if maybe_apply_follow_update(
                &nostr_client,
                &event,
                &own_pubkey,
                &mut watch_filter,
                &mut active_ids,
            )
            .await?
            {
                continue;
            }

            // Channel messages are picked by the channel subscription and checked for
            // channel membership below, but `--author` still applies to them. Everything
            // else goes through the shared filter, exactly as in --json mode.
            let reason = if event.kind == Kind::ChannelMessage && watching_channel.is_some() {
                if !watch_filter.author_allowed(&event) {
                    continue;
                }
                None
            } else if general_watch {
                match watch_filter.match_event(&event) {
                    Some(reason) => Some(reason),
                    None => continue,
                }
            } else {
                continue;
            };

            let (sender_name, sender_avatar) =
                get_profile_info(&nostr_client, &event.pubkey, &mut profile_cache).await;

            let note_id = event.id.to_bech32()?;

            let message = match event.kind {
                Kind::ChannelMessage => {
                    if let Some(ref ch_id) = watching_channel {
                        // Check if this message belongs to the watched channel
                        let belongs = event.tags.iter().any(|t| {
                            if let Some(TagStandard::Event {
                                event_id, marker, ..
                            }) = t.as_standardized()
                            {
                                let marker_match =
                                    marker.as_ref().is_some_and(|m| *m == Marker::Root);
                                event_id.to_hex() == *ch_id && marker_match
                            } else {
                                false
                            }
                        });
                        if !belongs {
                            continue;
                        }
                        let npub_str = event.pubkey.to_bech32()?;
                        let msg = format!(
                            "**{}**\nnpub: {}\nnote: {}\n\n{}",
                            sender_name, npub_str, note_id, event.content
                        );
                        msg
                    } else {
                        continue;
                    }
                }
                Kind::TextNote => match reason {
                    Some(MatchReason::Keyword(ref kw)) => format!(
                        "🔍 **keyword match: {}**\n{}\n> {}\nnote: {}",
                        kw, sender_name, event.content, note_id
                    ),
                    ref other => {
                        let has_e_tag = event.tags.iter().any(|t| {
                            matches!(t.as_standardized(), Some(TagStandard::Event { .. }))
                        });
                        let (emoji, label) = text_note_label(other.as_ref(), has_e_tag);
                        format!(
                            "{} **{}** from {}\n> {}\n🔗 {}",
                            emoji, label, sender_name, event.content, note_id
                        )
                    }
                },
                Kind::Reaction => {
                    let emoji = if event.content.is_empty() {
                        "👍"
                    } else {
                        &event.content
                    };
                    let npub_str = event.pubkey.to_bech32()?;

                    // Get the original event ID from e tag
                    let original_event_id = event.tags.iter().find_map(|t| {
                        if let Some(TagStandard::Event { event_id, .. }) = t.as_standardized() {
                            Some(*event_id)
                        } else {
                            None
                        }
                    });

                    let mut original_content_line = String::new();
                    let mut original_note_str = "unknown".to_string();

                    if let Some(orig_id) = original_event_id {
                        original_note_str = orig_id
                            .to_bech32()
                            .unwrap_or_else(|_| "unknown".to_string());

                        // Fetch the original post
                        let filter = Filter::new().id(orig_id).kind(Kind::TextNote).limit(1);
                        if let Ok(events) = nostr_client
                            .fetch_events(filter, std::time::Duration::from_secs(5))
                            .await
                        {
                            if let Some(orig_event) = events.first() {
                                let content: String =
                                    orig_event.content.chars().take(200).collect();
                                let ellipsis = if orig_event.content.chars().count() > 200 {
                                    "..."
                                } else {
                                    ""
                                };
                                original_content_line = format!("\n\n> {}{}", content, ellipsis);
                            }
                        }
                    }

                    format!(
                        "**{}** reacted {}\nnpub: {}{}\nnote: {}\nreaction_note: {}",
                        sender_name,
                        emoji,
                        npub_str,
                        original_content_line,
                        original_note_str,
                        note_id
                    )
                }
                k if k == Kind::from(9735u16) => {
                    // Zap Receipt (NIP-57)
                    let npub_str_val = event
                        .pubkey
                        .to_bech32()
                        .unwrap_or_else(|_| event.pubkey.to_hex());

                    // Parse description tag for zapper info
                    let description_json = event.tags.iter().find_map(|t| {
                        if t.kind() == TagKind::custom("description") {
                            t.content().map(|s| s.to_string())
                        } else {
                            None
                        }
                    });

                    let (zap_message, zapper_npub) = if let Some(desc_json) = description_json {
                        if let Ok(zap_request) =
                            serde_json::from_str::<serde_json::Value>(&desc_json)
                        {
                            let content = zap_request["content"].as_str().unwrap_or("").to_string();
                            let zapper_npub = if let Some(pk_hex) = zap_request["pubkey"].as_str() {
                                PublicKey::from_hex(pk_hex)
                                    .ok()
                                    .and_then(|pk| pk.to_bech32().ok())
                                    .unwrap_or_else(|| pk_hex.to_string())
                            } else {
                                npub_str_val.clone()
                            };
                            (content, zapper_npub)
                        } else {
                            (String::new(), npub_str_val.clone())
                        }
                    } else {
                        (String::new(), npub_str_val.clone())
                    };

                    let has_bolt11 = event
                        .tags
                        .iter()
                        .any(|t| t.kind() == TagKind::custom("bolt11"));

                    if has_bolt11 {
                        if zap_message.is_empty() {
                            format!("⚡ Zap受信！\nfrom: {}\nnote: {}", zapper_npub, note_id)
                        } else {
                            format!(
                                "⚡ Zap受信！\nfrom: {}\nメッセージ: {}\nnote: {}",
                                zapper_npub, zap_message, note_id
                            )
                        }
                    } else {
                        continue;
                    }
                }
                k if extra_kinds.contains(&k.as_u16()) => {
                    // Generic custom kind notification
                    let npub_str_val = event
                        .pubkey
                        .to_bech32()
                        .unwrap_or_else(|_| event.pubkey.to_hex());
                    let content_preview: String = event.content.chars().take(500).collect();
                    let ellipsis = if event.content.chars().count() > 500 {
                        "..."
                    } else {
                        ""
                    };
                    format!(
                        "📡 kind:{} from {}\n> {}{}\nnote: {}",
                        k.as_u16(),
                        npub_str_val,
                        content_preview,
                        ellipsis,
                        note_id
                    )
                }
                _ => continue,
            };

            println!("[{}] {}", chrono::Local::now().format("%H:%M:%S"), message);

            if let Err(e) = send_discord_webhook(
                &http_client,
                webhook_url,
                &message,
                &sender_name,
                sender_avatar.as_deref(),
            )
            .await
            {
                eprintln!("Webhook error: {}", e);
            }
        }
    }

    Ok(())
}

/// Fixed id for the kind:3 replacement subscription, so `--only-follows` keeps exactly one
/// of them open across the run.
fn follows_update_subscription_id() -> SubscriptionId {
    SubscriptionId::new("nostaro-follows-update")
}

/// The pubkeys `p`-tagged by a kind:3 contact list. Mirrors [`client::fetch_contacts`]'s
/// extraction so the initial fetch and a live update agree on what "follows" means.
fn contacts_from_event(event: &Event) -> Vec<PublicKey> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            if let Some(TagStandard::PublicKey { public_key, .. }) = tag.as_standardized() {
                Some(*public_key)
            } else {
                None
            }
        })
        .collect()
}

/// Open the watch subscriptions and return their ids, so `--only-follows` can replace them
/// when the follow list changes. Shared by both output modes so neither can subscribe
/// differently from the other.
async fn subscribe_watch(
    client: &Client,
    filter: &WatchFilter,
    since: Timestamp,
) -> Result<Vec<SubscriptionId>> {
    let mut ids = Vec::new();
    for f in filter.subscriptions(since) {
        let output = client.subscribe(f, None).await?;
        ids.push(output.val);
    }
    Ok(ids)
}

/// If `event` is a replacement of our own follow list under `--only-follows`, refresh the
/// allowed-author set and re-issue the follow-narrowed subscriptions, then report `true` so
/// the caller drops the event (it is not a watched notification). Everything else returns
/// `false` and is handled normally.
///
/// A cleared list leaves an empty author set: [`WatchFilter::subscriptions`] then opens
/// nothing and [`WatchFilter::match_event`] drops everything — the safe direction. We warn
/// loudly rather than silently reverting to a firehose, and never crash the running watch.
async fn maybe_apply_follow_update(
    client: &Client,
    event: &Event,
    own_pubkey: &PublicKey,
    filter: &mut WatchFilter,
    active_ids: &mut Vec<SubscriptionId>,
) -> Result<bool> {
    if !filter.only_follows || event.kind != Kind::ContactList || event.pubkey != *own_pubkey {
        return Ok(false);
    }

    let follows = contacts_from_event(event);
    filter.set_follows(&follows);

    for id in active_ids.drain(..) {
        client.unsubscribe(&id).await;
    }
    let new_ids = subscribe_watch(client, filter, Timestamp::now()).await?;
    active_ids.extend(new_ids);

    if follows.is_empty() {
        eprintln!(
            "WARNING --only-follows: your kind:3 follow list is now empty; nothing will pass \
             until you follow someone again"
        );
    } else {
        eprintln!(
            "--only-follows: follow list updated; now pinned to {} authors",
            filter.authors.len()
        );
    }
    Ok(true)
}

/// Matched event schema emitted on stdout by `watch --json`, one line per event (JSONL).
#[derive(Serialize)]
struct JsonEvent {
    id: String,
    pubkey: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    npub: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    author_name: Option<String>,
    created_at: u64,
    kind: u16,
    content: String,
    tags: Tags,
}

/// Prints the active filter on stdout (webhook mode). The `--json` path prints the same
/// lines on stderr instead, so its stdout stays pure JSONL.
fn describe_filter(filter: &WatchFilter) -> Result<()> {
    for line in filter_description(filter)? {
        println!("{}", line);
    }
    Ok(())
}

/// Human-readable summary of what we subscribed to. Shared by both output modes so the
/// diagnostics can never drift from each other.
fn filter_description(filter: &WatchFilter) -> Result<Vec<String>> {
    let mut lines = vec![format!(
        "Kinds: {:?}",
        filter.kinds.iter().map(|k| k.as_u16()).collect::<Vec<_>>()
    )];
    if filter.mention_only {
        lines.push(format!(
            "Watching mentions of: {}",
            filter.target_pubkey.to_bech32()?
        ));
    }
    if filter.only_follows {
        lines.push(format!(
            "Only-follows: {} followed authors (dynamic, from kind:3)",
            filter.authors.len()
        ));
    } else if !filter.authors.is_empty() {
        lines.push(format!("Authors: {}", filter.authors.len()));
    }
    if !filter.keywords.is_empty() {
        lines.push(format!("Keywords: {:?}", filter.keywords));
    }
    if filter.is_narrowed() {
        lines.push(match filter.match_mode {
            MatchMode::Any => "Match: any (an event matching one condition is kept)".to_string(),
            MatchMode::All => "Match: all (every condition above must hold)".to_string(),
        });
    }
    if !filter.is_narrowed() {
        lines.push(
            "WARNING: no mention/keyword/author filter - every event of these kinds is matched"
                .to_string(),
        );
    }
    Ok(lines)
}

/// Output-agnostic watch loop for consumers like OpenCrab: subscribes through the shared
/// [`WatchFilter`] (identical to the webhook path) and prints one JSON object per matched
/// event to stdout (JSONL). All progress/diagnostic output goes to stderr so stdout stays
/// pure JSONL.
async fn watch_json(
    nostr_client: &Client,
    watch_filter: &mut WatchFilter,
    own_pubkey: PublicKey,
) -> Result<()> {
    eprintln!("Watching (JSON mode)");
    for line in filter_description(watch_filter)? {
        eprintln!("{}", line);
    }
    eprintln!("Press Ctrl+C to stop.\n");

    let mut active_ids = subscribe_watch(nostr_client, watch_filter, Timestamp::now()).await?;

    let mut dedup = EventDeduplicator::new();
    let mut author_name_cache: HashMap<PublicKey, Option<String>> = HashMap::new();

    let mut notifications = nostr_client.notifications();
    while let Ok(notification) = notifications.recv().await {
        if let RelayPoolNotification::Event { event, .. } = notification {
            if !dedup.accept(&event) {
                continue;
            }

            if maybe_apply_follow_update(
                nostr_client,
                &event,
                &own_pubkey,
                watch_filter,
                &mut active_ids,
            )
            .await?
            {
                continue;
            }

            if watch_filter.match_event(&event).is_none() {
                continue;
            }

            match build_json_event(nostr_client, &event, &mut author_name_cache).await {
                Ok(line) => println!("{}", line),
                Err(e) => eprintln!("Failed to serialize event {}: {}", event.id, e),
            }
        }
    }

    Ok(())
}

async fn build_json_event(
    nostr_client: &Client,
    event: &Event,
    author_name_cache: &mut HashMap<PublicKey, Option<String>>,
) -> Result<String> {
    let author_name = get_author_name(nostr_client, &event.pubkey, author_name_cache).await;

    let json_event = JsonEvent {
        id: event.id.to_hex(),
        pubkey: event.pubkey.to_hex(),
        npub: event.pubkey.to_bech32().ok(),
        note_id: event.id.to_bech32().ok(),
        author_name,
        created_at: event.created_at.as_u64(),
        kind: event.kind.as_u16(),
        content: event.content.clone(),
        tags: event.tags.clone(),
    };

    Ok(serde_json::to_string(&json_event)?)
}

async fn get_author_name(
    nostr_client: &Client,
    pubkey: &PublicKey,
    cache: &mut HashMap<PublicKey, Option<String>>,
) -> Option<String> {
    if let Some(name) = cache.get(pubkey) {
        return name.clone();
    }

    let name =
        match client::fetch_profile_with_timeout(nostr_client, pubkey, AUTHOR_NAME_FETCH_TIMEOUT)
            .await
        {
            Ok(Some(metadata)) => resolve_display_name(&metadata),
            _ => None,
        };

    cache.insert(*pubkey, name.clone());
    name
}

async fn get_profile_info(
    nostr_client: &Client,
    pubkey: &PublicKey,
    cache: &mut HashMap<PublicKey, (String, Option<String>)>,
) -> (String, Option<String>) {
    if let Some(info) = cache.get(pubkey) {
        return info.clone();
    }

    let npub = pubkey.to_bech32().unwrap_or_else(|_| pubkey.to_hex());

    let info = match client::fetch_profile(nostr_client, pubkey).await {
        Ok(Some(metadata)) => {
            let display = resolve_display_name(&metadata).unwrap_or_else(|| npub.clone());
            let picture = metadata
                .picture
                .map(|u| u.to_string())
                .filter(|s| !s.is_empty());
            (display, picture)
        }
        _ => (npub, None),
    };

    cache.insert(*pubkey, info.clone());
    info
}

async fn send_discord_webhook(
    client: &reqwest::Client,
    webhook_url: &str,
    content: &str,
    username: &str,
    avatar_url: Option<&str>,
) -> Result<()> {
    let content = if content.chars().count() > 2000 {
        format!("{}...", content.chars().take(1997).collect::<String>())
    } else {
        content.to_string()
    };

    let mut body = serde_json::json!({
        "content": content,
        "username": username,
    });

    if let Some(url) = avatar_url {
        if url.starts_with("http://") || url.starts_with("https://") {
            body["avatar_url"] = serde_json::Value::String(url.to_string());
        }
    }

    let resp = client.post(webhook_url).json(&body).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Discord webhook failed ({}): {}", status, body);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stand-in for the arguments `run` collects, so tests build filters through the very
    /// same `build_watch_filter` the binary uses.
    struct Args {
        own: PublicKey,
        target: PublicKey,
        kinds: Vec<u16>,
        mention_only: bool,
        watching_mentions: bool,
        keywords: Vec<String>,
        authors: Vec<PublicKey>,
        match_mode: MatchMode,
        only_follows: bool,
    }

    impl Args {
        fn new(own: PublicKey) -> Self {
            Self {
                own,
                target: own,
                kinds: vec![],
                mention_only: true,
                watching_mentions: true,
                keywords: vec![],
                authors: vec![],
                match_mode: MatchMode::Any,
                only_follows: false,
            }
        }
        fn only_follows(mut self) -> Self {
            self.only_follows = true;
            self
        }
        fn kinds(mut self, kinds: &[u16]) -> Self {
            self.kinds = kinds.to_vec();
            self
        }
        fn keywords(mut self, keywords: &[&str]) -> Self {
            self.keywords = keywords.iter().map(|k| k.to_string()).collect();
            self
        }
        fn authors(mut self, authors: &[PublicKey]) -> Self {
            self.authors = authors.to_vec();
            self
        }
        fn npub(mut self, target: PublicKey) -> Self {
            self.target = target;
            self
        }
        fn no_mention_only(mut self) -> Self {
            self.mention_only = false;
            self
        }
        fn channel_only(mut self) -> Self {
            self.watching_mentions = false;
            self
        }
        fn all(mut self) -> Self {
            self.match_mode = MatchMode::All;
            self
        }
        fn build(&self) -> WatchFilter {
            build_watch_filter(
                self.own,
                self.target,
                &self.kinds,
                self.mention_only,
                self.watching_mentions,
                &self.keywords,
                &self.authors,
                self.match_mode,
                self.only_follows,
            )
        }
    }

    fn note_from(keys: &Keys, content: &str, tags: Vec<Tag>) -> Event {
        EventBuilder::text_note(content)
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign")
    }

    fn note(content: &str, tags: Vec<Tag>) -> Event {
        note_from(&Keys::generate(), content, tags)
    }

    fn event_of_kind(kind: u16, content: &str, tags: Vec<Tag>) -> Event {
        EventBuilder::new(Kind::from(kind), content)
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .expect("sign")
    }

    fn reaction_to(target: &PublicKey) -> Event {
        EventBuilder::new(Kind::Reaction, "+")
            .tags(vec![Tag::public_key(*target)])
            .sign_with_keys(&Keys::generate())
            .expect("sign")
    }

    fn p_tag_targets(filter: &Filter) -> Option<Vec<String>> {
        filter
            .generic_tags
            .get(&SingleLetterTag::lowercase(Alphabet::P))
            .map(|v| v.iter().cloned().collect())
    }

    fn has_p_tag(filter: &Filter) -> bool {
        p_tag_targets(filter).is_some()
    }

    fn kinds_of(filter: &Filter) -> Vec<u16> {
        let mut kinds: Vec<u16> = filter
            .kinds
            .as_ref()
            .expect("kinds")
            .iter()
            .map(|k| k.as_u16())
            .collect();
        kinds.sort_unstable();
        kinds
    }

    // --- kinds ---------------------------------------------------------------

    #[test]
    fn default_kinds_cover_notes_and_reactions() {
        let me = Keys::generate().public_key();
        let filter = Args::new(me).build();
        assert_eq!(filter.kinds, vec![Kind::TextNote, Kind::Reaction]);
        // ...but keywords are only matched against kind:1 by default: the content of a
        // reaction is an emoji.
        assert_eq!(filter.keyword_kinds, vec![Kind::TextNote]);

        let filter = Args::new(me).kinds(&[1, 9735]).build();
        assert_eq!(filter.kinds, vec![Kind::TextNote, Kind::from(9735u16)]);
        assert_eq!(filter.keyword_kinds, filter.kinds);
    }

    #[test]
    fn keyword_subscription_follows_the_requested_kinds() {
        // Regression: the keyword subscription used to be hard-coded to kind:1, so
        // `--kind 30023 --keyword foo` subscribed to the wrong kind entirely — no
        // long-form article could ever match, while unrelated kind:1 traffic poured in.
        let me = Keys::generate().public_key();
        let filter = Args::new(me)
            .kinds(&[30023])
            .keywords(&["foo"])
            .no_mention_only()
            .build();

        let subs = filter.subscriptions(Timestamp::now());
        assert_eq!(subs.len(), 1);
        assert_eq!(kinds_of(&subs[0]), vec![30023]);

        let article = event_of_kind(30023, "an article about foo", vec![]);
        assert_eq!(
            filter.match_event(&article),
            Some(MatchReason::Keyword("foo".to_string()))
        );
        // A kind:1 note is not what was asked for, even if it contains the keyword.
        assert!(filter
            .match_event(&note("a note about foo", vec![]))
            .is_none());
    }

    #[test]
    fn default_keyword_matching_ignores_reaction_content() {
        let me = Keys::generate().public_key();
        let filter = Args::new(me).keywords(&["foo"]).build();
        let subs = filter.subscriptions(Timestamp::now());
        assert_eq!(
            kinds_of(&subs[1]),
            vec![1],
            "keyword subscription is kind:1"
        );

        let reaction = EventBuilder::new(Kind::Reaction, "foo")
            .sign_with_keys(&Keys::generate())
            .expect("sign");
        assert!(filter.match_event(&reaction).is_none());
    }

    // --- p tag ---------------------------------------------------------------

    #[test]
    fn p_tag_reply_matches_without_any_hint_in_content() {
        let me = Keys::generate().public_key();
        // An e/p-tag-only reply: nothing in the content names the target.
        let event = note("thanks!", vec![Tag::public_key(me)]);
        assert_eq!(
            Args::new(me).build().match_event(&event),
            Some(MatchReason::Mention)
        );
    }

    #[test]
    fn reaction_targeting_us_matches() {
        let me = Keys::generate().public_key();
        let event = reaction_to(&me);
        assert_eq!(event.kind, Kind::Reaction);
        assert_eq!(
            Args::new(me).build().match_event(&event),
            Some(MatchReason::Mention)
        );
    }

    #[test]
    fn events_for_someone_else_do_not_match() {
        let me = Keys::generate().public_key();
        let other = Keys::generate().public_key();
        let filter = Args::new(me).build();
        assert!(filter
            .match_event(&note("hi", vec![Tag::public_key(other)]))
            .is_none());
        assert!(filter.match_event(&reaction_to(&other)).is_none());
        assert!(filter.match_event(&note("hi", vec![])).is_none());
    }

    #[test]
    fn npub_switches_the_watched_pubkey_everywhere() {
        let me = Keys::generate().public_key();
        let watched = Keys::generate().public_key();
        let filter = Args::new(me).npub(watched).build();

        // The subscription must ask the relay for the *watched* pubkey, not ours.
        let subs = filter.subscriptions(Timestamp::now());
        assert_eq!(subs.len(), 1);
        assert_eq!(p_tag_targets(&subs[0]), Some(vec![watched.to_hex()]));

        // ...and so must the local verdict.
        assert_eq!(
            filter.match_event(&note("hi", vec![Tag::public_key(watched)])),
            Some(MatchReason::Mention)
        );
        assert!(filter
            .match_event(&note("hi", vec![Tag::public_key(me)]))
            .is_none());
    }

    // --- our own events -------------------------------------------------------

    #[test]
    fn our_own_events_are_not_echoed_back() {
        let me = Keys::generate();
        let filter = Args::new(me.public_key()).keywords(&["foo"]).build();
        // Self-addressed or keyword-matching, it is still our own post.
        assert!(filter
            .match_event(&note_from(
                &me,
                "foo",
                vec![Tag::public_key(me.public_key())]
            ))
            .is_none());
    }

    #[test]
    fn watching_yourself_on_purpose_still_works() {
        let me = Keys::generate();
        let filter = Args::new(me.public_key())
            .kinds(&[1])
            .authors(&[me.public_key()])
            .build();
        assert_eq!(
            filter.match_event(&note_from(&me, "my own post", vec![])),
            Some(MatchReason::Author)
        );
    }

    // --- keyword --------------------------------------------------------------

    #[test]
    fn keyword_matches_without_a_p_tag() {
        let me = Keys::generate().public_key();
        let filter = Args::new(me).keywords(&["nostr"]).build();
        assert_eq!(
            filter.match_event(&note("I love Nostr", vec![])),
            Some(MatchReason::Keyword("nostr".to_string()))
        );
        assert!(filter.match_event(&note("I love coffee", vec![])).is_none());
    }

    #[test]
    fn keyword_and_p_tag_are_both_matched_by_one_filter() {
        // The regression that started this: keyword watching must not cost us p-tag
        // replies, and vice versa.
        let me = Keys::generate().public_key();
        let filter = Args::new(me).keywords(&["nostr"]).build();
        assert_eq!(
            filter.match_event(&note("no keyword here", vec![Tag::public_key(me)])),
            Some(MatchReason::Mention)
        );
        assert_eq!(
            filter.match_event(&note("about nostr", vec![])),
            Some(MatchReason::Keyword("nostr".to_string()))
        );
    }

    #[test]
    fn keyword_matching_is_case_insensitive() {
        assert_eq!(
            matched_keyword("Hello NOSTR world", &["nostr".to_string()]),
            Some("nostr".to_string())
        );
        assert_eq!(matched_keyword("hello", &["nostr".to_string()]), None);
        assert_eq!(matched_keyword("hello", &[]), None);
    }

    // --- author ---------------------------------------------------------------

    #[test]
    fn author_matches_plain_posts_with_no_p_tag() {
        // `--author=X --kind=1`, what OpenCrab sends: every post by X must come through.
        let me = Keys::generate().public_key();
        let watched = Keys::generate();
        let filter = Args::new(me)
            .kinds(&[1])
            .authors(&[watched.public_key()])
            .build();
        assert_eq!(
            filter.match_event(&note_from(&watched, "just a regular post", vec![])),
            Some(MatchReason::Author)
        );
        assert!(filter
            .match_event(&note("post by a stranger", vec![]))
            .is_none());
    }

    #[test]
    fn author_watching_keeps_mentions_and_keywords_too() {
        let me = Keys::generate().public_key();
        let watched = Keys::generate();
        let filter = Args::new(me)
            .kinds(&[1])
            .keywords(&["nostr"])
            .authors(&[watched.public_key()])
            .build();
        assert_eq!(
            filter.match_event(&note_from(&watched, "anything", vec![])),
            Some(MatchReason::Author)
        );
        assert_eq!(
            filter.match_event(&note("hey", vec![Tag::public_key(me)])),
            Some(MatchReason::Mention)
        );
        assert_eq!(
            filter.match_event(&note("about nostr", vec![])),
            Some(MatchReason::Keyword("nostr".to_string()))
        );
        assert!(filter.match_event(&note("unrelated", vec![])).is_none());
    }

    #[test]
    fn author_allowed_is_a_plain_membership_test_for_channel_messages() {
        // NIP-28 channel messages skip the mention/keyword conditions but must still
        // honour --author, as they did before the filter was unified.
        let me = Keys::generate().public_key();
        let watched = Keys::generate();
        let filter = Args::new(me)
            .channel_only()
            .authors(&[watched.public_key()])
            .build();
        let mine = event_of_kind(42, "hello channel", vec![]);
        assert!(!filter.author_allowed(&mine));
        assert!(filter.author_allowed(&note_from(&watched, "hello channel", vec![])));

        // With no --author every channel member is allowed.
        let open = Args::new(me).channel_only().build();
        assert!(open.author_allowed(&mine));
    }

    // --- no narrowing at all ---------------------------------------------------

    #[test]
    fn no_condition_at_all_matches_everything() {
        // `--kind 1 --no-mention-only` with nothing else: explicitly asking for all of it.
        let me = Keys::generate().public_key();
        let filter = Args::new(me).kinds(&[1]).no_mention_only().build();
        assert!(!filter.is_narrowed());
        assert_eq!(
            filter.match_event(&note("anything at all", vec![])),
            Some(MatchReason::Unfiltered)
        );
    }

    #[test]
    fn no_mention_only_still_honours_keywords() {
        // `--kind 1 --no-mention-only --keyword foo` stays keyword-filtered instead of
        // degrading into a kind:1 firehose.
        let me = Keys::generate().public_key();
        let filter = Args::new(me)
            .kinds(&[1])
            .no_mention_only()
            .keywords(&["foo"])
            .build();
        assert!(filter.is_narrowed());
        assert_eq!(
            filter.match_event(&note("has foo inside", vec![])),
            Some(MatchReason::Keyword("foo".to_string()))
        );
        assert!(filter.match_event(&note("nothing here", vec![])).is_none());
        // p tags no longer count once --no-mention-only is given.
        assert!(filter
            .match_event(&note("nothing here", vec![Tag::public_key(me)]))
            .is_none());
    }

    // --- --match any / all ------------------------------------------------------

    #[test]
    fn default_match_mode_is_any() {
        assert_eq!(MatchMode::default(), MatchMode::Any);
        let me = Keys::generate().public_key();
        assert_eq!(Args::new(me).build().match_mode, MatchMode::Any);
    }

    #[test]
    fn match_all_requires_author_and_keyword_together() {
        let me = Keys::generate().public_key();
        let watched = Keys::generate();
        let filter = Args::new(me)
            .kinds(&[1])
            .no_mention_only()
            .keywords(&["nostr"])
            .authors(&[watched.public_key()])
            .all()
            .build();

        assert_eq!(
            filter.match_event(&note_from(&watched, "about nostr", vec![])),
            Some(MatchReason::Keyword("nostr".to_string()))
        );
        // Right author, wrong content.
        assert!(filter
            .match_event(&note_from(&watched, "about lunch", vec![]))
            .is_none());
        // Right content, wrong author.
        assert!(filter.match_event(&note("about nostr", vec![])).is_none());
    }

    #[test]
    fn match_all_requires_mention_and_keyword_together() {
        let me = Keys::generate().public_key();
        let filter = Args::new(me).keywords(&["nostr"]).all().build();

        assert_eq!(
            filter.match_event(&note("about nostr", vec![Tag::public_key(me)])),
            Some(MatchReason::Mention)
        );
        assert!(filter
            .match_event(&note("about lunch", vec![Tag::public_key(me)]))
            .is_none());
        assert!(filter.match_event(&note("about nostr", vec![])).is_none());
    }

    #[test]
    fn match_all_makes_author_an_exclusive_scope() {
        let me = Keys::generate().public_key();
        let watched = Keys::generate();
        let filter = Args::new(me)
            .kinds(&[1])
            .authors(&[watched.public_key()])
            .all()
            .build();

        assert_eq!(
            filter.match_event(&note_from(&watched, "hi", vec![Tag::public_key(me)])),
            Some(MatchReason::Mention)
        );
        assert!(filter
            .match_event(&note("hi", vec![Tag::public_key(me)]))
            .is_none());
        assert!(filter
            .match_event(&note_from(&watched, "not addressed to me", vec![]))
            .is_none());
    }

    #[test]
    fn a_single_condition_behaves_the_same_in_both_modes() {
        let me = Keys::generate().public_key();
        let any = Args::new(me).build();
        let all = Args::new(me).all().build();
        for event in [note("hi", vec![Tag::public_key(me)]), note("hi", vec![])] {
            assert_eq!(any.match_event(&event), all.match_event(&event));
        }
    }

    #[test]
    fn no_condition_passes_everything_in_both_modes() {
        let me = Keys::generate().public_key();
        for args in [
            Args::new(me).kinds(&[1]).no_mention_only(),
            Args::new(me).kinds(&[1]).no_mention_only().all(),
        ] {
            let filter = args.build();
            assert_eq!(
                filter.match_event(&note("anything at all", vec![])),
                Some(MatchReason::Unfiltered)
            );
            let subs = filter.subscriptions(Timestamp::now());
            assert_eq!(subs.len(), 1);
            assert!(!has_p_tag(&subs[0]));
            assert_eq!(subs[0].authors, None);
        }
    }

    // --- subscriptions ----------------------------------------------------------

    #[test]
    fn one_subscription_per_active_condition() {
        let me = Keys::generate().public_key();
        let author = Keys::generate().public_key();
        let now = Timestamp::now();

        let subs = Args::new(me).build().subscriptions(now);
        assert_eq!(subs.len(), 1);
        assert_eq!(p_tag_targets(&subs[0]), Some(vec![me.to_hex()]));

        let subs = Args::new(me).keywords(&["foo"]).build().subscriptions(now);
        assert_eq!(subs.len(), 2);
        assert!(has_p_tag(&subs[0]));
        assert!(!has_p_tag(&subs[1]));
        assert_eq!(kinds_of(&subs[1]), vec![1]);

        // The author subscription must not be p-tag narrowed, or "follow everything X
        // posts" collapses to "X's replies to me".
        let subs = Args::new(me)
            .kinds(&[1])
            .authors(&[author])
            .build()
            .subscriptions(now);
        assert_eq!(subs.len(), 2);
        assert!(has_p_tag(&subs[0]));
        assert!(!has_p_tag(&subs[1]));
        assert_eq!(subs[1].authors, Some([author].into_iter().collect()));
    }

    #[test]
    fn without_keywords_no_bare_subscription_is_opened() {
        let me = Keys::generate().public_key();
        let subs = Args::new(me).build().subscriptions(Timestamp::now());
        assert!(
            subs.iter().all(has_p_tag),
            "an un-narrowed subscription would be a firehose"
        );
    }

    #[test]
    fn match_all_collapses_the_subscriptions_into_one() {
        let me = Keys::generate().public_key();
        let author = Keys::generate().public_key();
        let now = Timestamp::now();
        let args = Args::new(me)
            .kinds(&[1])
            .keywords(&["foo"])
            .authors(&[author]);

        let subs = args.all().build().subscriptions(now);
        assert_eq!(subs.len(), 1, "AND composes into a single relay filter");
        assert_eq!(p_tag_targets(&subs[0]), Some(vec![me.to_hex()]));
        assert_eq!(subs[0].authors, Some([author].into_iter().collect()));
        assert_eq!(kinds_of(&subs[0]), vec![1]);

        // ...whereas the same conditions with --match any need one subscription each.
        let any = Args::new(me)
            .kinds(&[1])
            .keywords(&["foo"])
            .authors(&[author])
            .build();
        assert_eq!(any.subscriptions(now).len(), 3);
    }

    // --- run() wiring ------------------------------------------------------------

    #[test]
    fn json_mode_always_watches_mentions() {
        // `run` derives this once, before the --json branch, and hands it to the single
        // `build_watch_filter` call both modes share. --json rejects --channel, so the
        // derivation cannot come out false there.
        assert!(watching_mentions(None, None));
        assert!(watching_mentions(None, Some("npub1example")));
    }

    #[test]
    fn channel_alone_watches_no_mentions() {
        assert!(!watching_mentions(Some("cafe"), None));
        // ...but --channel together with --npub does watch mentions.
        assert!(watching_mentions(Some("cafe"), Some("npub1example")));
    }

    #[test]
    fn channel_alone_opens_no_general_subscriptions() {
        // The channel subscription supplies the events; --author is applied locally.
        assert!(!opens_general_subscriptions(false, &[]));
        assert!(!opens_general_subscriptions(
            watching_mentions(Some("cafe"), None),
            &[]
        ));
        // Keywords still need their own subscription even alongside a channel.
        assert!(opens_general_subscriptions(false, &["foo".to_string()]));
        assert!(opens_general_subscriptions(true, &[]));
    }

    /// `--channel` alone watches no mentions, so the p-tag condition is dropped there.
    #[test]
    fn channel_only_run_drops_the_mention_condition() {
        let me = Keys::generate().public_key();
        let filter = Args::new(me).channel_only().keywords(&["foo"]).build();
        assert!(!filter.mention_only);
        assert!(filter
            .match_event(&note("hi", vec![Tag::public_key(me)]))
            .is_none());
        assert_eq!(
            filter.match_event(&note("has foo", vec![])),
            Some(MatchReason::Keyword("foo".to_string()))
        );
    }

    // --- effective mention-only ----------------------------------------------------

    #[test]
    fn no_mention_only_wins_over_the_default() {
        assert!(effective_mention_only(true, false));
        assert!(!effective_mention_only(true, true));
        assert!(!effective_mention_only(false, false));
    }

    // --- webhook labels --------------------------------------------------------------

    #[test]
    fn webhook_labels_follow_the_match_reason() {
        // An author/unfiltered hit is a post we chose to follow, not a mention of us.
        assert_eq!(
            text_note_label(Some(&MatchReason::Author), false),
            ("📝", "投稿")
        );
        assert_eq!(
            text_note_label(Some(&MatchReason::Author), true),
            ("📝", "投稿")
        );
        assert_eq!(
            text_note_label(Some(&MatchReason::Unfiltered), false),
            ("📝", "投稿")
        );
        assert_eq!(
            text_note_label(Some(&MatchReason::Mention), false),
            ("📩", "メンション")
        );
        assert_eq!(
            text_note_label(Some(&MatchReason::Mention), true),
            ("📩", "リプライ")
        );
    }

    // --- --only-follows ----------------------------------------------------------------

    #[test]
    fn only_follows_drops_the_mention_condition() {
        // The p-tag firehose is exactly what --only-follows must not open: anyone can
        // p-tag you. The mention condition is therefore forced off even though it defaults
        // on.
        let me = Keys::generate().public_key();
        let filter = Args::new(me).only_follows().build();
        assert!(!filter.mention_only);
    }

    #[test]
    fn only_follows_gates_hard_on_the_follow_set() {
        // Only followees pass; a non-followee's reply/reaction never does, whatever it
        // matches. This is the whole point of the flag.
        let me = Keys::generate().public_key();
        let friend = Keys::generate();
        let stranger = Keys::generate();

        let mut filter = Args::new(me).kinds(&[1, 7]).only_follows().build();
        filter.set_follows(&[friend.public_key()]);

        assert_eq!(
            filter.match_event(&note_from(&friend, "just a post", vec![])),
            Some(MatchReason::Author)
        );
        // A friend's reply addressed to me also comes through the authors filter.
        assert_eq!(
            filter.match_event(&note_from(&friend, "hi", vec![Tag::public_key(me)])),
            Some(MatchReason::Author)
        );
        // A stranger p-tagging me is dropped, even though the default mode would keep it.
        assert!(filter
            .match_event(&note_from(&stranger, "hey you", vec![Tag::public_key(me)]))
            .is_none());
    }

    #[test]
    fn only_follows_subscription_is_author_narrowed_and_opens_no_firehose() {
        let me = Keys::generate().public_key();
        let friend = Keys::generate().public_key();
        let mut filter = Args::new(me).kinds(&[1, 7]).only_follows().build();
        filter.set_follows(&[friend]);

        let subs = filter.subscriptions(Timestamp::now());
        assert_eq!(subs.len(), 1, "a single follow-narrowed subscription");
        assert!(!has_p_tag(&subs[0]), "no p-tag firehose is opened");
        assert_eq!(subs[0].authors, Some([friend].into_iter().collect()));
        assert_eq!(kinds_of(&subs[0]), vec![1, 7]);
    }

    #[test]
    fn only_follows_merges_static_authors_with_the_follow_list() {
        let me = Keys::generate().public_key();
        let friend = Keys::generate();
        let extra = Keys::generate();
        // --author <extra> alongside --only-follows: the two sources are unioned.
        let mut filter = Args::new(me)
            .kinds(&[1])
            .authors(&[extra.public_key()])
            .only_follows()
            .build();
        filter.set_follows(&[friend.public_key()]);

        assert_eq!(
            filter.match_event(&note_from(&friend, "from a followee", vec![])),
            Some(MatchReason::Author)
        );
        assert_eq!(
            filter.match_event(&note_from(&extra, "from a static author", vec![])),
            Some(MatchReason::Author)
        );
        let subs = filter.subscriptions(Timestamp::now());
        assert_eq!(
            subs[0].authors,
            Some(
                [friend.public_key(), extra.public_key()]
                    .into_iter()
                    .collect()
            )
        );
    }

    #[test]
    fn only_follows_set_follows_dedupes_against_static_authors() {
        let me = Keys::generate().public_key();
        let shared = Keys::generate().public_key();
        let mut filter = Args::new(me)
            .kinds(&[1])
            .authors(&[shared])
            .only_follows()
            .build();
        // The same pubkey appears in both --author and the follow list.
        filter.set_follows(&[shared]);
        assert_eq!(filter.authors, vec![shared], "no duplicate author entries");
    }

    #[test]
    fn only_follows_empty_list_drops_everything_and_opens_nothing() {
        // A cleared follow list must never degrade into a firehose: no subscription is
        // opened and every event is dropped locally.
        let me = Keys::generate().public_key();
        let stranger = Keys::generate();
        let mut filter = Args::new(me).kinds(&[1]).only_follows().build();
        filter.set_follows(&[]);

        assert!(filter.subscriptions(Timestamp::now()).is_empty());
        assert!(filter
            .match_event(&note_from(&stranger, "anything", vec![]))
            .is_none());
    }

    #[test]
    fn only_follows_with_match_all_keeps_keyword_narrowing() {
        // --only-follows --keyword foo --match all: a followee's post that also contains
        // the keyword. The relay filter narrows to the follow set and the keyword kinds;
        // the keyword itself is checked locally.
        let me = Keys::generate().public_key();
        let friend = Keys::generate();
        let mut filter = Args::new(me)
            .kinds(&[1])
            .keywords(&["foo"])
            .only_follows()
            .all()
            .build();
        filter.set_follows(&[friend.public_key()]);

        assert_eq!(
            filter.match_event(&note_from(&friend, "about foo", vec![])),
            Some(MatchReason::Keyword("foo".to_string()))
        );
        // Right author, wrong content.
        assert!(filter
            .match_event(&note_from(&friend, "about bar", vec![]))
            .is_none());
        let subs = filter.subscriptions(Timestamp::now());
        assert_eq!(subs.len(), 1);
        assert!(subs[0].authors.is_some());
    }

    #[test]
    fn contacts_from_event_reads_the_p_tags() {
        let a = Keys::generate().public_key();
        let b = Keys::generate().public_key();
        let list = EventBuilder::new(Kind::ContactList, "")
            .tags(vec![Tag::public_key(a), Tag::public_key(b)])
            .sign_with_keys(&Keys::generate())
            .expect("sign");
        assert_eq!(contacts_from_event(&list), vec![a, b]);
    }
}
