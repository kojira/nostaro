//! NIP-10 tags for a kind:1 reply.
//!
//! Marked `e` tags (the preferred NIP-10 scheme):
//! - replying to a thread root: one `e` tag marked `root` pointing at the target;
//! - replying deeper in a thread: an `e` tag marked `root` (the thread root) and one marked
//!   `reply` (the note being answered).
//!
//! The thread root of the target is its `e` tag marked `root`; a target that only uses the
//! deprecated positional scheme has its root in its first `e` tag.
//!
//! The reply's `p` tags are the target's `p` tags plus the target's author (NIP-10 "The p tag").
//!
//! Kind:1 replies must not answer other kinds (NIP-10 points to NIP-22 for those), so a
//! non-kind-1 target is refused.

use anyhow::{bail, Result};
use nostr_sdk::prelude::*;

/// Builds the tags of a kind:1 reply to `target`.
pub fn reply_tags(target: &Event) -> Result<Vec<Tag>> {
    if target.kind != Kind::TextNote {
        bail!(
            "cannot reply with a kind:1 note to a kind:{} event; NIP-10 replies are only for kind:1 (other kinds use NIP-22 comments, which nostaro does not support)",
            target.kind.as_u16()
        );
    }
    let target_id = target.id.to_hex();
    let target_author = target.pubkey.to_hex();

    let mut tags = Vec::new();
    match thread_root(target) {
        None => {
            tags.push(Tag::parse(["e", &target_id, "", "root", &target_author])?);
        }
        Some(root) => {
            let mut root_tag = vec!["e".to_string(), root.id, String::new(), "root".to_string()];
            if let Some(author) = root.author {
                root_tag.push(author);
            }
            tags.push(Tag::parse(root_tag)?);
            tags.push(Tag::parse(["e", &target_id, "", "reply", &target_author])?);
        }
    }

    let mut pubkeys: Vec<String> = Vec::new();
    for pubkey in target_p_tags(target).into_iter().chain([target_author]) {
        if !pubkeys.contains(&pubkey) {
            pubkeys.push(pubkey);
        }
    }
    for pubkey in pubkeys {
        tags.push(Tag::parse(["p", &pubkey])?);
    }
    Ok(tags)
}

struct Root {
    id: String,
    author: Option<String>,
}

/// The thread root the target belongs to, or `None` when the target is itself a root.
fn thread_root(target: &Event) -> Option<Root> {
    let e_tags: Vec<&[String]> = target
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter(|tag| tag.len() >= 2 && tag[0] == "e" && EventId::from_hex(&tag[1]).is_ok())
        .collect();
    let marked_root = e_tags
        .iter()
        .find(|tag| tag.get(3).map(String::as_str) == Some("root"));
    // Without a `root` marker, fall back to the deprecated positional scheme: the first
    // `e` tag is the root.
    let root = match marked_root {
        Some(tag) => *tag,
        None => *e_tags.first()?,
    };
    let author = root
        .get(4)
        .filter(|pubkey| PublicKey::from_hex(pubkey.as_str()).is_ok())
        .cloned();
    Some(Root {
        id: root[1].clone(),
        author,
    })
}

