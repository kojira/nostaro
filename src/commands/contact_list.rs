//! Follow-list files: decide the new kind:3 locally, then publish it **once**.
//!
//! `nostaro following --out following.json --out-format json` saves
//! `{"count":N,"users":[{"npub","hex"}]}`; `--out following.txt` saves one npub per
//! line. Both are accepted here, so a saved listing can be applied as-is:
//!
//! ```text
//! nostaro follow   --from-file following.json             # add the set
//! nostaro follow   --from-file following.json --replace   # make it the whole set
//! nostaro unfollow --from-file following.json             # remove the set
//! nostaro unfollow --from-file following.json --all       # unfollow everyone
//! ```
//!
//! Applying N entries must never mean N publishes. `nostaro follow <npub>` fetches the
//! contact list, adds one key, and republishes the whole kind:3 — correct for one call,
//! but looping it over a 100-entry file puts 100 kind:3 events on the relays within a few
//! seconds, which is indistinguishable from spam. Here the set is computed from one read
//! and published as one event, whatever the list size.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{bail, Context, Result};
use nostr_sdk::prelude::*;

use crate::utils::resolve_pubkey;

/// What a from-file command does with the follow set that is already published.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ListMode {
    /// Keep everyone already followed and add the file's set (the default for `follow`).
    Add,
    /// Publish exactly the file's set (`follow --from-file --replace`).
    Replace,
    /// Drop the file's set (the default for `unfollow`).
    Remove,
    /// Drop everyone (`unfollow --from-file --all`).
    RemoveAll,
}

/// The kind:3 a from-file command publishes, and what it changes.
#[derive(Debug)]
pub struct ContactListPlan {
    /// The contact list to publish, in publish order.
    pub contacts: Vec<PublicKey>,
    /// Newly followed by this plan (present in `contacts`).
    pub added: Vec<PublicKey>,
    /// No longer followed by this plan (absent from `contacts`).
    pub removed: Vec<PublicKey>,
}

impl ContactListPlan {
    /// Nothing to publish when the set would not change. Re-running the same file must
    /// cost no event: it changes nothing, and a second kind:3 is exactly the traffic this
    /// command exists to avoid.
    pub fn is_noop(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// Read a saved listing file and resolve every entry to a pubkey.
///
/// Parsing happens before any relay is contacted (`create_client`), so a file with a bad
/// entry cannot publish a partial list: [`read_listing`] fails and the caller never
/// starts a client.
pub fn read_listing(path: &Path) -> Result<Vec<PublicKey>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read the follow-list file {}", path.display()))?;
    parse_listing(&raw).with_context(|| format!("in the follow-list file {}", path.display()))
}

/// Parse a saved listing: the `--out-format json` document, a bare JSON array, or the
/// text listing (one npub/hex per line). Duplicate entries collapse to the first
/// occurrence, keeping the file's order.
pub fn parse_listing(raw: &str) -> Result<Vec<PublicKey>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("the follow-list file is empty (expected a saved `nostaro following` listing)");
    }
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return parse_json_listing(trimmed);
    }
    parse_text_listing(trimmed)
}

fn parse_json_listing(raw: &str) -> Result<Vec<PublicKey>> {
    let shape = "the follow-list file must be {\"count\":N,\"users\":[{\"npub\",\"hex\"}]} \
                 or a JSON array of npub/hex values";
    let value: serde_json::Value =
        serde_json::from_str(raw).context("the follow-list file is not valid JSON")?;
    let items = match &value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(map) => match map.get("users").and_then(|u| u.as_array()) {
            Some(items) => items,
            None => bail!("{shape}"),
        },
        _ => bail!("{shape}"),
    };

    let mut wanted = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let entry = match item {
            serde_json::Value::String(entry) => entry.as_str(),
            serde_json::Value::Object(map) => map
                .get("npub")
                .or_else(|| map.get("hex"))
                .and_then(|entry| entry.as_str())
                .ok_or_else(|| anyhow::anyhow!("entry {index} has neither \"npub\" nor \"hex\""))?,
            other => bail!("entry {index} is not a pubkey: {other}"),
        };
        wanted.push(resolve_entry(index, entry)?);
    }
    Ok(collapse(wanted))
}

fn parse_text_listing(raw: &str) -> Result<Vec<PublicKey>> {
    let mut wanted = Vec::new();
    for (index, line) in raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .enumerate()
    {
        wanted.push(resolve_entry(index, line)?);
    }
    if wanted.is_empty() {
        bail!("the follow-list file has no entries");
    }
    Ok(collapse(wanted))
}

