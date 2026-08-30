//! Success output for the publishing commands (`post`, `reply`).
//!
//! Issue #18: `reply`/`post` used to discard the `Output<EventId>` they got back
//! and print only "…published successfully!", so a caller (OpenCrab's
//! nostr-gateway) had no way to learn the id of the event it just published.
//!
//! The fix is purely **additive**:
//!
//! * default (text) mode keeps every line it printed before and appends one more,
//!   `Event id: <64-hex>` (case A), so an existing consumer that greps the old
//!   lines is untouched;
//! * `--json` mode prints a single `{"event_id":"<64-hex>"}` document on stdout
//!   and routes the human status lines to stderr (case B), mirroring how
//!   `vanity --json` already behaves.
//!
//! The id is emitted as 64-char lowercase hex (not bech32) so the consumer can
//! capture it without decoding.

use nostr_sdk::prelude::*;

/// The `--json` success document for a published event: `{"event_id":"<hex>"}`.
pub fn event_id_json(id: &EventId) -> String {
    serde_json::json!({ "event_id": id.to_hex() }).to_string()
}

/// The text line appended after a successful publish (case A).
pub fn event_id_line(id: &EventId) -> String {
    format!("Event id: {}", id.to_hex())
}

/// A human status line. In text mode it goes to stdout exactly as before; in
/// `--json` mode it goes to stderr so stdout carries only the JSON document.
pub fn status(json: bool, msg: &str) {
    if json {
        eprintln!("{}", msg);
    } else {
        println!("{}", msg);
    }
}

/// The machine-readable result, always on stdout: the JSON document in `--json`
/// mode, otherwise the `Event id:` line.
pub fn emit_event_id(json: bool, id: &EventId) {
    if json {
        println!("{}", event_id_json(id));
    } else {
        println!("{}", event_id_line(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_id() -> EventId {
        // A real, deterministic id: sign a note with a throwaway key and take its
        // event id, so the test needs no hand-written hex constant to keep valid.
        let keys = Keys::generate();
        EventBuilder::text_note("issue #18 fixture")
            .sign_with_keys(&keys)
            .expect("sign")
            .id
    }

    #[test]
    fn text_line_is_the_lowercase_hex_id() {
        let id = sample_id();
        let line = event_id_line(&id);
        assert_eq!(line, format!("Event id: {}", id.to_hex()));
        // The whole point of #18: the consumer must get the 64-char hex form.
        let hex = line.strip_prefix("Event id: ").expect("has the prefix");
        assert_eq!(hex.len(), 64);
        assert!(hex
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(hex, id.to_hex());
    }

    #[test]
    fn json_document_carries_the_hex_id_under_event_id() {
        let id = sample_id();
        let value: serde_json::Value =
            serde_json::from_str(&event_id_json(&id)).expect("valid JSON");
        assert_eq!(value["event_id"], serde_json::json!(id.to_hex()));
        // Nothing but the one field, so the shape stays stable for consumers.
        assert_eq!(
            value
                .as_object()
                .expect("object")
                .keys()
                .collect::<Vec<_>>(),
            vec!["event_id"]
        );
    }
}
