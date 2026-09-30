use anyhow::Result;
use nostr_sdk::prelude::*;

use crate::client;
use crate::config::NostaroConfig;
use crate::keys;

/// `--mention` の相手を本文先頭の `nostr:npub1…` と p タグにする（NIP-27 / NIP-10）。
/// 相手は hex か npub1 を受ける。p タグで相手に通知が届き、本文の URI で多くのクライアントが
/// @名前 と表示する。
pub fn apply_mention(content: &str, mention: &str) -> Result<(String, Tag)> {
    let pubkey = PublicKey::parse(mention)?;
    let content = format!("nostr:{} {}", pubkey.to_bech32()?, content);
    Ok((content, Tag::public_key(pubkey)))
}

pub async fn run(message: &str, quote: Option<&str>, mention: Option<&str>) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let mut content = message.to_string();
    let mut extra_tags: Vec<Tag> = Vec::new();

    if let Some(mention) = mention {
        let (with_mention, tag) = apply_mention(&content, mention)?;
        content = with_mention;
        extra_tags.push(tag);
    }

    if let Some(quote_str) = quote {
        // Parse the quoted entity
        let (event_id, relay_hint) = if quote_str.starts_with("nevent1") {
            let nip19_event = Nip19Event::from_bech32(quote_str)?;
            let relay = nip19_event
                .relays
                .first()
                .map(|r| r.to_string())
                .unwrap_or_default();
            (nip19_event.event_id, relay)
        } else {
            let id = EventId::parse(quote_str).or_else(|_| EventId::from_bech32(quote_str))?;
            (id, String::new())
        };

        // Add q tag (NIP-18 quote repost)
        extra_tags.push(Tag::custom(
            TagKind::custom("q".to_string()),
            vec![event_id.to_hex(), relay_hint],
        ));

        // Append nostr: URI to content
        content.push_str(&format!("\n\nnostr:{}", quote_str));
    }

    println!("Publishing note...");
    if extra_tags.is_empty() {
        client::post_note(&nostr_client, &content).await?;
    } else {
        // Build event with extra tags
        let builder = EventBuilder::text_note(&content).tags(extra_tags);
        let output = client::publish(&nostr_client, builder).await?;
        println!("Event ID: {}", output.id().to_bech32()?);
    }
    println!("Note published successfully!");

    nostr_client.disconnect().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";

    #[test]
    fn mention_prefixes_npub_uri_and_adds_p_tag() {
        let npub = PublicKey::parse(HEX).unwrap().to_bech32().unwrap();
        for input in [HEX, npub.as_str()] {
            let (content, tag) = apply_mention("こんにちは", input).unwrap();
            assert_eq!(content, format!("nostr:{npub} こんにちは"));
            assert_eq!(tag.as_slice(), &["p".to_string(), HEX.to_string()]);
        }
    }

    #[test]
    fn mention_rejects_non_pubkey() {
        assert!(apply_mention("x", "u2").is_err());
    }
}
