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
/// Built straight from the command-line arguments (`--npub`, `--kind`, `--keyword`,
/// `--author`, `--mention-only`, `--match`) and used by **both** output modes: `--json`
/// and the Discord webhook differ only in where a matched event goes, never in what is
/// matched.
struct WatchFilter {
    /// Pubkey watched via `p` tags — `--npub` or, by default, our own.
    target_pubkey: PublicKey,
    /// Kinds to subscribe to. `--kind` when given, otherwise kind:1 + kind:7 so mentions,
    /// replies and reactions all arrive.
    kinds: Vec<Kind>,
    /// Whether the `p`-tag condition is in play (`--mention-only`, the default).
    mention_only: bool,
    keywords: Vec<String>,
    authors: Vec<PublicKey>,
    /// Whether the conditions above are OR'd (`any`) or AND'd (`all`).
    match_mode: MatchMode,
}

impl WatchFilter {
    fn new(
        target_pubkey: PublicKey,
        extra_kinds: &[u16],
        mention_only: bool,
        keywords: &[String],
        authors: &[PublicKey],
        match_mode: MatchMode,
    ) -> Self {
        let kinds = if extra_kinds.is_empty() {
            vec![Kind::TextNote, Kind::Reaction]
        } else {
            extra_kinds.iter().map(|&k| Kind::from(k)).collect()
        };
        Self {
            target_pubkey,
            kinds,
            mention_only,
            keywords: keywords.to_vec(),
            authors: authors.to_vec(),
            match_mode,
        }
    }

    /// True when at least one narrowing condition (p tag / keyword / author) is set.
    fn is_narrowed(&self) -> bool {
        self.mention_only || !self.keywords.is_empty() || !self.authors.is_empty()
    }