fn resolve_entry(index: usize, entry: &str) -> Result<PublicKey> {
    resolve_pubkey(entry)
        .map_err(|_| anyhow::anyhow!("entry {index} is not a valid pubkey: {entry:?}"))
}

/// First occurrence wins, so `Add` appends in the file's order.
fn collapse(pubkeys: Vec<PublicKey>) -> Vec<PublicKey> {
    let mut seen = HashSet::with_capacity(pubkeys.len());
    pubkeys
        .into_iter()
        .filter(|pubkey| seen.insert(*pubkey))
        .collect()
}

/// Compute the kind:3 to publish for `mode`.
pub fn plan(current: &[PublicKey], wanted: &[PublicKey], mode: ListMode) -> ContactListPlan {
    let current = collapse(current.to_vec());
    let wanted = collapse(wanted.to_vec());
    let followed: HashSet<PublicKey> = current.iter().copied().collect();

    match mode {
        ListMode::Add => {
            let added: Vec<PublicKey> = wanted
                .iter()
                .copied()
                .filter(|pubkey| !followed.contains(pubkey))
                .collect();
            let mut contacts = current;
            contacts.extend(added.iter().copied());
            ContactListPlan {
                contacts,
                added,
                removed: Vec::new(),
            }
        }
        ListMode::Replace => {
            let wanted_set: HashSet<PublicKey> = wanted.iter().copied().collect();
            ContactListPlan {
                added: wanted
                    .iter()
                    .copied()
                    .filter(|pubkey| !followed.contains(pubkey))
                    .collect(),
                removed: current
                    .into_iter()
                    .filter(|pubkey| !wanted_set.contains(pubkey))
                    .collect(),
                contacts: wanted,
            }
        }
        ListMode::Remove => {
            let dropped: HashSet<PublicKey> = wanted.iter().copied().collect();
            ContactListPlan {
                removed: current
                    .iter()
                    .copied()
                    .filter(|pubkey| dropped.contains(pubkey))
                    .collect(),
                contacts: current
                    .into_iter()
                    .filter(|pubkey| !dropped.contains(pubkey))
                    .collect(),
                added: Vec::new(),
            }
        }
        ListMode::RemoveAll => ContactListPlan {
            contacts: Vec::new(),
            added: Vec::new(),
            removed: current,
        },
    }
}

