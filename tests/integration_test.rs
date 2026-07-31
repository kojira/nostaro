use std::path::PathBuf;

// ── Config roundtrip via file I/O ────────────────────────────────────

#[test]
fn config_save_load_roundtrip() {
    let dir = std::env::temp_dir().join(format!("nostaro_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");

    let config = nostaro::config::NostaroConfig {
        secret_key: Some("deadbeef".to_string()),
        relays: vec![
            "wss://relay.example.com".to_string(),
            "wss://relay2.example.com".to_string(),
        ],
        default_relays: vec!["wss://default.example.com".to_string()],
        blossom_server: None,
        coinos_api_token_path: None,
    };

    config.save_to(&path).unwrap();

    let loaded = nostaro::config::NostaroConfig::load_from(&path).unwrap();
    assert_eq!(loaded.secret_key, config.secret_key);
    assert_eq!(loaded.relays, config.relays);
    assert_eq!(loaded.default_relays, config.default_relays);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn config_load_nonexistent_returns_default() {
    let path = PathBuf::from("/tmp/nostaro_nonexistent_config_v3_test.toml");
    std::fs::remove_file(&path).ok();

    let config = nostaro::config::NostaroConfig::load_from(&path).unwrap();
    assert!(config.secret_key.is_none());
    assert!(config.relays.is_empty());
    assert_eq!(config.default_relays.len(), 4);
}

#[test]
fn config_active_relays_prefers_custom() {
    let mut config = nostaro::config::NostaroConfig::default();
    assert_eq!(config.active_relays(), config.default_relays);

    config.relays = vec!["wss://custom.relay".to_string()];
    assert_eq!(config.active_relays(), vec!["wss://custom.relay"]);
}

#[test]
fn config_blossom_url_default() {
    let config = nostaro::config::NostaroConfig::default();
    assert_eq!(config.blossom_url(), "https://blossom.primal.net");
}

#[test]
fn config_blossom_url_custom() {
    let config = nostaro::config::NostaroConfig {
        blossom_server: Some("https://custom.blossom.server".to_string()),
        ..nostaro::config::NostaroConfig::default()
    };
    assert_eq!(config.blossom_url(), "https://custom.blossom.server");
}

#[test]
fn config_backward_compatible_without_blossom() {
    let toml_str = r#"
secret_key = "nsec1test"
relays = ["wss://relay.damus.io"]
default_relays = ["wss://relay.damus.io"]
"#;
    let config: nostaro::config::NostaroConfig = toml::from_str(toml_str).unwrap();
    assert!(config.blossom_server.is_none());
    assert_eq!(config.secret_key, Some("nsec1test".to_string()));
}

// ── Key generation ───────────────────────────────────────────────────

#[test]
fn key_generation_produces_valid_keys() {
    let keys = nostaro::keys::generate_keys();
    let npub = keys.public_key().to_bech32().unwrap();
    let nsec = keys.secret_key().to_bech32().unwrap();

    assert!(npub.starts_with("npub1"));
    assert!(nsec.starts_with("nsec1"));
    assert!(npub.len() > 10);
    assert!(nsec.len() > 10);
}

#[test]
fn key_generation_is_unique() {
    let keys1 = nostaro::keys::generate_keys();
    let keys2 = nostaro::keys::generate_keys();
    assert_ne!(keys1.public_key(), keys2.public_key());
}

#[test]
fn keys_from_config_with_valid_nsec() {
    let keys = nostaro::keys::generate_keys();
    let nsec = keys.secret_key().to_bech32().unwrap();

    let config = nostaro::config::NostaroConfig {
        secret_key: Some(nsec),
        ..nostaro::config::NostaroConfig::default()
    };

    let loaded = nostaro::keys::keys_from_config(&config).unwrap();
    assert_eq!(loaded.public_key(), keys.public_key());
}

#[test]
fn keys_from_config_with_hex_key() {
    let keys = nostaro::keys::generate_keys();
    let hex_secret = keys.secret_key().to_secret_hex();

    let config = nostaro::config::NostaroConfig {
        secret_key: Some(hex_secret),
        ..nostaro::config::NostaroConfig::default()
    };

    let loaded = nostaro::keys::keys_from_config(&config).unwrap();
    assert_eq!(loaded.public_key(), keys.public_key());
}

#[test]
fn keys_from_config_missing_key_errors() {
    let config = nostaro::config::NostaroConfig::default();
    let result = nostaro::keys::keys_from_config(&config);
    assert!(result.is_err());
}

// ── Cache tests ──────────────────────────────────────────────────────

#[test]
fn cache_store_and_retrieve_event() {
    let cache = nostaro::cache::CacheDb::open().unwrap();
    let test_id = format!("test_event_{}", std::process::id());
    cache
        .store_event(&test_id, "pubkey1", 1, "test content", 12345, "[]", "{}")
        .unwrap();
    let event = cache.get_event(&test_id).unwrap().unwrap();
    assert_eq!(event.content, "test content");
    assert_eq!(event.kind, 1);
    assert_eq!(event.created_at, 12345);
}

#[test]
fn cache_store_and_retrieve_profile() {
    let cache = nostaro::cache::CacheDb::open().unwrap();
    let test_pk = format!("test_pk_{}", std::process::id());
    cache
        .store_profile(&test_pk, Some("alice"), Some("Alice"), Some("bio"), None)
        .unwrap();
    let profile = cache.get_profile(&test_pk).unwrap().unwrap();
    assert_eq!(profile.name.unwrap(), "alice");
    assert_eq!(profile.display_name.unwrap(), "Alice");
    assert!(profile.picture.is_none());
}

// ── Event file input (`event --file`) ────────────────────────────────

use nostaro::commands::event::{load_event_spec, parse_event_spec};

/// Scratch directory for tests that touch the filesystem. `CARGO_TARGET_TMPDIR`
/// keeps them inside `target/`, so nothing leaks into the user's temp dir.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn event_file_minimal_document() {
    let spec = parse_event_spec(r#"{"kind": 1, "content": "hello"}"#).unwrap();
    assert_eq!(spec.kind, 1);
    assert_eq!(spec.content, "hello");
    assert!(spec.tags.is_empty());
    assert!(spec.parsed_tags().unwrap().is_empty());
}

#[test]
fn event_file_content_and_tags_default_to_empty() {
    let spec = parse_event_spec(r#"{"kind": 3}"#).unwrap();
    assert_eq!(spec.content, "");
    assert!(spec.tags.is_empty());
}

#[test]
fn event_file_carries_a_thousand_p_tags() {
    // The whole point of the file input: a follow list that could never be
    // passed as 1000 `--tag` arguments.
    let pubkeys: Vec<String> = (0..1000).map(|i| format!("{:064x}", i)).collect();
    let tags: Vec<Vec<String>> = pubkeys
        .iter()
        .map(|pk| vec!["p".into(), pk.clone()])
        .collect();
    let json = serde_json::json!({ "kind": 3, "content": "", "tags": tags }).to_string();

    let spec = parse_event_spec(&json).unwrap();
    assert_eq!(spec.kind, 3);
    assert_eq!(spec.tags.len(), 1000);

    let parsed = spec.parsed_tags().unwrap();
    assert_eq!(parsed.len(), 1000);
    // Every tag survives as a real nostr p tag, in file order.
    for (index, tag) in parsed.iter().enumerate() {
        assert_eq!(tag.as_slice(), ["p".to_string(), pubkeys[index].clone()]);
    }

    // ...and all 1000 of them end up in a single signed event, which is what a
    // kind:3 follow list needs.
    let keys = nostaro::keys::generate_keys();
    let event = nostaro::commands::event::build_event(spec.kind, spec.content, parsed)
        .sign_with_keys(&keys)
        .unwrap();
    assert_eq!(event.kind.as_u16(), 3);
    assert_eq!(event.tags.len(), 1000);
    assert!(event.verify().is_ok());
}

#[test]
fn event_file_keeps_tag_values_containing_commas() {
    // Something `--tag "key,value"` cannot express, since it splits on commas.
    let spec = parse_event_spec(r#"{"kind": 1, "tags": [["alt", "a,b,c"]]}"#).unwrap();
    let parsed = spec.parsed_tags().unwrap();
    assert_eq!(
        parsed[0].as_slice(),
        ["alt".to_string(), "a,b,c".to_string()]
    );
}

#[test]
fn event_file_rejects_signed_only_fields() {
    for field in ["id", "sig", "pubkey", "created_at"] {
        let json = format!(r#"{{"kind": 1, "content": "x", "{}": "whatever"}}"#, field);
        let err = parse_event_spec(&json).unwrap_err().to_string();
        assert!(
            err.contains(field) && err.contains("must not appear"),
            "field {} should be rejected explicitly, got: {}",
            field,
            err
        );
    }
}

#[test]
fn event_file_rejects_unknown_fields() {
    // A "tag"/"tags" typo must not silently publish a tagless event.
    let err = parse_event_spec(r#"{"kind": 1, "tag": [["p", "abc"]]}"#)
        .unwrap_err()
        .to_string();
    assert!(err.contains("unsigned event"), "got: {}", err);
}

#[test]
fn event_file_requires_kind() {
    let err = parse_event_spec(r#"{"content": "hello"}"#)
        .unwrap_err()
        .to_string();
    assert!(err.contains("unsigned event"), "got: {}", err);
}

#[test]
fn event_file_rejects_empty_document() {
    let err = parse_event_spec("   \n").unwrap_err().to_string();
    assert!(err.contains("empty"), "got: {}", err);
}

#[test]
fn event_file_rejects_broken_json() {
    let err = parse_event_spec(r#"{"kind": 1,"#).unwrap_err().to_string();
    assert!(err.contains("not valid JSON"), "got: {}", err);
}

#[test]
fn event_file_rejects_non_object_json() {
    let err = parse_event_spec("[1, 2, 3]").unwrap_err().to_string();
    assert!(err.contains("JSON object"), "got: {}", err);
}

#[test]
fn event_file_rejects_empty_tag() {
    let hex = "0".repeat(64);
    let json = format!(r#"{{"kind": 1, "tags": [["p", "{}"], []]}}"#, hex);
    let spec = parse_event_spec(&json).unwrap();
    let err = spec.parsed_tags().unwrap_err().to_string();
    assert!(err.contains("tags[1]"), "got: {}", err);
}

#[test]
fn event_file_rejects_wrong_tag_shape() {
    // `"tags": ["p"]` is an array of strings, not of tags: a type error.
    let err = parse_event_spec(r#"{"kind": 1, "tags": ["p"]}"#)
        .unwrap_err()
        .to_string();
    assert!(err.contains("unsigned event"), "got: {}", err);
}

#[test]
fn event_file_rejects_a_tag_with_no_value() {
    // `Tag::parse` accepts ["p"], which would publish a kind:3 entry that every
    // relay and client ignores — and a kind:3 replaces the whole follow list.
    let hex = "0".repeat(64);
    let json = format!(r#"{{"kind": 3, "tags": [["p", "{}"], ["p"]]}}"#, hex);
    let spec = parse_event_spec(&json).unwrap();
    let err = spec.parsed_tags().unwrap_err().to_string();
    assert!(
        err.contains("tags[1]") && err.contains("no value"),
        "got: {}",
        err
    );
}

#[test]
fn event_file_rejects_npub_in_a_p_tag() {
    // The documented follow-list recipe feeds hex pubkeys; pasting an npub must
    // not silently drop that person from the published list.
    let npub = "npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqshp52w2";
    let json = format!(r#"{{"kind": 3, "tags": [["p", "{}"]]}}"#, npub);
    let err = parse_event_spec(&json)
        .unwrap()
        .parsed_tags()
        .unwrap_err()
        .to_string();
    assert!(err.contains("tags[0]"), "got: {}", err);
    assert!(err.contains("64-character hex"), "got: {}", err);
    assert!(err.contains("nostaro decode"), "got: {}", err);
}

#[test]
fn event_file_rejects_malformed_hex_in_p_and_e_tags() {
    let too_short = "abc";
    let not_hex = "zz00000000000000000000000000000000000000000000000000000000000z";
    for (name, value) in [("p", too_short), ("e", not_hex)] {
        let json = format!(r#"{{"kind": 1, "tags": [["{}", "{}"]]}}"#, name, value);
        let err = parse_event_spec(&json)
            .unwrap()
            .parsed_tags()
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("64-character hex"),
            "{} tag {:?} should be rejected, got: {}",
            name,
            value,
            err
        );
    }
}

#[test]
fn event_file_keeps_non_hex_tag_values_untouched() {
    // Only p/e values are hex; every other tag stays free-form.
    let spec =
        parse_event_spec(r#"{"kind": 1, "tags": [["t", "nostr"], ["alt", "a note"]]}"#).unwrap();
    assert_eq!(spec.parsed_tags().unwrap().len(), 2);
}

#[test]
fn event_file_rejects_a_file_over_the_size_limit() {
    let dir = scratch("event_file_too_big");
    let path = dir.join("huge.json");
    let padding = "x".repeat(8 * 1024 * 1024 + 1);
    std::fs::write(&path, format!(r#"{{"kind":1,"content":"{}"}}"#, padding)).unwrap();

    let err = load_event_spec(&path).unwrap_err().to_string();
    assert!(err.contains("the limit is"), "got: {}", err);
    std::fs::remove_file(&path).ok();
}

#[test]
fn event_file_load_roundtrip_from_disk() {
    let dir = scratch("event_file_load");
    let path = dir.join("follow.json");
    std::fs::write(
        &path,
        r#"{"kind": 3, "content": "", "tags": [["p", "abc"], ["p", "def"]]}"#,
    )
    .unwrap();

    let spec = load_event_spec(&path).unwrap();
    assert_eq!(spec.kind, 3);
    assert_eq!(spec.tags.len(), 2);
}

#[test]
fn event_file_missing_path_names_the_file() {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("definitely_not_here.json");
    std::fs::remove_file(&path).ok();
    let err = load_event_spec(&path).unwrap_err().to_string();
    assert!(err.contains("failed to read event file"), "got: {}", err);
}

// ── Output sink (`--out`) ────────────────────────────────────────────

// The sink is process-global, so a single test owns it for the whole binary.
#[test]
fn out_writes_the_body_to_a_file_and_reports_the_line_count() {
    use nostaro::outln;
    use nostaro::output::{self, OutFormat};

    let dir = scratch("output_sink");
    let text_path = dir.join("body.txt");
    std::fs::write(&text_path, "stale content that must be overwritten").unwrap();

    output::configure(Some(text_path.clone()), OutFormat::Text);
    assert!(!output::is_json());
    outln!("first").unwrap();
    outln!("second {}", 2).unwrap();
    output::finish().unwrap();

    assert_eq!(
        std::fs::read_to_string(&text_path).unwrap(),
        "first\nsecond 2\n"
    );

    // JSON mode: the document replaces the text rendering entirely.
    let json_path = dir.join("body.json");
    output::configure(Some(json_path.clone()), OutFormat::Json);
    assert!(output::is_json());
    outln!("this text body is dropped in json mode").unwrap();
    output::write_json(&serde_json::json!({"count": 1, "users": ["npub1x"]})).unwrap();
    output::finish().unwrap();

    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json_path).unwrap()).unwrap();
    assert_eq!(written["count"], 1);
    assert_eq!(written["users"][0], "npub1x");

    // A supported command whose result is empty still creates the file, so
    // "no results" is distinguishable from "this command ignores --out".
    let empty_path = dir.join("empty.txt");
    output::configure(Some(empty_path.clone()), OutFormat::Text);
    output::open_body().unwrap();
    output::finish().unwrap();
    assert_eq!(std::fs::read_to_string(&empty_path).unwrap(), "");

    // Backstop for a supported command that emitted nothing structured: it must
    // say so instead of leaving an empty file that looks like a valid result.
    // (The CLI itself refuses --out-format json on unsupported commands before
    // they run, so this can only be reached by a bug.)
    let unsupported = dir.join("unsupported.json");
    output::configure(Some(unsupported.clone()), OutFormat::Json);
    outln!("text only").unwrap();
    let err = output::finish().unwrap_err().to_string();
    assert!(err.contains("produced no JSON output"), "{}", err);
    assert!(!unsupported.exists());

    // No --out: nothing is created, output goes back to stdout.
    output::configure(None, OutFormat::Text);
    outln!("back on stdout").unwrap();
    output::finish().unwrap();
}

// ── CLI parsing ──────────────────────────────────────────────────────

use clap::Parser;
use nostr_sdk::prelude::ToBech32;

#[derive(Parser, Debug)]
#[command(name = "nostaro")]
struct TestCli {
    #[command(subcommand)]
    command: TestCommands,
}

#[derive(clap::Subcommand, Debug)]
enum TestCommands {
    Init,
    Post {
        message: String,
    },
    Reply {
        note_id: String,
        message: String,
    },
    Repost {
        note_id: String,
    },
    Timeline {
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
    Search {
        query: String,
    },
    Profile {
        #[command(subcommand)]
        action: TestProfileAction,
    },
    Follow {
        npub: String,
    },
    Unfollow {
        npub: String,
    },
    Following,
    React {
        note_id: String,
        #[arg(default_value = "\u{26A1}")]
        emoji: String,
    },
    Dm {
        #[command(subcommand)]
        action: TestDmAction,
    },
    Zap {
        target: String,
        amount: u64,
        #[arg(short, long)]
        message: Option<String>,
    },
    Channel {
        #[command(subcommand)]
        action: TestChannelAction,
    },
    Upload {
        file: String,
    },
    Relay {
        #[command(subcommand)]
        action: TestRelayAction,
    },
}

#[derive(clap::Subcommand, Debug)]
enum TestProfileAction {
    Show {
        #[arg(short = 'p', long)]
        pubkey: Option<String>,
    },
    Set {
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        display_name: Option<String>,
        #[arg(long)]
        about: Option<String>,
        #[arg(long)]
        picture: Option<String>,
    },
}

#[derive(clap::Subcommand, Debug)]
enum TestDmAction {
    Send { npub: String, message: String },
    Read { npub: Option<String> },
}

#[derive(clap::Subcommand, Debug)]
enum TestChannelAction {
    List,
    Read { id: String },
    Post { id: String, message: String },
}

#[derive(clap::Subcommand, Debug)]
enum TestRelayAction {
    Add { url: String },
    Remove { url: String },
    List,
}

#[test]
fn cli_parse_init() {
    let cli = TestCli::try_parse_from(["nostaro", "init"]).unwrap();
    assert!(matches!(cli.command, TestCommands::Init));
}

#[test]
fn cli_parse_post() {
    let cli = TestCli::try_parse_from(["nostaro", "post", "Hello Nostr!"]).unwrap();
    match cli.command {
        TestCommands::Post { message } => assert_eq!(message, "Hello Nostr!"),
        _ => panic!("Expected Post command"),
    }
}

#[test]
fn cli_parse_reply() {
    let cli = TestCli::try_parse_from(["nostaro", "reply", "note1abc", "Hello reply!"]).unwrap();
    match cli.command {
        TestCommands::Reply { note_id, message } => {
            assert_eq!(note_id, "note1abc");
            assert_eq!(message, "Hello reply!");
        }
        _ => panic!("Expected Reply command"),
    }
}

#[test]
fn cli_parse_repost() {
    let cli = TestCli::try_parse_from(["nostaro", "repost", "note1abc"]).unwrap();
    match cli.command {
        TestCommands::Repost { note_id } => assert_eq!(note_id, "note1abc"),
        _ => panic!("Expected Repost command"),
    }
}

#[test]
fn cli_parse_timeline_default_limit() {
    let cli = TestCli::try_parse_from(["nostaro", "timeline"]).unwrap();
    match cli.command {
        TestCommands::Timeline { limit } => assert_eq!(limit, 20),
        _ => panic!("Expected Timeline command"),
    }
}

#[test]
fn cli_parse_timeline_custom_limit() {
    let cli = TestCli::try_parse_from(["nostaro", "timeline", "--limit", "50"]).unwrap();
    match cli.command {
        TestCommands::Timeline { limit } => assert_eq!(limit, 50),
        _ => panic!("Expected Timeline command"),
    }
}

#[test]
fn cli_parse_search() {
    let cli = TestCli::try_parse_from(["nostaro", "search", "bitcoin"]).unwrap();
    match cli.command {
        TestCommands::Search { query } => assert_eq!(query, "bitcoin"),
        _ => panic!("Expected Search command"),
    }
}

#[test]
fn cli_parse_profile_show_no_pubkey() {
    let cli = TestCli::try_parse_from(["nostaro", "profile", "show"]).unwrap();
    match cli.command {
        TestCommands::Profile { action } => match action {
            TestProfileAction::Show { pubkey } => assert!(pubkey.is_none()),
            _ => panic!("Expected Show action"),
        },
        _ => panic!("Expected Profile command"),
    }
}

#[test]
fn cli_parse_profile_show_with_pubkey() {
    let cli = TestCli::try_parse_from(["nostaro", "profile", "show", "-p", "npub1abc123"]).unwrap();
    match cli.command {
        TestCommands::Profile { action } => match action {
            TestProfileAction::Show { pubkey } => assert_eq!(pubkey.unwrap(), "npub1abc123"),
            _ => panic!("Expected Show action"),
        },
        _ => panic!("Expected Profile command"),
    }
}

#[test]
fn cli_parse_profile_set_all_fields() {
    let cli = TestCli::try_parse_from([
        "nostaro",
        "profile",
        "set",
        "--name",
        "test",
        "--display-name",
        "Test User",
        "--about",
        "A test bio",
        "--picture",
        "https://example.com/pic.png",
    ])
    .unwrap();
    match cli.command {
        TestCommands::Profile { action } => match action {
            TestProfileAction::Set {
                name,
                display_name,
                about,
                picture,
            } => {
                assert_eq!(name.unwrap(), "test");
                assert_eq!(display_name.unwrap(), "Test User");
                assert_eq!(about.unwrap(), "A test bio");
                assert_eq!(picture.unwrap(), "https://example.com/pic.png");
            }
            _ => panic!("Expected Set action"),
        },
        _ => panic!("Expected Profile command"),
    }
}

#[test]
fn cli_parse_follow() {
    let cli = TestCli::try_parse_from(["nostaro", "follow", "npub1abc123"]).unwrap();
    match cli.command {
        TestCommands::Follow { npub } => assert_eq!(npub, "npub1abc123"),
        _ => panic!("Expected Follow command"),
    }
}

#[test]
fn cli_parse_unfollow() {
    let cli = TestCli::try_parse_from(["nostaro", "unfollow", "npub1abc123"]).unwrap();
    match cli.command {
        TestCommands::Unfollow { npub } => assert_eq!(npub, "npub1abc123"),
        _ => panic!("Expected Unfollow command"),
    }
}

#[test]
fn cli_parse_following() {
    let cli = TestCli::try_parse_from(["nostaro", "following"]).unwrap();
    assert!(matches!(cli.command, TestCommands::Following));
}

#[test]
fn cli_parse_react_default_emoji() {
    let cli = TestCli::try_parse_from(["nostaro", "react", "abc123"]).unwrap();
    match cli.command {
        TestCommands::React { note_id, emoji } => {
            assert_eq!(note_id, "abc123");
            assert_eq!(emoji, "\u{26A1}");
        }
        _ => panic!("Expected React command"),
    }
}

#[test]
fn cli_parse_react_custom_emoji() {
    let cli = TestCli::try_parse_from(["nostaro", "react", "abc123", "+"]).unwrap();
    match cli.command {
        TestCommands::React { note_id, emoji } => {
            assert_eq!(note_id, "abc123");
            assert_eq!(emoji, "+");
        }
        _ => panic!("Expected React command"),
    }
}

#[test]
fn cli_parse_dm_send() {
    let cli = TestCli::try_parse_from(["nostaro", "dm", "send", "npub1abc", "Hello DM!"]).unwrap();
    match cli.command {
        TestCommands::Dm { action } => match action {
            TestDmAction::Send { npub, message } => {
                assert_eq!(npub, "npub1abc");
                assert_eq!(message, "Hello DM!");
            }
            _ => panic!("Expected Send action"),
        },
        _ => panic!("Expected Dm command"),
    }
}

#[test]
fn cli_parse_dm_read_no_filter() {
    let cli = TestCli::try_parse_from(["nostaro", "dm", "read"]).unwrap();
    match cli.command {
        TestCommands::Dm { action } => match action {
            TestDmAction::Read { npub } => assert!(npub.is_none()),
            _ => panic!("Expected Read action"),
        },
        _ => panic!("Expected Dm command"),
    }
}

#[test]
fn cli_parse_dm_read_with_filter() {
    let cli = TestCli::try_parse_from(["nostaro", "dm", "read", "npub1abc"]).unwrap();
    match cli.command {
        TestCommands::Dm { action } => match action {
            TestDmAction::Read { npub } => assert_eq!(npub.unwrap(), "npub1abc"),
            _ => panic!("Expected Read action"),
        },
        _ => panic!("Expected Dm command"),
    }
}

#[test]
fn cli_parse_zap() {
    let cli = TestCli::try_parse_from(["nostaro", "zap", "npub1abc", "1000"]).unwrap();
    match cli.command {
        TestCommands::Zap {
            target,
            amount,
            message,
        } => {
            assert_eq!(target, "npub1abc");
            assert_eq!(amount, 1000);
            assert!(message.is_none());
        }
        _ => panic!("Expected Zap command"),
    }
}

#[test]
fn cli_parse_zap_with_message() {
    let cli = TestCli::try_parse_from(["nostaro", "zap", "npub1abc", "2100", "-m", "Great post!"])
        .unwrap();
    match cli.command {
        TestCommands::Zap {
            target,
            amount,
            message,
        } => {
            assert_eq!(target, "npub1abc");
            assert_eq!(amount, 2100);
            assert_eq!(message.unwrap(), "Great post!");
        }
        _ => panic!("Expected Zap command"),
    }
}

#[test]
fn cli_parse_channel_list() {
    let cli = TestCli::try_parse_from(["nostaro", "channel", "list"]).unwrap();
    match cli.command {
        TestCommands::Channel { action } => {
            assert!(matches!(action, TestChannelAction::List));
        }
        _ => panic!("Expected Channel command"),
    }
}

#[test]
fn cli_parse_channel_read() {
    let cli = TestCli::try_parse_from(["nostaro", "channel", "read", "abc123"]).unwrap();
    match cli.command {
        TestCommands::Channel { action } => match action {
            TestChannelAction::Read { id } => assert_eq!(id, "abc123"),
            _ => panic!("Expected Read action"),
        },
        _ => panic!("Expected Channel command"),
    }
}

#[test]
fn cli_parse_channel_post() {
    let cli = TestCli::try_parse_from(["nostaro", "channel", "post", "abc123", "Hello channel!"])
        .unwrap();
    match cli.command {
        TestCommands::Channel { action } => match action {
            TestChannelAction::Post { id, message } => {
                assert_eq!(id, "abc123");
                assert_eq!(message, "Hello channel!");
            }
            _ => panic!("Expected Post action"),
        },
        _ => panic!("Expected Channel command"),
    }
}

#[test]
fn cli_parse_upload() {
    let cli = TestCli::try_parse_from(["nostaro", "upload", "photo.jpg"]).unwrap();
    match cli.command {
        TestCommands::Upload { file } => assert_eq!(file, "photo.jpg"),
        _ => panic!("Expected Upload command"),
    }
}

#[test]
fn cli_parse_relay_add() {
    let cli = TestCli::try_parse_from(["nostaro", "relay", "add", "wss://relay.damus.io"]).unwrap();
    match cli.command {
        TestCommands::Relay { action } => match action {
            TestRelayAction::Add { url } => assert_eq!(url, "wss://relay.damus.io"),
            _ => panic!("Expected Add action"),
        },
        _ => panic!("Expected Relay command"),
    }
}

#[test]
fn cli_parse_relay_remove() {
    let cli =
        TestCli::try_parse_from(["nostaro", "relay", "remove", "wss://relay.damus.io"]).unwrap();
    match cli.command {
        TestCommands::Relay { action } => match action {
            TestRelayAction::Remove { url } => assert_eq!(url, "wss://relay.damus.io"),
            _ => panic!("Expected Remove action"),
        },
        _ => panic!("Expected Relay command"),
    }
}

#[test]
fn cli_parse_relay_list() {
    let cli = TestCli::try_parse_from(["nostaro", "relay", "list"]).unwrap();
    match cli.command {
        TestCommands::Relay { action } => {
            assert!(matches!(action, TestRelayAction::List));
        }
        _ => panic!("Expected Relay command"),
    }
}

#[test]
fn cli_parse_unknown_command_fails() {
    let result = TestCli::try_parse_from(["nostaro", "unknown"]);
    assert!(result.is_err());
}

#[test]
fn cli_parse_post_missing_message_fails() {
    let result = TestCli::try_parse_from(["nostaro", "post"]);
    assert!(result.is_err());
}
