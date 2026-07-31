use anyhow::{anyhow, bail, Context, Result};
use nostr_sdk::prelude::*;
use serde::Deserialize;
use std::path::Path;

use crate::client;
use crate::config::NostaroConfig;
use crate::keys;

/// Fields that describe a *signed* event. They cannot be honoured here (nostaro
/// signs with the configured key), so a file containing them is rejected
/// instead of silently publishing something different from what was written.
const SIGNED_ONLY_FIELDS: [&str; 4] = ["id", "sig", "pubkey", "created_at"];

/// An unsigned event as described by a `--file` JSON document.
///
/// ```json
/// { "kind": 3, "content": "", "tags": [["p", "<hex>"], ["p", "<hex>"]] }
/// ```
///
/// `deny_unknown_fields` turns a typo (`"tag"` instead of `"tags"`) into an
/// error rather than an event that quietly lost all of its tags.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventSpec {
    /// Event kind (required).
    pub kind: u16,
    /// Event content; defaults to the empty string (kind:3 and friends).
    #[serde(default)]
    pub content: String,
    /// Tags as arrays of strings; defaults to none.
    #[serde(default)]
    pub tags: Vec<Vec<String>>,
}

impl EventSpec {
    /// Convert the raw string arrays into nostr tags.
    pub fn parsed_tags(&self) -> Result<Vec<Tag>> {
        let mut parsed = Vec::with_capacity(self.tags.len());
        for (index, values) in self.tags.iter().enumerate() {
            if values.is_empty() {
                bail!(
                    "tags[{}] is empty; every tag needs at least a name element, e.g. [\"p\", \"<hex>\"]",
                    index
                );
            }
            let tag = Tag::parse(values.clone())
                .with_context(|| format!("tags[{}] is not a valid nostr tag", index))?;
            parsed.push(tag);
        }
        Ok(parsed)
    }
}

/// Assemble the unsigned event that gets signed and published. Shared by the
/// `--file` and the inline-flag paths.
pub fn build_event(kind: u16, content: String, tags: Vec<Tag>) -> EventBuilder {
    EventBuilder::new(Kind::from(kind), content).tags(tags)
}

/// Parse the JSON document of an event file.
pub fn parse_event_spec(json: &str) -> Result<EventSpec> {
    if json.trim().is_empty() {
        bail!("the event file is empty; it must contain a JSON object such as {{\"kind\":1,\"content\":\"hello\"}}");
    }

    let value: serde_json::Value =
        serde_json::from_str(json).context("the event file is not valid JSON")?;

    let object = value.as_object().ok_or_else(|| {
        anyhow!("the event file must contain a JSON object with \"kind\", and optionally \"content\" and \"tags\"")
    })?;

    for field in SIGNED_ONLY_FIELDS {
        if object.contains_key(field) {
            bail!(
                "\"{}\" must not appear in an event file: nostaro takes only kind/content/tags \
                 from the file and fills in pubkey, created_at, id and sig itself when it signs. \
                 Remove the field and retry.",
                field
            );
        }
    }

    let spec: EventSpec = serde_json::from_value(value)
        .context("the event file does not describe an unsigned event")?;
    Ok(spec)
}

/// Read and parse an event file.
pub fn load_event_spec(path: &Path) -> Result<EventSpec> {
    let json = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read event file {}", path.display()))?;
    parse_event_spec(&json).with_context(|| format!("invalid event file {}", path.display()))
}

/// Parse the repeatable `--tag "key,value"` flags.
fn parse_tag_flags(tags: &[String]) -> Result<Vec<Tag>> {
    let mut parsed = Vec::with_capacity(tags.len());
    for tag_str in tags {
        let parts: Vec<String> = tag_str.split(',').map(|s| s.to_string()).collect();
        if parts.len() < 2 {
            bail!(
                "Invalid tag format: '{}'. Expected 'key,value[,value...]'",
                tag_str
            );
        }
        parsed.push(Tag::parse(parts)?);
    }
    Ok(parsed)
}

pub async fn run(
    kind: Option<u16>,
    tags: Vec<String>,
    content: Option<String>,
    file: Option<&Path>,
) -> Result<()> {
    // Resolve the event before touching the config, the key or the network, so
    // that a malformed file fails fast and without connecting anywhere.
    let (kind, content, parsed_tags) = match file {
        Some(path) => {
            let spec = load_event_spec(path)?;
            let parsed_tags = spec
                .parsed_tags()
                .with_context(|| format!("invalid event file {}", path.display()))?;
            (spec.kind, spec.content, parsed_tags)
        }
        None => {
            let kind = kind.ok_or_else(|| anyhow!("--kind is required unless --file is given"))?;
            (kind, content.unwrap_or_default(), parse_tag_flags(&tags)?)
        }
    };

    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let tag_count = parsed_tags.len();
    println!("Publishing kind:{} event ({} tag(s))...", kind, tag_count);
    let builder = build_event(kind, content, parsed_tags);
    let output = nostr_client.send_event_builder(builder).await?;
    println!("Event published! ID: {}", output.id().to_hex());

    nostr_client.disconnect().await;
    Ok(())
}
