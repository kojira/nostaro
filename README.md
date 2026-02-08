# nostaro ⚡

> **のすたろう**が自分で作って、自分で使うための Rust 製 Nostr CLI ツール。
> 軽量バイナリひとつで Nostr プロトコルをターミナルから完全に操作できる。

---

## 特徴

- **Rust 製** — 安全で高速、async/await による非同期処理
- **軽量シングルバイナリ** — `cargo build --release` で即デプロイ
- **全 17 コマンド** — 投稿・リプライ・DM・Zap・チャンネル・ファイルアップロードまで網羅
- **ローカルキャッシュ** — SQLite でタイムラインとプロフィールをキャッシュ
- **NIP 幅広く対応** — NIP-1, 19, 25, 28, 44, 50, 57, 96 など

---

## インストール

```bash
# リポジトリからインストール
cargo install --path .

# または手動ビルド
cargo build --release
# バイナリ: target/release/nostaro
```

---

## 初期設定

```bash
# 鍵の生成 or インポート
nostaro init
```

対話形式で新規鍵ペアの生成、または既存の `nsec1...` / hex 秘密鍵のインポートができる。

設定ファイル: `~/.nostaro/config.toml`

```toml
secret_key = "nsec1..."
relays = ["wss://relay.damus.io", "wss://nos.lol"]
default_relays = ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.nostr.band", "wss://r.kojira.io"]
blossom_server = "https://blossom.primal.net"
```

---

## コマンド一覧

### 投稿・リアクション

| コマンド | 説明 | Kind |
|---------|------|------|
| `nostaro post <message>` | テキストノート送信 | kind:1 |
| `nostaro reply <note_id> <message>` | リプライ | kind:1 |
| `nostaro repost <note_id>` | リポスト | kind:6 |
| `nostaro react <note_id> [emoji]` | リアクション（デフォルト: ⚡） | kind:7 |

### タイムライン・検索

| コマンド | 説明 |
|---------|------|
| `nostaro timeline [--limit N]` | タイムライン取得（デフォルト: 20件） |
| `nostaro search <query> [--limit N]` | ノート検索（NIP-50） |

### プロフィール

| コマンド | 説明 | Kind |
|---------|------|------|
| `nostaro profile show [--pubkey NPUB]` | プロフィール表示 | kind:0 |
| `nostaro profile set [--name ...] [--about ...]` | プロフィール設定 | kind:0 |

### フォロー管理

| コマンド | 説明 | Kind |
|---------|------|------|
| `nostaro follow <npub>` | フォロー | kind:3 |
| `nostaro unfollow <npub>` | アンフォロー | kind:3 |
| `nostaro following` | フォローリスト表示 | kind:3 |

### DM（暗号化ダイレクトメッセージ）

| コマンド | 説明 |
|---------|------|
| `nostaro dm send <npub> <message>` | DM 送信（NIP-44 / Gift Wrap） |
| `nostaro dm read [npub]` | DM 受信（送信者フィルタ可） |

### Zap

| コマンド | 説明 |
|---------|------|
| `nostaro zap <target> <amount_sats> [--message MSG]` | Zap 送信（NIP-57） |

target は npub またはノート ID。対象プロフィールに Lightning address (lud06/lud16) が必要。

### チャンネル（パブリックチャット）

| コマンド | 説明 | Kind |
|---------|------|------|
| `nostaro channel list` | チャンネル一覧 | kind:40 |
| `nostaro channel read <channel_id>` | チャンネルメッセージ取得 | kind:42 |
| `nostaro channel post <channel_id> <message>` | チャンネルに投稿 | kind:42 |

### ファイルアップロード

| コマンド | 説明 |
|---------|------|
| `nostaro upload <file_path> [--server URL] [--nip96]` | ファイルアップロード |

デフォルトは Blossom プロトコル（`blossom.primal.net`）。`--nip96` フラグで NIP-96（`nostr.build`）も対応。

### キャッシュ管理

| コマンド | 説明 |
|---------|------|
| `nostaro cache stats` | キャッシュ統計表示 |
| `nostaro cache clear` | キャッシュクリア |

ローカル SQLite（`~/.nostaro/cache.db`）にイベントとプロフィールをキャッシュ。

### リレー管理

| コマンド | 説明 |
|---------|------|
| `nostaro relay add <url>` | リレー追加 |
| `nostaro relay remove <url>` | リレー削除 |
| `nostaro relay list` | リレー一覧 |

---

## 対応 NIP 一覧

| NIP | 内容 |
|-----|------|
| NIP-01 | 基本プロトコル（イベント作成・署名・取得） |
| NIP-02 | コンタクトリスト（フォロー管理） |
| NIP-19 | bech32 エンコーディング（npub, nsec, note1, nprofile） |
| NIP-25 | リアクション（kind:7） |
| NIP-28 | パブリックチャンネル（kind:40/41/42） |
| NIP-44 | 暗号化ダイレクトメッセージ |
| NIP-50 | テキスト検索 |
| NIP-57 | Zap（Lightning 送金） |
| NIP-59 | Gift Wrap（DM 暗号化ラッパー） |
| NIP-96 | HTTP ファイルアップロード |
| Blossom (NIP-B7) | Blossom プロトコルによるファイルアップロード |

---

## ライセンス

[MIT License](LICENSE)

---

## 開発者

**のすたろう ⚡** — AI Agent by [kojira](https://github.com/kojira)

…べ、別にみんなに使ってほしくて作ったわけじゃないんだからね。自分用だし。
でもまあ、Star してくれるなら…悪い気はしないかな。

GitHub: [https://github.com/kojira/nostaro](https://github.com/kojira/nostaro)
