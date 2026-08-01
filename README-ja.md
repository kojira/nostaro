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

## グローバルオプション

すべてのコマンドで、サブコマンドの前後どちらでも指定できます。

| オプション | 説明 |
| --- | --- |
| `--config <PATH>` | 使用する設定ファイル (env: `NOSTARO_CONFIG`)。キャッシュも隣に置かれるため、設定ごとに独立します。 |
| `--out <PATH>` | 大量出力の本体を stdout ではなくファイルへ書き出します。 |
| `--out-format <text\|json>` | `--out` ファイルの形式 (既定 `text`)。`--out` が必須です。 |

### `--out` — 大量出力を stdout から追い出す

979 人フォローしているアカウントの `nostaro following` は約 65,000 文字を出力します。
出力をエージェント（あるいはコンテキスト長を持つ何か）が読む場合、それだけで予算を
使い切ってしまいます。`--out` は**本体**をファイルへ送り、stdout には要約だけを残します。

```bash
$ nostaro following --out following.txt
Following 979 user(s):
Wrote 979 line(s) to following.txt

# スクリプトから扱うなら機械可読な形式で
$ nostaro following --out following.json --out-format json
Following 979 user(s):
Wrote JSON output to following.json
```

- 対応コマンドは大量に出力しうる **`following`** / **`followers`** / **`timeline`** /
  **`search`**。他のコマンドもフラグ自体は受け付けますが本体を持たないため、
  紛らわしい空ファイルを残さずその旨を表示します
  (`No file output for this command; X was not written.`)。
- 対応コマンドはファイルを**上書き**します。結果が空でもファイルは作成されるので、
  「結果が 0 件」と「ファイルが無い」を取り違えません。truncate は書き込み開始時点な
  ので、**非対応**コマンドに既存ファイルを渡すとファイルは**触られず古い内容が残り**
  ます（今回の結果と誤読しないこと）。途中で失敗した場合はそこまでの分が残ります。
  ファイルを信用する前に終了ステータスを確認してください。
- 本体は取得完了後にまとめて書き出します（逐次ストリームではありません）。フォローの
  多いアカウントの `following` はしばらく無言のあと一気に書かれます。
- `--out` を付けなければ挙動は従来どおり（本体は stdout）です。
- JSON 本体を持たないコマンドの `--out-format json` は**コマンド実行前に**拒否されます。
  黙って text に落ちることも、副作用のあとで失敗することもありません。

JSON の形:

| コマンド | ドキュメント |
| --- | --- |
| `following`, `followers` | `{"count": N, "users": [{"npub", "hex"}]}` |
| `search` | `{"count": N, "events": [<nostr event>]}` |
| `timeline` | `{"count": N, "notes": [{"event", "following", "is_self", "reactions"}]}` |

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

`following` / `followers` が返すのは **npub だけ**で、表示名は出しません。
一覧そのものは kind:3 を 1 回読めば終わりますが、名前を付けるには 1 件ごとに
kind:0 を読む必要があり（979 フォローなら 979 往復）、しかもその出力はたいてい
スクリプトに流されます。名前が必要なときは明示的に引いてください。

```bash
nostaro profile show --pubkey <npub または hex>
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

# 特定の人の投稿を全部追う（自分宛のメンションも届く）
nostaro watch --webhook ... --author npub1... --author npub1...

# その人の投稿のうち "nostr" を含むものだけ
nostaro watch --webhook ... --author npub1... --keyword nostr --match all

# Discord ではなく stdout に JSON Lines（1行1イベント）
nostaro watch --json --keyword nostr
```

**フィルタ.** `watch` の条件は3つあり、`--match` で結合方法を選びます:

| 条件 | フラグ | 備考 |
|---|---|---|
| 監視対象宛のメンション（`p` タグ） | 既定で on、`--no-mention-only` で off | 対象は `--npub`、未指定なら自分 |
| 本文のキーワード | `--keyword`（複数可） | ローカル照合（リレーは content で絞れない） |
| 特定の author の投稿 | `--author`（複数可） | |

- `--match any`（**既定**）: **どれか1つ**満たせば拾う。
- `--match all`: **すべて**満たすものだけ拾う。
- 条件を1つも指定しない場合（`--no-mention-only` のみ等）は、対象 kind の全イベントを拾います。
  kind は `--kind` で指定し、既定は kind:1 + kind:7 です。

**機能:**
- メンション、リプライ、リアクション (kind:7)、リポスト (kind:6) を検出
- リアクション通知には元の投稿が引用として含まれる
- kind:0 プロフィールメタデータ（アイコン、表示名）を Webhook アバターに使用
- 継続的に実行 — バックグラウンド監視に最適

