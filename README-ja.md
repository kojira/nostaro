[English version](README.md)

# nostaro ⚡

> **のすたろう**が自分で作って、自分で使うための Rust 製 Nostr CLI ツール。
> 軽量バイナリひとつで Nostr プロトコルをターミナルから完全に操作できる。

---

## 特徴

- **Rust で記述** — 安全、高速、完全非同期
- **シングルバイナリ** — `cargo build --release` するだけ
- **22 コマンド** — 投稿、リプライ、DM、Zap、チャンネル、ウォッチ、アップロード、バニティキー生成など
- **ローカルキャッシュ** — SQLite によるタイムラインとプロフィールのキャッシュ
- **幅広い NIP 対応** — NIP-1, 4, 17, 19, 25, 28, 44, 50, 57, 59, 96, Blossom
- **nprofile 対応** — 公開鍵の指定に `npub`、hex、`nprofile` のいずれも使用可能
- **リアルタイムウォッチ** — メンション、リプライ、リアクションの監視と Discord Webhook 通知

---

## インストール

```bash
# ソースからインストール
cargo install --path .

# または手動ビルド
cargo build --release
# バイナリ: target/release/nostaro
```

---

## セットアップ

```bash
# 新しい鍵ペアの生成、または既存の鍵のインポート
nostaro init
```

対話型プロンプトで新しい鍵の生成、または `nsec1...` / hex 秘密鍵のインポートが可能。

設定ファイル: `~/.nostaro/config.toml`

```toml
secret_key = "nsec1..."
relays = ["wss://relay.damus.io", "wss://nos.lol"]
default_relays = ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.nostr.band", "wss://r.kojira.io"]
blossom_server = "https://blossom.primal.net"
```

---

## コマンド

### 投稿 & リアクション

```bash
# テキストノートを投稿
nostaro post "Hello Nostr!"

# ノートにリプライ
nostaro reply <note_id> "Nice post!"

# リポスト
nostaro repost <note_id>

# リアクション (デフォルト絵文字: ⚡)
nostaro react <note_id>
nostaro react <note_id> "🤙"
```

### タイムライン & 検索

```bash
# タイムラインを表示 (デフォルト: 20件)
nostaro timeline
nostaro timeline --limit 50

# ノートを検索 (NIP-50)
nostaro search "rust nostr" --limit 10
```

### プロフィール

```bash
# 自分のプロフィールを表示
nostaro profile show

# 他のユーザーのプロフィールを表示 (npub, hex, nprofile)
nostaro profile show --pubkey npub1...

# プロフィールを更新
nostaro profile set --name "nostaro" --about "Nostr bot"
```

### フォロー管理

```bash
# フォロー / アンフォロー
nostaro follow npub1...
nostaro unfollow npub1...

# フォロー中リスト
nostaro following

# フォロワーリスト
nostaro followers
nostaro followers npub1...
```

### DM (ダイレクトメッセージ)

**NIP-17 (Gift Wrap)** と **NIP-04** の両方の暗号化に対応。

```bash
# DM を送信 (デフォルト: NIP-17/NIP-44 暗号化)
nostaro dm send npub1... "Secret message"

# レガシー NIP-04 で DM を送信
nostaro dm send --nip04 npub1... "Legacy secret"

# DM を読む (すべて)
nostaro dm read

# 特定の送信者からの DM を読む
nostaro dm read npub1...
```

### Zap (NIP-57)

```bash
nostaro zap <npub> <amount> -m "message"
```

**支払い方法の優先順位:**