/// Publish `plan.contacts`, at most once, and report whether an event went out.
///
/// Taking the publish as an argument is what makes "one publish for the whole file" a
/// testable property: the test counts calls instead of needing a relay.
pub async fn publish_plan<P, F>(plan: &ContactListPlan, publish: P) -> Result<bool>
where
    P: FnOnce(Vec<PublicKey>) -> F,
    F: std::future::Future<Output = Result<()>>,
{
    if plan.is_noop() {
        return Ok(false);
    }
    publish(plan.contacts.clone()).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(count: usize) -> Vec<PublicKey> {
        (0..count).map(|_| Keys::generate().public_key()).collect()
    }

    fn json_document(pubkeys: &[PublicKey]) -> String {
        let users: Vec<serde_json::Value> = pubkeys
            .iter()
            .map(|pubkey| {
                serde_json::json!({
                    "npub": pubkey.to_bech32().unwrap(),
                    "hex": pubkey.to_hex(),
                })
            })
            .collect();
        serde_json::json!({ "count": users.len(), "users": users }).to_string()
    }

    /// The document `nostaro following --out x --out-format json` writes is directly
    /// usable as input, which is the whole point of the file interface.
    #[test]
    fn json_document_from_following_is_accepted() {
        let pubkeys = keys(3);
        let parsed = parse_listing(&json_document(&pubkeys)).unwrap();
        assert_eq!(parsed, pubkeys);
    }

    #[test]
    fn bare_array_of_npub_and_hex_is_accepted() {
        let pubkeys = keys(2);
        let raw = format!(
            "[\"{}\", \"{}\"]",
            pubkeys[0].to_bech32().unwrap(),
            pubkeys[1].to_hex()
        );
        assert_eq!(parse_listing(&raw).unwrap(), pubkeys);
    }

    /// `following --out` without `--out-format json` writes the text listing; a saved
    /// listing is a saved listing either way.
    #[test]
    fn text_listing_is_accepted() {
        let pubkeys = keys(2);
        let raw = format!(
            "  {}\n\n  {}\n",
            pubkeys[0].to_bech32().unwrap(),
            pubkeys[1].to_hex()
        );
        assert_eq!(parse_listing(&raw).unwrap(), pubkeys);
    }

    #[test]
    fn duplicate_entries_collapse_and_keep_file_order() {
        let pubkeys = keys(2);
        let raw = format!(
            "[\"{}\", \"{}\", \"{}\"]",
            pubkeys[1].to_bech32().unwrap(),
            pubkeys[0].to_hex(),
            pubkeys[1].to_hex()
        );
        // File order, first occurrence: the repeated npub does not move or duplicate.
        assert_eq!(parse_listing(&raw).unwrap(), vec![pubkeys[1], pubkeys[0]]);
    }

    /// A bad entry names itself (index and value) and stops the command before any
    /// client is created, so no partial list can be published.
    #[test]
    fn invalid_entry_reports_its_index_and_value() {
        let good = keys(1)[0].to_bech32().unwrap();
        let raw = format!("[\"{good}\", \"npub1notapubkey\"]");
        let error = parse_listing(&raw).unwrap_err().to_string();
        assert!(error.contains("entry 1"), "names the index: {error}");
        assert!(
            error.contains("npub1notapubkey"),
            "names the value: {error}"
        );
    }

    #[test]
    fn empty_file_is_rejected() {
        let error = parse_listing("   \n\n").unwrap_err().to_string();
        assert!(error.contains("empty"), "{error}");
    }

    #[test]
    fn object_without_users_array_is_rejected() {
        let error = parse_listing("{\"count\":0}").unwrap_err().to_string();
        assert!(error.contains("users"), "{error}");
    }

    #[test]
    fn add_keeps_existing_follows_and_appends_the_new_ones() {
        let existing = keys(2);
        let extra = keys(2);
        let wanted = vec![extra[0], existing[1], extra[1]];

        let plan = plan(&existing, &wanted, ListMode::Add);

        assert_eq!(plan.contacts.len(), 4);
        assert_eq!(&plan.contacts[..2], &existing[..], "existing order kept");
        assert_eq!(plan.added, vec![extra[0], extra[1]]);
        assert!(plan.removed.is_empty());
    }

    #[test]
    fn add_is_a_noop_when_every_entry_is_already_followed() {
        let existing = keys(3);
        let plan = plan(&existing, &existing, ListMode::Add);
        assert!(plan.is_noop(), "nothing changes, so nothing is published");
        assert_eq!(plan.contacts, existing);
    }

    #[test]
    fn replace_publishes_exactly_the_file_set() {
        let existing = keys(3);
        let wanted = vec![existing[2], keys(1)[0]];

        let plan = plan(&existing, &wanted, ListMode::Replace);

        assert_eq!(plan.contacts, wanted);
        assert_eq!(plan.added.len(), 1);
        assert_eq!(plan.removed, vec![existing[0], existing[1]]);
    }

    #[test]
    fn replace_of_the_same_set_is_a_noop() {
        let existing = keys(2);
        let plan = plan(&existing, &existing, ListMode::Replace);
        assert!(plan.is_noop());
    }

    #[test]
    fn remove_drops_only_the_file_set() {
        let existing = keys(3);
        let plan = plan(&existing, &[existing[1]], ListMode::Remove);
        assert_eq!(plan.contacts, vec![existing[0], existing[2]]);
        assert_eq!(plan.removed, vec![existing[1]]);
    }

    #[test]
    fn remove_of_an_unfollowed_set_is_a_noop() {
        let existing = keys(2);
        let plan = plan(&existing, &keys(1), ListMode::Remove);
        assert!(plan.is_noop());
        assert_eq!(plan.contacts, existing);
    }

    #[test]
    fn remove_all_empties_the_list() {
        let existing = keys(3);
        let plan = plan(&existing, &[], ListMode::RemoveAll);
        assert!(plan.contacts.is_empty());
        assert_eq!(plan.removed, existing);
        assert!(!plan.is_noop());
    }

    /// The property the command exists for: 100 entries, one kind:3.
    #[tokio::test]
    async fn a_hundred_entry_file_publishes_once() {
        let wanted = keys(100);
        let plan = plan(&[], &wanted, ListMode::Add);

        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let published_size = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = calls.clone();
        let size = published_size.clone();
        let published = publish_plan(&plan, move |contacts| async move {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            size.store(contacts.len(), std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap();

        assert!(published);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            published_size.load(std::sync::atomic::Ordering::SeqCst),
            100
        );
    }

    #[tokio::test]
    async fn an_unchanged_file_publishes_nothing() {
        let existing = keys(2);
        let plan = plan(&existing, &existing, ListMode::Add);
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = calls.clone();

        let published = publish_plan(&plan, move |_| async move {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap();

        assert!(!published);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}
