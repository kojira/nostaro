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
    let mut config = nostaro::config::NostaroConfig::default();
    config.blossom_server = Some("https://custom.blossom.server".to_string());
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

    let mut config = nostaro::config::NostaroConfig::default();
    config.secret_key = Some(nsec);

    let loaded = nostaro::keys::keys_from_config(&config).unwrap();
    assert_eq!(loaded.public_key(), keys.public_key());
}

#[test]
fn keys_from_config_with_hex_key() {
    let keys = nostaro::keys::generate_keys();
    let hex_secret = keys.secret_key().to_secret_hex();

    let mut config = nostaro::config::NostaroConfig::default();
    config.secret_key = Some(hex_secret);

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
    Post { message: String },
    Reply { note_id: String, message: String },
    Repost { note_id: String },
    Timeline {
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
    Search { query: String },
    Profile {
        #[command(subcommand)]
        action: TestProfileAction,
    },
    Follow { npub: String },
    Unfollow { npub: String },
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
    Upload { file: String },
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
    let cli =
        TestCli::try_parse_from(["nostaro", "reply", "note1abc", "Hello reply!"]).unwrap();
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
    let cli =
        TestCli::try_parse_from(["nostaro", "profile", "show", "-p", "npub1abc123"]).unwrap();
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
    let cli =
        TestCli::try_parse_from(["nostaro", "dm", "send", "npub1abc", "Hello DM!"]).unwrap();
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
    let cli = TestCli::try_parse_from([
        "nostaro",
        "zap",
        "npub1abc",
        "2100",
        "-m",
        "Great post!",
    ])
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
    let cli =
        TestCli::try_parse_from(["nostaro", "channel", "post", "abc123", "Hello channel!"])
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
    let cli =
        TestCli::try_parse_from(["nostaro", "relay", "add", "wss://relay.damus.io"]).unwrap();
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