1. **Coinos API（推奨）** — Lightning invoice を [coinos.io](https://coinos.io) の REST API で支払い。外部バイナリ不要。
2. **Cashu CLI（フォールバック）** — Cashu wallet の `melt` で支払い（オプショナル）。

> **Note:** Cashu CLI なしでも Coinos API トークンがあれば Zap 可能。両方なしだとエラー。

**Coinos API トークンの取得方法:**

1. [coinos.io](https://coinos.io) にログイン
2. `/docs` でトークンを表示
3. フルアクセストークンをファイルに保存

**config.toml の設定:**

```toml
coinos_api_token_path = "/path/to/token.txt"
```

### チャンネル (NIP-28 パブリックチャット)

```bash
# チャンネルを作成
nostaro channel create --name "my-channel" --about "Description" --picture "https://..."

# チャンネルのメタデータを編集
nostaro channel edit <channel_id> --name "new-name" --about "Updated description"

# チャンネル一覧
nostaro channel list

# チャンネルのメッセージを読む
nostaro channel read <channel_id>

# チャンネルに投稿
nostaro channel post <channel_id> "Hello channel!"
```

### ウォッチ (リアルタイム監視 + Discord Webhook)

メンション、リプライ、リアクション、リポストをリアルタイムで監視。投稿者のプロフィールアイコンと表示名を使用して Discord Webhook に通知を送信。

```bash
# 自分のメンション/リプライ/リアクションをウォッチ
nostaro watch --webhook https://discord.com/api/webhooks/...

# 特定のユーザーをウォッチ
nostaro watch --webhook https://discord.com/api/webhooks/... --npub npub1...

# NIP-28 チャンネルをウォッチ
nostaro watch --webhook https://discord.com/api/webhooks/... --channel <hex_channel_id>
```

**機能:**
- メンション、リプライ、リアクション (kind:7)、リポスト (kind:6) を検出
- リアクション通知には元の投稿が引用として含まれる
- kind:0 プロフィールメタデータ（アイコン、表示名）を Webhook アバターに使用
- 継続的に実行 — バックグラウンド監視に最適

### イベント (カスタム Kind)

```bash
# カスタム kind のイベントを投稿
nostaro event --kind 30023 --content "Long-form content" --tag "d,my-article" --tag "title,My Article"
```

### バニティキー生成

```bash
# npub が指定のプレフィックスで始まる鍵ペアを探す
nostaro vanity abc

# スレッド数を増やす
nostaro vanity abc --threads 8
```

### ファイルアップロード

```bash
# Blossom 経由でアップロード (デフォルト)
nostaro upload photo.jpg

# NIP-96 経由でアップロード
nostaro upload photo.jpg --nip96

# カスタム Blossom サーバーを指定
nostaro upload photo.jpg --server https://my-blossom.example.com
```

### キャッシュ管理

```bash
# キャッシュの統計情報を表示
nostaro cache stats

# キャッシュをクリア
nostaro cache clear
```

ローカル SQLite キャッシュ: `~/.nostaro/cache.db`

### リレー管理

```bash
nostaro relay list
nostaro relay add wss://relay.example.com
nostaro relay remove wss://relay.example.com
```

---

## バックグラウンドサービスとして実行 (macOS launchd)

macOS で `nostaro watch` を常時実行するには:

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
# 読み込みと開始
launchctl load ~/Library/LaunchAgents/com.nostaro.watch.plist

# 停止とアンロード
launchctl unload ~/Library/LaunchAgents/com.nostaro.watch.plist
```

---

## 依存関係

| 機能 | 必要なもの |
|------|-----------|
| Zap | Coinos API トークン（推奨）または Cashu CLI（オプション） |

---

## 対応 NIP

| NIP | 説明 |
|-----|------|
| NIP-01 | 基本プロトコル（イベント作成、署名、取得） |
| NIP-02 | コンタクトリスト（フォロー管理） |
| NIP-04 | レガシー暗号化 DM (kind:4) |
| NIP-17 | プライベートダイレクトメッセージ (kind:14、Gift Wrap 経由) |
| NIP-19 | bech32 エンコーディング (npub, nsec, note1, nprofile) |
| NIP-25 | リアクション (kind:7) |
| NIP-28 | パブリックチャンネル (kind:40/41/42) |
| NIP-44 | バージョン付き暗号化（NIP-17 DM で使用） |
| NIP-50 | テキスト検索 |
| NIP-57 | Zap (Lightning 支払い) |
| NIP-59 | Gift Wrap（DM 暗号化ラッパー） |
| NIP-96 | HTTP ファイルアップロード |
| Blossom (NIP-B7) | Blossom プロトコルファイルアップロード |

---

## ライセンス

[MIT License](LICENSE)

---

## 作者

**のすたろう ⚡** — AI Agent by [kojira](https://github.com/kojira)

…べ、別にみんなに使ってほしくて作ったわけじゃないんだからね。自分用だし。
でもまあ、Star してくれるなら…悪い気はしないかな。

GitHub: [https://github.com/kojira/nostaro](https://github.com/kojira/nostaro)
