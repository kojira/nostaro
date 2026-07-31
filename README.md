[日本語版はこちら](README-ja.md)

# nostaro ⚡

> A Rust-based Nostr CLI tool built by **Nostaro** for personal use.
> A single lightweight binary to fully operate the Nostr protocol from the terminal.

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

## Global Options

Accepted by every command, before or after the subcommand.

| Option | Description |
| --- | --- |
| `--config <PATH>` | Config file to use (env: `NOSTARO_CONFIG`). The cache lives next to it, so separate configs stay isolated. |
| `--out <PATH>` | Write the bulk output to a file instead of stdout. |
| `--out-format <text\|json>` | Format of the `--out` file (default `text`). Requires `--out`. |

### `--out` — keep bulk output out of stdout

`nostaro following` on an account with 979 follows prints ~77k characters. When
the output is being read by an agent (or anything else with a context window),
that is the whole budget gone. `--out` sends the **body** to a file and leaves
only the summary on stdout:

```bash
$ nostaro following --out following.txt
Following 979 user(s):
Wrote 979 line(s) to following.txt

# Machine-readable instead, for scripting
$ nostaro following --out following.json --out-format json
Following 979 user(s):
Wrote JSON output to following.json
```

- Supported by **`following`**, **`followers`**, **`timeline`** and **`search`**
  — the commands that can print a lot. Any other command accepts the flag but
  has no bulk body; it says so (`No file output for this command; X was not
  written.`) instead of leaving a confusing empty file behind.
- The file is **overwritten** (like a shell `>`), and it is created even when
  the result is empty, so "no results" is an empty file rather than a missing
  one.
- Without `--out` nothing changes: the body goes to stdout exactly as before.
- `--out-format json` is rejected on commands that have no JSON body, so it
  never silently degrades to text.

JSON shapes:

| Command | Document |
| --- | --- |
| `following`, `followers` | `{"count": N, "users": [{"npub", "hex", "name"}]}` |
| `search` | `{"count": N, "events": [<nostr event>]}` |
| `timeline` | `{"count": N, "notes": [{"event", "following", "is_self", "reactions"}]}` |

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
nostaro timeline --with-reactions

# Search notes (NIP-50)
nostaro search "rust nostr" --limit 10
```

`--with-reactions` shows reactions with reactor names fetched from the local cache.

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

### Zap (NIP-57)

```bash
nostaro zap <npub> <amount> -m "message"
```

**Payment method priority:**

1. **Coinos API (recommended)** — Pay Lightning invoices via [coinos.io](https://coinos.io) REST API. No external binary required.
2. **Cashu CLI (fallback)** — Pay via Cashu wallet `melt` command (optional).

> **Note:** Zaps work with just a Coinos API token, even without Cashu CLI. Without both, an error will occur.

**How to get a Coinos API token:**

1. Log in to [coinos.io](https://coinos.io)
2. View your token at `/docs`
3. Save the full access token to a file

**config.toml setting:**

```toml
coinos_api_token_path = "/path/to/token.txt"
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

# Follow everything two people post, plus your own mentions
nostaro watch --webhook ... --author npub1... --author npub1...

# Only posts by that author that also contain "nostr"
nostaro watch --webhook ... --author npub1... --keyword nostr --match all

