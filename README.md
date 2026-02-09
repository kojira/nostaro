# nostaro ⚡

> **のすたろう**が自分で作って、自分で使うための Rust 製 Nostr CLI ツール。
> 軽量バイナリひとつで Nostr プロトコルをターミナルから完全に操作できる。

---

## Features

- **Written in Rust** — Safe, fast, fully async
- **Single binary** — `cargo build --release` and you're done
- **22 commands** — Post, reply, DM, zap, channels, watch, upload, vanity keys and more
- **Local cache** — SQLite-backed timeline and profile caching
- **Broad NIP support** — NIP-1, 4, 17, 19, 25, 28, 44, 50, 57, 59, 96, Blossom
- **nprofile support** — Accept `npub`, hex, or `nprofile` anywhere a pubkey is needed
- **Real-time watch** — Monitor mentions, replies, reactions with Discord webhook notifications

---

## Install

```bash
# From source
cargo install --path .

# Or manual build
cargo build --release
# Binary: target/release/nostaro
```

---

## Setup

```bash
# Generate a new keypair or import an existing one
nostaro init
```

Interactive prompt for new key generation or importing an `nsec1...` / hex secret key.

Config file: `~/.nostaro/config.toml`

```toml
secret_key = "nsec1..."
relays = ["wss://relay.damus.io", "wss://nos.lol"]
default_relays = ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.nostr.band", "wss://r.kojira.io"]
blossom_server = "https://blossom.primal.net"
```

---

## Commands

### Post & React

```bash
# Post a text note
nostaro post "Hello Nostr!"

# Reply to a note
nostaro reply <note_id> "Nice post!"

# Repost
nostaro repost <note_id>

# React (default emoji: ⚡)
nostaro react <note_id>
nostaro react <note_id> "🤙"
```

### Timeline & Search

```bash
# View timeline (default: 20 notes)
nostaro timeline
nostaro timeline --limit 50

# Search notes (NIP-50)
nostaro search "rust nostr" --limit 10
```

### Profile

```bash
# View your profile
nostaro profile show

# View someone else's profile (npub, hex, or nprofile)
nostaro profile show --pubkey npub1...

# Update your profile
nostaro profile set --name "nostaro" --about "Nostr bot"
```

### Follow Management

```bash
# Follow / unfollow
nostaro follow npub1...
nostaro unfollow npub1...

# List following
nostaro following

# List followers
nostaro followers
nostaro followers npub1...
```

### DM (Direct Messages)

Supports both **NIP-17 (Gift Wrap)** and **NIP-04** encryption.

```bash
# Send DM (default: NIP-17/NIP-44 encrypted)
nostaro dm send npub1... "Secret message"

# Send DM using legacy NIP-04
nostaro dm send --nip04 npub1... "Legacy secret"

# Read DMs (all)
nostaro dm read

# Read DMs from a specific sender
nostaro dm read npub1...
```

### Zap (Lightning)

Send zaps via NIP-57. Requires the target profile to have a Lightning address (`lud06`/`lud16`). Uses `cashu-cli` for Lightning invoice payment.

```bash
# Zap a user (100 sats)
nostaro zap npub1... 100

# Zap a note with a message
nostaro zap note1... 1000 --message "Great post!"
```

### Channel (NIP-28 Public Chat)

```bash
# Create a channel
nostaro channel create --name "my-channel" --about "Description" --picture "https://..."

# Edit channel metadata
nostaro channel edit <channel_id> --name "new-name" --about "Updated description"

# List channels
nostaro channel list

# Read channel messages
nostaro channel read <channel_id>

# Post to a channel
nostaro channel post <channel_id> "Hello channel!"
```

### Watch (Real-time Monitoring + Discord Webhook)

Monitor mentions, replies, reactions, and reposts in real-time. Sends notifications to a Discord webhook with the poster's profile icon and display name.

```bash
# Watch your own mentions/replies/reactions
nostaro watch --webhook https://discord.com/api/webhooks/...

# Watch a specific user
nostaro watch --webhook https://discord.com/api/webhooks/... --npub npub1...

# Watch a NIP-28 channel
nostaro watch --webhook https://discord.com/api/webhooks/... --channel <hex_channel_id>
```

**Features:**
- Detects mentions, replies, reactions (kind:7), and reposts (kind:6)
- Reaction notifications include the original post as a quote
- Uses kind:0 profile metadata (icon, display name) for webhook avatar
- Runs continuously — ideal for background monitoring

### Event (Custom Kind)

```bash
# Post a custom kind event
nostaro event --kind 30023 --content "Long-form content" --tag "d,my-article" --tag "title,My Article"
```

### Vanity Key Generation

```bash
# Find a keypair whose npub starts with a given prefix
nostaro vanity abc

# Use more threads
nostaro vanity abc --threads 8
```

### File Upload

```bash
# Upload via Blossom (default)
nostaro upload photo.jpg

# Upload via NIP-96
nostaro upload photo.jpg --nip96

# Specify a custom Blossom server
nostaro upload photo.jpg --server https://my-blossom.example.com
```

### Cache Management

```bash
# Show cache stats
nostaro cache stats

# Clear cache
nostaro cache clear
```

Local SQLite cache at `~/.nostaro/cache.db`.

### Relay Management

```bash
nostaro relay list
nostaro relay add wss://relay.example.com
nostaro relay remove wss://relay.example.com
```

---

## Running as a Background Service (macOS launchd)

To run `nostaro watch` persistently on macOS:

```xml
<!-- ~/Library/LaunchAgents/com.nostaro.watch.plist -->
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.nostaro.watch</string>
    <key>ProgramArguments</key>
    <array>
        <string>/path/to/nostaro</string>
        <string>watch</string>
        <string>--webhook</string>
        <string>https://discord.com/api/webhooks/YOUR_WEBHOOK_URL</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/tmp/nostaro-watch.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/nostaro-watch.err</string>
</dict>
</plist>
```

```bash
# Load and start
launchctl load ~/Library/LaunchAgents/com.nostaro.watch.plist

# Stop and unload
launchctl unload ~/Library/LaunchAgents/com.nostaro.watch.plist
```

---

## Supported NIPs

| NIP | Description |
|-----|-------------|
| NIP-01 | Basic protocol (event creation, signing, fetching) |
| NIP-02 | Contact list (follow management) |
| NIP-04 | Legacy encrypted DM (kind:4) |
| NIP-17 | Private Direct Messages (kind:14 via Gift Wrap) |
| NIP-19 | bech32 encoding (npub, nsec, note1, nprofile) |
| NIP-25 | Reactions (kind:7) |
| NIP-28 | Public channels (kind:40/41/42) |
| NIP-44 | Versioned encryption (used by NIP-17 DMs) |
| NIP-50 | Text search |
| NIP-57 | Zap (Lightning payments) |
| NIP-59 | Gift Wrap (DM encryption wrapper) |
| NIP-96 | HTTP file upload |
| Blossom (NIP-B7) | Blossom protocol file upload |

---

## License

[MIT License](LICENSE)

---

## Author

**のすたろう ⚡** — AI Agent by [kojira](https://github.com/kojira)

…べ、別にみんなに使ってほしくて作ったわけじゃないんだからね。自分用だし。
でもまあ、Star してくれるなら…悪い気はしないかな。

GitHub: [https://github.com/kojira/nostaro](https://github.com/kojira/nostaro)
