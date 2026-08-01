use anyhow::Result;
use nostr_sdk::prelude::*;

use crate::client;
use crate::config::NostaroConfig;
use crate::keys;
use crate::outln;
use crate::output;
use crate::utils::resolve_pubkey;

/// One entry of a following/followers listing: the npub and the hex pubkey
/// (what a kind:3 `p` tag needs).
///
/// Deliberately no display name. Listing a follow set is one kind:3 read; a
/// name per entry would be one kind:0 read per entry on top of it (979 follows
/// meant 979 round trips), which dwarfs the actual work and is pure waste for
/// `--out-format json`. Names are `nostaro profile show --pubkey <npub or hex>`'s
/// job.
struct Entry {
    npub: String,
    hex: String,
}

/// Turn the pubkeys of a kind:3 into printable entries. Pure conversion — it
/// talks to no relay.
fn describe(pubkeys: &[PublicKey]) -> Result<Vec<Entry>> {
    pubkeys
        .iter()
        .map(|pubkey| {
            Ok(Entry {
                npub: pubkey.to_bech32()?,
                hex: pubkey.to_hex(),
            })
        })
        .collect()
}

/// Assembling a listing takes pubkeys and nothing else — no client, no `async`,
/// so it cannot reach a relay. This pins that shape: putting a name back on an
/// entry means fetching a kind:0 per entry, which means `describe` has to grow a
/// `&Client` or become `async`, and this line stops compiling. If you are here
/// because of that error, you are about to re-add 979 round trips to a
/// 979-follow listing — that is the cost #8 removed.
const _: fn(&[PublicKey]) -> Result<Vec<Entry>> = describe;

/// The `--out-format json` document: npub and hex per user, nothing else.
fn to_json(entries: &[Entry]) -> serde_json::Value {
    let users: Vec<serde_json::Value> = entries
        .iter()
        .map(|entry| {
            serde_json::json!({
                "npub": entry.npub,
                "hex": entry.hex,
            })
        })
        .collect();
    serde_json::json!({
        "count": users.len(),
        "users": users,
    })
}

/// Emit the listing: one line per user, or a JSON document when the caller
/// asked for `--out-format json`.
fn emit(entries: &[Entry]) -> Result<()> {
    if output::is_json() {
        return output::write_json(&to_json(entries));
    }

    output::open_body()?;
    for entry in entries {
        outln!("  {}", entry.npub)?;
    }
    Ok(())
}

pub async fn follow(pubkey_str: &str) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let pubkey = resolve_pubkey(pubkey_str)?;

    let mut contacts = client::fetch_contacts(&nostr_client, &keys.public_key()).await?;

    if contacts.contains(&pubkey) {
        println!("Already following {}", pubkey.to_bech32()?);
        nostr_client.disconnect().await;
        return Ok(());
    }

    contacts.push(pubkey);

    client::publish_contact_list(&nostr_client, &contacts).await?;
    println!("Now following {}", pubkey.to_bech32()?);

    nostr_client.disconnect().await;
    Ok(())
}

pub async fn unfollow(pubkey_str: &str) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let pubkey = resolve_pubkey(pubkey_str)?;

    let mut contacts = client::fetch_contacts(&nostr_client, &keys.public_key()).await?;

    if !contacts.contains(&pubkey) {
        println!("Not following {}", pubkey.to_bech32()?);
        nostr_client.disconnect().await;
        return Ok(());
    }

    contacts.retain(|&p| p != pubkey);

    client::publish_contact_list(&nostr_client, &contacts).await?;
    println!("Unfollowed {}", pubkey.to_bech32()?);

    nostr_client.disconnect().await;
    Ok(())
}

pub async fn following(npub_str: Option<&str>) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let target_pubkey = match npub_str {
        Some(s) => resolve_pubkey(s)?,
        None => keys.public_key(),
    };

    let contacts = client::fetch_contacts(&nostr_client, &target_pubkey).await?;

    if contacts.is_empty() {
        if npub_str.is_some() {
            println!("{} is not following anyone.", target_pubkey.to_bech32()?);
        } else {
            println!("You're not following anyone yet.");
        }
        // An empty result is still a result: --out gets an empty listing.
        emit(&[])?;
        nostr_client.disconnect().await;
        return Ok(());
    }

    println!("Following {} user(s):", contacts.len());
    let entries = describe(&contacts)?;
    emit(&entries)?;

    nostr_client.disconnect().await;
    Ok(())
}