    /// The relay subscriptions needed to see every event `match_event` could keep.
    ///
    /// - `--match any`: one subscription per condition, so the relay delivers the union.
    /// - `--match all`: a single subscription combining the conditions a relay can
    ///   evaluate (kinds + `p` tag + authors). Keywords are never part of it — relays
    ///   cannot filter by content — so they stay a local check in `match_event`.
    ///
    /// With no condition at all we subscribe to the bare kinds, which is what
    /// `--no-mention-only` without any other flag explicitly asks for.
    fn subscriptions(&self, since: Timestamp) -> Vec<Filter> {
        if self.match_mode == MatchMode::All {
            let mut filter = Filter::new().kinds(self.kinds.clone()).since(since);
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
            // Relays cannot filter by content, so keywords need a kind:1 subscription
            // that is matched locally in `match_event`.
            filters.push(Filter::new().kind(Kind::TextNote).since(since));
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
        if let Some(kw) = matched_keyword(&event.content, &self.keywords) {
            return Some(MatchReason::Keyword(kw));
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
            Some(matched_keyword(&event.content, &self.keywords)?)
        };
        if !self.authors.is_empty() && !self.authors.contains(&event.pubkey) {
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
) -> Result<()> {
    if !json_output && webhook_url.is_none() {
        bail!("--webhook is required unless --json is specified");
    }
    if json_output && channel_id.is_some() {
        bail!("--channel is not supported together with --json");
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

    if json_output {
        let watch_filter = WatchFilter::new(
            target_pubkey,
            extra_kinds,
            mention_only,
            keywords,
            &author_pubkeys,
            match_mode,
        );
        return watch_json(&nostr_client, &watch_filter).await;
    }
    let webhook_url = webhook_url.expect("checked above");

    // Channel watch mode
    let watching_channel = channel_id.map(|s| s.to_string());

    if let Some(ref ch_id) = watching_channel {
        println!(
            "Watching NIP-28 channel: {}...",
            &ch_id[..16.min(ch_id.len())]
        );
        println!("Webhook: {}", webhook_url);
        println!("Press Ctrl+C to stop.\n");

        let channel_event_id = EventId::from_hex(ch_id)?;
        let filter = Filter::new()
            .kind(Kind::ChannelMessage)
            .event(channel_event_id)
            .since(Timestamp::now());

        nostr_client.subscribe(filter, None).await?;
    }

    // Mention/reply/reaction watch mode (skip if only channel is specified)
    let watching_mentions = channel_id.is_none() || npub_str.is_some();

    // Exactly the same filter the --json path builds. `--channel` on its own watches no
    // mentions, so the p-tag condition is dropped there; if nothing else is configured we
    // stay on the channel subscription alone instead of opening a general one.
    let watch_filter = if watching_mentions || !keywords.is_empty() || !author_pubkeys.is_empty() {
        Some(WatchFilter::new(
            target_pubkey,
            extra_kinds,
            mention_only && watching_mentions,
            keywords,
            &author_pubkeys,
            match_mode,
        ))
    } else {
        None
    };

    if let Some(ref watch_filter) = watch_filter {
        println!("Webhook: {}", webhook_url);
        describe_filter(watch_filter)?;
        println!("Press Ctrl+C to stop.\n");
        for filter in watch_filter.subscriptions(Timestamp::now()) {
            nostr_client.subscribe(filter, None).await?;
        }
    }

    let mut profile_cache: HashMap<PublicKey, (String, Option<String>)> = HashMap::new();
    let http_client = reqwest::Client::new();
    let mut dedup = EventDeduplicator::new();

    let mut notifications = nostr_client.notifications();
    while let Ok(notification) = notifications.recv().await {
        if let RelayPoolNotification::Event { event, .. } = notification {
            if !dedup.accept(&event) {
                continue;
            }

            if event.pubkey == own_pubkey && event.kind != Kind::ChannelMessage {
                continue;
            }

            // Channel messages are selected by the channel subscription below; everything
            // else goes through the shared filter, exactly as in --json mode.
            let reason = if event.kind == Kind::ChannelMessage && watching_channel.is_some() {
                None
            } else {
                match watch_filter.as_ref().and_then(|f| f.match_event(&event)) {
                    Some(reason) => Some(reason),
                    None => continue,
                }
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
                    _ => {
                        let has_e_tag = event.tags.iter().any(|t| {
                            matches!(t.as_standardized(), Some(TagStandard::Event { .. }))
                        });
                        let label = if has_e_tag {
                            "リプライ"
                        } else {
                            "メンション"
                        };
                        format!(
                            "📩 **{}** from {}\n> {}\n🔗 {}",
                            label, sender_name, event.content, note_id
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
    if !filter.authors.is_empty() {
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
async fn watch_json(nostr_client: &Client, watch_filter: &WatchFilter) -> Result<()> {
    eprintln!("Watching (JSON mode)");
    for line in filter_description(watch_filter)? {
        eprintln!("{}", line);
    }
    eprintln!("Press Ctrl+C to stop.\n");

    for filter in watch_filter.subscriptions(Timestamp::now()) {
        nostr_client.subscribe(filter, None).await?;
    }

    let mut dedup = EventDeduplicator::new();
    let mut author_name_cache: HashMap<PublicKey, Option<String>> = HashMap::new();

    let mut notifications = nostr_client.notifications();
    while let Ok(notification) = notifications.recv().await {
        if let RelayPoolNotification::Event { event, .. } = notification {
            if !dedup.accept(&event) {
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

    fn note_from(keys: &Keys, content: &str, tags: Vec<Tag>) -> Event {
        EventBuilder::text_note(content)
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign")
    }

    fn note(content: &str, tags: Vec<Tag>) -> Event {
        note_from(&Keys::generate(), content, tags)
    }

    fn reaction_to(target: &PublicKey) -> Event {
        EventBuilder::new(Kind::Reaction, "+")
            .tags(vec![Tag::public_key(*target)])
            .sign_with_keys(&Keys::generate())
            .expect("sign")
    }

    fn has_p_tag(filter: &Filter) -> bool {
        filter
            .generic_tags
            .contains_key(&SingleLetterTag::lowercase(Alphabet::P))
    }

    /// Default `watch` invocation: no --kind, no --keyword, no --author.
    fn mention_filter(target: PublicKey) -> WatchFilter {
        WatchFilter::new(target, &[], true, &[], &[], MatchMode::Any)
    }

    // --- kinds ---------------------------------------------------------------

    #[test]
    fn default_kinds_cover_notes_and_reactions() {
        let me = Keys::generate().public_key();
        assert_eq!(
            mention_filter(me).kinds,
            vec![Kind::TextNote, Kind::Reaction]
        );
        // --kind wins when given.
        assert_eq!(
            WatchFilter::new(me, &[1, 9735], true, &[], &[], MatchMode::Any).kinds,
            vec![Kind::TextNote, Kind::from(9735u16)]
        );
    }

    // --- p tag ---------------------------------------------------------------

    #[test]
    fn p_tag_reply_matches_without_any_hint_in_content() {
        let me = Keys::generate().public_key();
        // An e/p-tag-only reply: nothing in the content names the target.
        let event = note("thanks!", vec![Tag::public_key(me)]);
        assert_eq!(
            mention_filter(me).match_event(&event),
            Some(MatchReason::Mention)
        );
    }

    #[test]
    fn reaction_targeting_us_matches() {
        let me = Keys::generate().public_key();
        let event = reaction_to(&me);
        assert_eq!(event.kind, Kind::Reaction);
        assert_eq!(
            mention_filter(me).match_event(&event),
            Some(MatchReason::Mention)
        );
    }

    #[test]
    fn events_for_someone_else_do_not_match() {
        let me = Keys::generate().public_key();
        let other = Keys::generate().public_key();
        assert!(mention_filter(me)
            .match_event(&note("hi", vec![Tag::public_key(other)]))
            .is_none());
        assert!(mention_filter(me)
            .match_event(&reaction_to(&other))
            .is_none());
        assert!(mention_filter(me)
            .match_event(&note("hi", vec![]))
            .is_none());
    }

    // --- keyword --------------------------------------------------------------

    #[test]
    fn keyword_matches_without_a_p_tag() {
        let me = Keys::generate().public_key();
        let filter = WatchFilter::new(me, &[], true, &["nostr".to_string()], &[], MatchMode::Any);
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
        let filter = WatchFilter::new(me, &[], true, &["nostr".to_string()], &[], MatchMode::Any);
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
        assert_eq!(
            matched_keyword("hello nostr", &["NOSTR".to_string()]),
            Some("NOSTR".to_string())
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
        let filter = WatchFilter::new(me, &[1], true, &[], &[watched.public_key()], MatchMode::Any);
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
        let filter = WatchFilter::new(
            me,
            &[1],
            true,
            &["nostr".to_string()],
            &[watched.public_key()],
            MatchMode::Any,
        );
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

    // --- no narrowing at all ---------------------------------------------------

    #[test]
    fn no_condition_at_all_matches_everything() {
        // `--kind 1 --no-mention-only` with nothing else: explicitly asking for all of it.
        let me = Keys::generate().public_key();
        let filter = WatchFilter::new(me, &[1], false, &[], &[], MatchMode::Any);
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
        let filter = WatchFilter::new(me, &[1], false, &["foo".to_string()], &[], MatchMode::Any);
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

    // --- subscriptions ----------------------------------------------------------

    #[test]
    fn one_subscription_per_active_condition() {
        let me = Keys::generate().public_key();
        let author = Keys::generate().public_key();
        let now = Timestamp::now();

        // p tag only.
        let subs = mention_filter(me).subscriptions(now);
        assert_eq!(subs.len(), 1);
        assert!(has_p_tag(&subs[0]));

        // p tag + keyword: the keyword one is an un-narrowed kind:1, matched locally.
        let subs = WatchFilter::new(me, &[], true, &["foo".to_string()], &[], MatchMode::Any)
            .subscriptions(now);
        assert_eq!(subs.len(), 2);
        assert!(has_p_tag(&subs[0]));
        assert!(!has_p_tag(&subs[1]));
        assert_eq!(subs[1].kinds, Some([Kind::TextNote].into_iter().collect()));

        // p tag + author: the author subscription must not be p-tag narrowed, or
        // "follow everything X posts" collapses to "X's replies to me".
        let subs =
            WatchFilter::new(me, &[1], true, &[], &[author], MatchMode::Any).subscriptions(now);
        assert_eq!(subs.len(), 2);
        assert!(has_p_tag(&subs[0]));
        assert!(!has_p_tag(&subs[1]));
        assert_eq!(subs[1].authors, Some([author].into_iter().collect()));
    }

    #[test]
    fn without_keywords_no_bare_kind1_subscription_is_opened() {
        let me = Keys::generate().public_key();
        let subs = mention_filter(me).subscriptions(Timestamp::now());
        assert!(
            subs.iter().all(has_p_tag),
            "a kind:1 subscription with no narrowing would be a firehose"
        );
    }

    #[test]
    fn unnarrowed_filter_subscribes_to_the_bare_kinds() {
        let me = Keys::generate().public_key();
        let subs = WatchFilter::new(me, &[1], false, &[], &[], MatchMode::Any)
            .subscriptions(Timestamp::now());
        assert_eq!(subs.len(), 1);
        assert!(!has_p_tag(&subs[0]));
        assert_eq!(subs[0].authors, None);
    }

    // --- both output modes share the filter --------------------------------------

    #[test]
    fn webhook_and_json_modes_build_identical_filters() {
        // `run` builds the WatchFilter from the same arguments for both modes; the only
        // difference is where a matched event is written. Guard that the inputs a webhook
        // run uses (watching_mentions = true) produce the same subscriptions and verdicts.
        let me = Keys::generate().public_key();
        let author = Keys::generate();
        let args_kinds = [1u16];
        let keywords = ["foo".to_string()];
        let authors = [author.public_key()];

        // Both modes pass mention_only through unchanged (the webhook path additionally
        // ANDs in `watching_mentions`, which is true whenever --channel is not the only
        // thing being watched).
        let mention_only = true;
        let watching_mentions = true;
        let json_filter = WatchFilter::new(
            me,
            &args_kinds,
            mention_only,
            &keywords,
            &authors,
            MatchMode::Any,
        );
        let webhook_filter = WatchFilter::new(
            me,
            &args_kinds,
            mention_only && watching_mentions,
            &keywords,
            &authors,
            MatchMode::Any,
        );

        let now = Timestamp::now();
        assert_eq!(
            json_filter.subscriptions(now).len(),
            webhook_filter.subscriptions(now).len()
        );
        for event in [
            note("reply", vec![Tag::public_key(me)]),
            note("has foo", vec![]),
            note_from(&author, "by watched author", vec![]),
            note("nothing", vec![]),
        ] {
            assert_eq!(
                json_filter.match_event(&event),
                webhook_filter.match_event(&event)
            );
        }
    }

    // --- --match any / all ------------------------------------------------------

    #[test]
    fn default_match_mode_is_any() {
        // Nothing on the command line means OR, so nothing is lost by accident.
        assert_eq!(MatchMode::default(), MatchMode::Any);
        let me = Keys::generate().public_key();
        assert_eq!(mention_filter(me).match_mode, MatchMode::Any);
    }

    #[test]
    fn match_all_requires_author_and_keyword_together() {
        let me = Keys::generate().public_key();
        let watched = Keys::generate();
        let filter = WatchFilter::new(
            me,
            &[1],
            false,
            &["nostr".to_string()],
            &[watched.public_key()],
            MatchMode::All,
        );

        // Both conditions hold.
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
        let filter = WatchFilter::new(me, &[], true, &["nostr".to_string()], &[], MatchMode::All);

        assert_eq!(
            filter.match_event(&note("about nostr", vec![Tag::public_key(me)])),
            Some(MatchReason::Mention)
        );
        // Addressed to us but off-topic.
        assert!(filter
            .match_event(&note("about lunch", vec![Tag::public_key(me)]))
            .is_none());
        // On-topic but not addressed to us.
        assert!(filter.match_event(&note("about nostr", vec![])).is_none());
    }

    #[test]
    fn match_all_makes_author_an_exclusive_scope() {
        // The counterpart to `author_watching_keeps_mentions_and_keywords_too`: with
        // --match all, mentions from anyone else no longer come through.
        let me = Keys::generate().public_key();
        let watched = Keys::generate();
        let filter = WatchFilter::new(me, &[1], true, &[], &[watched.public_key()], MatchMode::All);

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
        let any = WatchFilter::new(me, &[], true, &[], &[], MatchMode::Any);
        let all = WatchFilter::new(me, &[], true, &[], &[], MatchMode::All);
        for event in [note("hi", vec![Tag::public_key(me)]), note("hi", vec![])] {
            assert_eq!(any.match_event(&event), all.match_event(&event));
        }
    }

    #[test]
    fn no_condition_passes_everything_in_both_modes() {
        let me = Keys::generate().public_key();
        for mode in [MatchMode::Any, MatchMode::All] {
            let filter = WatchFilter::new(me, &[1], false, &[], &[], mode);
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

    #[test]
    fn match_all_collapses_the_subscriptions_into_one() {
        let me = Keys::generate().public_key();
        let author = Keys::generate().public_key();
        let now = Timestamp::now();
        let filter = WatchFilter::new(
            me,
            &[1],
            true,
            &["foo".to_string()],
            &[author],
            MatchMode::All,
        );

        let subs = filter.subscriptions(now);
        assert_eq!(subs.len(), 1, "AND composes into a single relay filter");
        // Everything the relay can evaluate is on that one filter...
        assert!(has_p_tag(&subs[0]));
        assert_eq!(subs[0].authors, Some([author].into_iter().collect()));
        assert_eq!(subs[0].kinds, Some([Kind::TextNote].into_iter().collect()));

        // ...whereas the same conditions with --match any need one subscription each.
        let any = WatchFilter::new(
            me,
            &[1],
            true,
            &["foo".to_string()],
            &[author],
            MatchMode::Any,
        );
        assert_eq!(any.subscriptions(now).len(), 3);
    }

    /// `--channel` alone watches no mentions, so the p-tag condition is dropped there.
    #[test]
    fn channel_only_run_drops_the_mention_condition() {
        let me = Keys::generate().public_key();
        // `mention_only && watching_mentions` with watching_mentions = false.
        let filter = WatchFilter::new(me, &[], false, &["foo".to_string()], &[], MatchMode::Any);
        assert!(!filter.mention_only);
        assert!(filter
            .match_event(&note("hi", vec![Tag::public_key(me)]))
            .is_none());
        assert_eq!(
            filter.match_event(&note("has foo", vec![])),
            Some(MatchReason::Keyword("foo".to_string()))
        );
    }
}