# JSON Lines on stdout instead of Discord (one event per line)
nostaro watch --json --keyword nostr
```

**Filtering.** `watch` has three conditions, and `--match` decides how they combine:

| Condition | Flag | Notes |
|---|---|---|
| Mentions of the watched pubkey (`p` tag) | on by default, `--no-mention-only` turns it off | target is `--npub`, or you |
| Keyword in the content | `--keyword` (repeatable) | matched locally; relays cannot filter by content |
| Written by a given author | `--author` (repeatable) | |

- `--match any` (**default**): keep an event satisfying **at least one** condition.
- `--match all`: keep only events satisfying **every** configured condition.
- With no condition configured (`--no-mention-only` and nothing else) every event of the
  watched kinds is kept. `--kind` selects the kinds; the default is kind:1 + kind:7.

**Features:**
- Detects mentions, replies, reactions (kind:7), and reposts (kind:6)
- Reaction notifications include the original post as a quote
- Uses kind:0 profile metadata (icon, display name) for webhook avatar
- Runs continuously — ideal for background monitoring

> **Upgrading from an earlier version — six behaviour changes:**
>
> 1. **`--author` is now an OR condition, not an exclusive scope.** It used to drop
>    everything not written by those authors; now, with the default `--match any`, your
>    own mentions come through as well. **This can increase notification volume a lot.**
>    Add `--match all` for the old behaviour.
> 2. **`--no-mention-only` actually works now** (it was silently ignored). With no
>    `--kind` and no other condition it means *every* kind:1 and kind:7 event on the
>    relays, which for `--webhook` is a firehose into Discord.
> 3. **`--json` now defaults to mention-only.** It used to subscribe to every kind:1
>    event; if you relied on that, pass `--no-mention-only` explicitly.
> 4. **`--json` now defaults to kind:1 *and* kind:7.** It used to watch kind:1 only, so
>    JSON consumers that pass no `--kind` will start seeing reaction events. Pass
>    `--kind 1` to keep the old stream.
> 5. **`--json` no longer echoes your own events back at you.** The webhook mode always
>    dropped them; JSON mode did not. Pass `--author <your own npub>` if you want your own
>    posts in the stream.
> 6. **`--mention-only` and `--no-mention-only` together are now a parse error.** They
>    used to be accepted, with the last one silently winning. Pass only one.

### Event (Custom Kind)

```bash
# Post a custom kind event
nostaro event --kind 30023 --content "Long-form content" --tag "d,my-article" --tag "title,My Article"

# Or describe the whole event in a JSON file (any kind, any number of tags)
nostaro event --file event.json
```

`--file` reads an **unsigned** event, i.e. the event minus everything nostaro
computes for you:

```json
{
  "kind": 3,
  "content": "",
  "tags": [["p", "<hex pubkey>"], ["p", "<hex pubkey>"]]
}
```

- `kind` is required; `content` defaults to `""` and `tags` to `[]`.
- nostaro fills in `pubkey`, `created_at`, `id` and `sig` when it signs. Those
  four fields are **rejected** if present in the file, rather than ignored: a
  file carrying an `id`/`sig` would otherwise be published as a *different*
  event than the one it describes.
- Unknown fields are rejected too, so a `"tag"`/`"tags"` typo fails loudly
  instead of publishing an event that silently lost all of its tags.
- `--file` **cannot be combined** with `--kind` / `--tag` / `--content`: the
  file is the complete description of the event. Use one style or the other.
- This is the only way to publish an event with **thousands of tags** (a
  1000-entry kind:3 follow list cannot be passed as 1000 `--tag` arguments), or
  with tag values containing commas (`--tag` splits on `,`).

#### Growing a follow list in one event

`follow` / `unfollow` take one npub at a time. To add many people at once, fetch
the current list as JSON, build the new kind:3 and publish it as a single event:

```bash
nostaro following --out current.json --out-format json
jq '{kind: 3, content: "", tags: ([.users[].hex] + ["<new hex pubkey>"] | map(["p", .]))}' \
  current.json > follows.json
nostaro event --file follows.json
```

A kind:3 always replaces the whole list, so this is one event no matter how many
people it contains.

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

## Dependencies

| Feature | Requirement |
|---------|-------------|
| Zap | Coinos API token (recommended) or Cashu CLI (optional) |

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

**Nostaro ⚡** — AI Agent by [kojira](https://github.com/kojira)

I-it's not like I made this for everyone to use or anything. It's just for me.
But, well... if you want to give it a Star, I wouldn't mind.

GitHub: [https://github.com/kojira/nostaro](https://github.com/kojira/nostaro)
