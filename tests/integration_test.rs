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
    };

    config.save_to(&path).unwrap();

    let loaded = nostaro::config::NostaroConfig::load_from(&path).unwrap();
    assert_eq!(loaded.secret_key, config.secret_key);
    assert_eq!(loaded.relays, config.relays);
    assert_eq!(loaded.default_relays, config.default_relays);

    // Clean up
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn config_load_nonexistent_returns_default() {
    let path = PathBuf::from("/tmp/nostaro_nonexistent_config_test.toml");
    // Ensure file does not exist
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

// ── CLI parsing ──────────────────────────────────────────────────────

use clap::Parser;

// Re-define the CLI types here since they are private in main.
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
    Timeline {
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
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
        "nostaro", "profile", "set",
        "--name", "test",
        "--display-name", "Test User",
        "--about", "A test bio",
        "--picture", "https://example.com/pic.png",
    ]).unwrap();
    match cli.command {
        TestCommands::Profile { action } => match action {
            TestProfileAction::Set { name, display_name, about, picture } => {
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
fn cli_parse_profile_set_partial_fields() {
    let cli = TestCli::try_parse_from([
        "nostaro", "profile", "set", "--name", "only-name",
    ]).unwrap();
    match cli.command {
        TestCommands::Profile { action } => match action {
            TestProfileAction::Set { name, display_name, about, picture } => {
                assert_eq!(name.unwrap(), "only-name");
                assert!(display_name.is_none());
                assert!(about.is_none());
                assert!(picture.is_none());
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

// We need the nostr_sdk prelude for bech32 methods
use nostr_sdk::prelude::ToBech32;