pub async fn followers(npub_str: Option<&str>) -> Result<()> {
    let config = NostaroConfig::load()?;
    let keys = keys::keys_from_config(&config)?;
    let nostr_client = client::create_client(&keys, &config).await?;

    let target_pubkey = match npub_str {
        Some(s) => resolve_pubkey(s)?,
        None => keys.public_key(),
    };

    let follower_list = client::fetch_followers(&nostr_client, &target_pubkey).await?;

    if follower_list.is_empty() {
        if npub_str.is_some() {
            println!("No followers found for {}.", target_pubkey.to_bech32()?);
        } else {
            println!("No followers found.");
        }
        emit(&[])?;
        nostr_client.disconnect().await;
        return Ok(());
    }

    println!("{} follower(s):", follower_list.len());
    let entries = describe(&follower_list)?;
    emit(&entries)?;

    nostr_client.disconnect().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pubkeys() -> Vec<PublicKey> {
        (0..3).map(|_| Keys::generate().public_key()).collect()
    }

    #[test]
    fn describe_yields_npub_and_hex_only() {
        let pubkeys = sample_pubkeys();
        let entries = describe(&pubkeys).unwrap();

        assert_eq!(entries.len(), pubkeys.len());
        for (entry, pubkey) in entries.iter().zip(&pubkeys) {
            assert_eq!(entry.npub, pubkey.to_bech32().unwrap());
            assert_eq!(entry.hex, pubkey.to_hex());
        }
    }

    #[test]
    fn json_users_carry_no_name_field() {
        let pubkeys = sample_pubkeys();
        let document = to_json(&describe(&pubkeys).unwrap());

        assert_eq!(document["count"], pubkeys.len());
        let users = document["users"].as_array().unwrap();
        assert_eq!(users.len(), pubkeys.len());
        for (user, pubkey) in users.iter().zip(&pubkeys) {
            let object = user.as_object().unwrap();
            // Key *set*, not key order: with `preserve_order` on, a serde_json
            // map keeps insertion order instead of sorting, and an ordered
            // compare would fail without the output having changed.
            assert!(
                object.contains_key("npub") && object.contains_key("hex"),
                "the JSON body carries npub and hex"
            );
            assert_eq!(
                object.len(),
                2,
                "the JSON body is npub + hex, nothing else (notably no name)"
            );
            assert_eq!(object["npub"], pubkey.to_bech32().unwrap());
            assert_eq!(object["hex"], pubkey.to_hex());
        }
    }

    /// An empty listing is still a listing: `count: 0` with an empty array,
    /// never a missing `users` key.
    #[test]
    fn json_of_an_empty_listing_is_still_a_document() {
        let document = to_json(&[]);
        assert_eq!(document["count"], 0);
        assert_eq!(document["users"].as_array().unwrap().len(), 0);
    }

    /// The text body is bare npubs — one per line, no name in parentheses.
    ///
    /// The output sink is process-global, so this is the only test in the lib
    /// test binary that touches it.
    #[test]
    fn text_body_is_bare_npubs() {
        use crate::output::OutFormat;
        use std::path::PathBuf;

        // pid in the name: two `cargo test` runs over the same checkout would
        // otherwise write the same file and read each other's body.
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("nostaro_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("following.txt");

        let pubkeys = sample_pubkeys();
        let entries = describe(&pubkeys).unwrap();

        output::configure(Some(path.clone()), OutFormat::Text);
        emit(&entries).unwrap();
        output::finish().unwrap();
        output::configure(None, OutFormat::Text);

        let body = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let expected: String = pubkeys
            .iter()
            .map(|pubkey| format!("  {}\n", pubkey.to_bech32().unwrap()))
            .collect();
        assert_eq!(body, expected);
        assert!(!body.contains('('), "no name decoration: {}", body);
    }
}