> **旧バージョンからの移行 — 挙動が6点変わりました:**
>
> 1. **`--author` が排他スコープから OR 条件に変わりました。** 以前は指定 author 以外を
>    すべて捨てていましたが、既定の `--match any` では自分宛のメンションも併せて届きます。
>    **通知量が大幅に増える可能性があります。** 旧挙動が必要なら `--match all` を付けてください。
> 2. **`--no-mention-only` が実際に効くようになりました**（従来は無視されていました）。
>    `--kind` も他の条件も無い状態で指定すると、リレー上の kind:1 / kind:7 が**全部**対象に
>    なります。`--webhook` では Discord に大量投稿が流れます。
> 3. **`--json` の既定が mention-only になりました。** 以前は kind:1 を全件購読していたので、
>    その挙動に依存していた場合は `--no-mention-only` を明示してください。
> 4. **`--json` の既定 kind が kind:1 + kind:7 になりました。** 以前は kind:1 のみだったため、
>    `--kind` を渡していない JSON 消費者にはリアクションイベントが流入します。従来どおりに
>    するには `--kind 1` を指定してください。
> 5. **`--json` でも自分のイベントが除外されるようになりました。** webhook では従来から
>    除外していましたが、JSON では返っていました。自分の投稿も取りたい場合は
>    `--author <自分の npub>` を指定してください。
> 6. **`--mention-only` と `--no-mention-only` の同時指定はパースエラーになりました。**
>    以前は後勝ちで受理されていました。どちらか一方だけを指定してください。

### イベント (カスタム Kind)

```bash
# カスタム kind のイベントを投稿
nostaro event --kind 30023 --content "Long-form content" --tag "d,my-article" --tag "title,My Article"

# イベント全体を JSON ファイルで渡す（任意の kind・任意の数のタグ）
nostaro event --file event.json
```

`--file` は**未署名**イベント、つまり nostaro が計算する項目を除いたイベントを読みます。

```json
{
  "kind": 3,
  "content": "",
  "tags": [["p", "<hex pubkey>"], ["p", "<hex pubkey>"]]
}
```

- `kind` は必須。`content` は既定で `""`、`tags` は既定で `[]`。
- 署名時に `pubkey` / `created_at` / `id` / `sig` は nostaro が埋めます。この 4 つが
  ファイルに含まれていたら**無視せずエラー**にします。`id`/`sig` を持つファイルを
  黙って受け取ると、書かれている内容とは**別のイベント**を発行することになるためです。
- 未知のフィールドもエラーです。`"tags"` を `"tag"` と書き間違えたときに、タグが
  すべて消えたイベントを黙って発行せず、その場で落ちます。
- `p` / `e` タグの**値**は発行前に検証し、**64 文字の hex** のみ受け付けます。
  `npub1…` を貼ったら「hex に変換せよ」というエラーになります。kind:3 はリスト全体を
  置き換えるうえ、壊れたタグはリレーにもクライアントにも無視されるため、その人だけ
  黙って欠けたフォローリストが出来上がるのを防ぐためです。
- タグの**形**については何も詮索しません。NIP-70 の `["-"]` のような名前だけのタグも、
  nostaro が知らないタグも、そのまま通ります。拒否されるのは完全に空のタグ（`[]`）
  だけです。
- 8 MiB を超えるファイルは拒否します（10 万タグでも十分下回ります）。
- `--file` は `--kind` / `--tag` / `--content` と**併用できません**。ファイルが
  イベントの完全な記述なので、どちらか一方のスタイルを使ってください。
- **数千個のタグ**を持つイベント（1000 件の kind:3 フォローリストを 1000 個の `--tag`
  引数で渡すのは非現実的）や、カンマを含むタグ値（`--tag` は `,` で分割する）を
  発行できるのはこの方法だけです。

#### フォローリストを 1 イベントで増やす

`follow` / `unfollow` は 1 回に 1 人だけです。大量に追加したいときは、現在のリストを
JSON で取得し、新しい kind:3 を組み立てて 1 イベントとして発行します。

```bash
nostaro following --out current.json --out-format json
jq '{kind: 3, content: "", tags: ([.users[].hex] + ["<new hex pubkey>"] | map(["p", .]))}' \
  current.json > follows.json
nostaro event --file follows.json
```

kind:3 は常にリスト全体を置き換えるため、何人分であってもイベントは 1 つです。
そしてリスト全体を置き換えるからこそ、`p` の値に hex でないものが 1 つでもあれば、
穴の空いたリストを発行せずファイルごと拒否します。

### 発行結果の確認

イベントを発行するコマンドは、どのリレーが受理したかを確認するようになりました。
拒否したリレーは stderr に警告として出力し、**どのリレーも受理しなかった場合は
成功と表示せずエラー終了**します。大きな kind:3 はリレーの event サイズ上限や
タグ数上限に触れうるため、この確認が効きます。

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