fn target_p_tags(target: &Event) -> Vec<String> {
    target
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter(|tag| tag.len() >= 2 && tag[0] == "p" && PublicKey::from_hex(&tag[1]).is_ok())
        .map(|tag| tag[1].to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(keys: &Keys, kind: Kind, tags: Vec<Vec<&str>>) -> Event {
        let tags = tags
            .into_iter()
            .map(|t| Tag::parse(t).unwrap())
            .collect::<Vec<_>>();
        EventBuilder::new(kind, "x")
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap()
    }

    fn strings(tags: &[Tag]) -> Vec<Vec<String>> {
        tags.iter().map(|t| t.as_slice().to_vec()).collect()
    }

    const ROOT: &str = "a2106aba9d49aabae4d2c89815fa09dc48d42dbe96ad8deca05f628ade30a185";
    const MID: &str = "a5511926bc287c529c09fa61554d392ae4338a6e8a5a0c74d2e23266f4367185";
    const P1: &str = "9be0be987e28ff62d7cadbd0696b47a87d33c7de3fee4092adafa1b9d683d918";
    const P2: &str = "21ac29561b5de90cdc21995fc0707525cd78c8a52d87721ab681d3d609d1e2df";

    #[test]
    fn reply_to_a_root_note_has_a_single_root_marked_e_tag() {
        let author = Keys::generate();
        let target = event(&author, Kind::TextNote, vec![]);
        let tags = strings(&reply_tags(&target).unwrap());
        let id = target.id.to_hex();
        let pk = author.public_key().to_hex();
        assert_eq!(
            tags,
            vec![
                vec![
                    "e".to_string(),
                    id,
                    String::new(),
                    "root".into(),
                    pk.clone()
                ],
                vec!["p".to_string(), pk],
            ]
        );
    }

    #[test]
    fn reply_to_a_marked_reply_keeps_its_root_and_marks_the_target_as_reply() {
        let author = Keys::generate();
        let target = event(
            &author,
            Kind::TextNote,
            vec![
                vec!["e", MID, "wss://yabu.me/", "reply"],
                vec!["e", ROOT, "wss://relay.example/", "root", P2],
                vec!["p", P1],
            ],
        );
        let tags = strings(&reply_tags(&target).unwrap());
        let id = target.id.to_hex();
        let pk = author.public_key().to_hex();
        assert_eq!(tags[0], vec!["e", ROOT, "", "root", P2]);
        assert_eq!(
            tags[1],
            vec!["e".to_string(), id, String::new(), "reply".into(), pk]
        );
        assert_eq!(tags.len(), 4);
    }

    #[test]
    fn root_marker_without_author_omits_the_pubkey_field() {
        let author = Keys::generate();
        let target = event(&author, Kind::TextNote, vec![vec!["e", ROOT, "", "root"]]);
        let tags = strings(&reply_tags(&target).unwrap());
        assert_eq!(tags[0], vec!["e", ROOT, "", "root"]);
    }

    #[test]
    fn positional_e_tags_take_the_first_one_as_the_root() {
        let author = Keys::generate();
        let target = event(
            &author,
            Kind::TextNote,
            vec![vec!["e", ROOT], vec!["e", MID]],
        );
        let tags = strings(&reply_tags(&target).unwrap());
        assert_eq!(tags[0], vec!["e", ROOT, "", "root"]);
        assert_eq!(tags[1][3], "reply");
        assert_eq!(tags[1][1], target.id.to_hex());
    }

    #[test]
    fn reply_marker_without_root_falls_back_to_the_first_e_tag() {
        // Older nostaro replies carried only ["e", <target>, "", "reply"].
        let author = Keys::generate();
        let target = event(&author, Kind::TextNote, vec![vec!["e", MID, "", "reply"]]);
        let tags = strings(&reply_tags(&target).unwrap());
        assert_eq!(tags[0], vec!["e", MID, "", "root"]);
        assert_eq!(tags[1][1], target.id.to_hex());
        assert_eq!(tags[1][3], "reply");
    }

    #[test]
    fn p_tags_merge_the_targets_p_tags_and_its_author_without_duplicates() {
        let author = Keys::generate();
        let pk = author.public_key().to_hex();
        let target = event(
            &author,
            Kind::TextNote,
            vec![
                vec!["p", P1, "wss://yabu.me/"],
                vec!["p", P2],
                vec!["p", P1],
                vec!["p", &pk],
            ],
        );
        let ps: Vec<String> = strings(&reply_tags(&target).unwrap())
            .into_iter()
            .filter(|t| t[0] == "p")
            .map(|t| t[1].clone())
            .collect();
        assert_eq!(ps, vec![P1.to_string(), P2.to_string(), pk]);
    }

    #[test]
    fn non_kind_1_target_is_refused() {
        let author = Keys::generate();
        let target = event(&author, Kind::Reaction, vec![]);
        let error = reply_tags(&target).unwrap_err().to_string();
        assert!(error.contains("kind:7"), "{error}");
        assert!(error.contains("NIP-22"), "{error}");
    }
}
