# WATCH_REDESIGN.md — nostaro watch コマンド 柔軟フィルタ設計書

> 作成日: 2026-03-23  
> ステータス: 設計フェーズ（コード変更なし）

---

## 1. 現状の watch.rs 主要ロジック

### 1.1 シグネチャ

```rust
pub async fn run(
    webhook_url: &str,
    npub_str: Option<&str>,
    channel_id: Option<&str>,
    keywords: &[String],
) -> Result<()>
```

### 1.2 サブスクリプション構成（現行）

| # | 条件 | Nostr Filter | 用途 |
|---|------|-------------|------|
| A | `channel_id` が Some | `.kind(ChannelMessage).event(channel_id)` | NIP-28チャンネル監視 |
| B | `channel_id` が None または `npub_str` が Some | `.pubkey(target).kinds([TextNote(1), Reaction(7)])` | メンション・リプライ・リアクション監視 |
| C | `keywords` が空でない | `.kind(TextNote(1))` | キーワードマッチ（ローカルフィルタ） |

### 1.3 イベント処理ロジック

```
受信イベント
 ├─ Kind::ChannelMessage → チャンネルメッセージ通知
 ├─ Kind::TextNote (1)
 │   ├─ p-tag に自分の pubkey → リプライ/メンション通知
 │   └─ keywords マッチ → キーワード通知
 ├─ Kind::Reaction (7) → リアクション通知（元投稿内容もフェッチ）
 └─ それ以外 → `continue`（無視）
```

### 1.4 現行の制約

- kind のハードコード: `TextNote(1)`, `Reaction(7)`, `ChannelMessage(42)` のみ
- kind:9735 (Zap Receipt) の受信・解析は未実装
- サブスクリプションの kind を実行時に変更できない
- `--mention-only` という概念が CLI に存在しない（サブスクリプション B は常に p-tag フィルタ）

---

## 2. 追加する CLI オプション仕様

### 2.1 全オプション一覧（変更後）

```
nostaro watch [OPTIONS]

OPTIONS:
  --webhook <URL>         Discord webhook URL（必須）
  --npub <NPUB>           監視対象 pubkey（デフォルト: 自分）
  --channel <HEX>         NIP-28 チャンネル ID（hex）
  --keyword <KEYWORD>     監視キーワード（複数指定可）
  --kind <NUMBER>         追加監視する kind 番号（複数指定可） [新規]
  --mention-only          p-tag メンションのみ通知（デフォルト: true） [新規]
  --no-mention-only       メンション以外の kind も通知（--mention-only を無効化） [新規]
```

### 2.2 `--kind <numbers>`

| 項目 | 内容 |
|------|------|
| 型 | `String` → パース後 `Vec<u16>` |
| デフォルト | 空（指定なしの場合は従来の kind:1 + kind:7 を使用） |
| 複数指定 | `--kind 1,9735,7` のようにカンマ区切りで一括指定 |
| 動作 | 指定 kind を追加サブスクリプションとして登録 |
| 例外 | kind:9735 には専用パーサーを適用（3.1節参照） |

**サブコマンド定義（Rust clap）:**

```rust
/// Comma-separated event kinds to watch (e.g. --kind 1,9735,7)
#[arg(long = "kind", value_parser = parse_kinds)]
kinds: Vec<u16>,
```

**カスタムパーサー:**

```rust
fn parse_kinds(s: &str) -> Result<Vec<u16>, String> {
    s.split(',')
        .map(|k| k.trim().parse::<u16>().map_err(|e| format!("invalid kind '{}': {}", k, e)))
        .collect()
}
```

### 2.3 `--mention-only` / `--no-mention-only`

| 項目 | 内容 |
|------|------|
| 型 | `bool`（フラグ） |
| デフォルト | `true`（メンションのみ） |
| 動作（true） | サブスクリプション B に `.pubkey(target)` を付与し、p-tag メンションのみ受信 |
| 動作（false） | `.pubkey(target)` なし → 指定 kind のすべてのイベントを受信（量が多いため注意） |

**clap 定義案:**

```rust
/// Only receive events that mention (p-tag) the target pubkey (default: true)
#[arg(long, default_value_t = true)]
mention_only: bool,

/// Disable mention-only mode (receive all events of watched kinds)
#[arg(long, action = clap::ArgAction::SetFalse, overrides_with = "mention_only")]
no_mention_only: bool,
```

> **注意**: `clap` の `default_value_t = true` + `SetFalse` パターンで `--no-mention-only` を実装する。

---

## 3. kind:9735 (Zap Receipt) 対応時の特記事項

### 3.1 Zap Receipt の構造（NIP-57）

```json
{
  "kind": 9735,
  "tags": [
    ["p", "<recipient_pubkey>"],
    ["e", "<zapped_event_id>"],
    ["bolt11", "<lightning_invoice>"],
    ["description", "<zap_request_json>"]
  ],
  "content": ""
}
```

### 3.2 amount の取得方法

Zap Receipt から金額を取得するには **2段階** の処理が必要:

1. `bolt11` タグから BOLT11 invoice を取得
2. invoice をデコードして `amount` フィールド（ミリサトシ）を読む

**推奨ライブラリ**: `lightning-invoice` crate（既存 Cargo.toml に未追加の場合は追加必要）

```rust
use lightning_invoice::Bolt11Invoice;
use std::str::FromStr;

fn parse_zap_amount(event: &Event) -> Option<u64> {
    let bolt11 = event.tags.iter().find_map(|t| {
        if t.kind() == TagKind::custom("bolt11") {
            t.content()
        } else {
            None
        }
    })?;

    let invoice = Bolt11Invoice::from_str(bolt11).ok()?;
    invoice.amount_milli_satoshis().map(|msats| msats / 1000)
}
```

### 3.3 Zap Requestの description タグから追加情報取得

`description` タグには Zap Request イベント（kind:9734）の JSON が含まれる。これをパースすることで:
- Zap メッセージ（`content` フィールド）
- Zapper の pubkey
- どのイベントへの Zap か（`e` タグ）

を取得できる。

```rust
fn parse_zap_description(event: &Event) -> Option<(String, PublicKey)> {
    let description_json = event.tags.iter().find_map(|t| {
        if t.kind() == TagKind::custom("description") {
            t.content()
        } else {
            None
        }
    })?;

    let zap_request: serde_json::Value = serde_json::from_str(description_json).ok()?;
    let content = zap_request["content"].as_str()?.to_string();
    let pubkey_hex = zap_request["pubkey"].as_str()?;
    let pubkey = PublicKey::from_hex(pubkey_hex).ok()?;
    Some((content, pubkey))
}
```

### 3.4 Zap 通知フォーマット案

```
⚡ **{sender_name}** が {amount} sats をzapしました！
メッセージ: {zap_message}
npub: {sender_npub}
対象note: {zapped_note_id}
```

### 3.5 サブスクリプション設計

kind:9735 を `--kind 9735` で指定した場合、受信する側は **Zap Receipt** なので:
- `p` タグに監視対象 pubkey が含まれる → `--mention-only true` で自然にフィルタされる
- 追加のサブスクリプション: `.kinds([Kind::from(9735u16)]).pubkey(target_pubkey)`

---

## 4. フィルタロジックの変更方針

### 4.1 設計方針: 統合サブスクリプション vs 分離サブスクリプション

**採用: 分離サブスクリプション方式**

理由:
- 既存のチャンネル監視・キーワード監視・メンション監視はそれぞれ異なるフィルタ条件を持つ
- 統合すると OR 条件が複雑になり、受信後のローカルフィルタが増える
- nostr-sdk の `subscribe()` は複数回呼び出し可能で、内部でマージされる

### 4.2 新しいサブスクリプション構成

```
サブスクリプション構成（変更後）

[A] チャンネル監視（--channel 指定時）
    Filter: .kind(ChannelMessage).event(channel_id).since(now)

[B] デフォルトメンション監視（--kind 未指定 かつ channel のみでない場合）
    Filter: .pubkey(target).kinds([TextNote(1), Reaction(7)]).since(now)
    ※ --kind 指定時はこのサブスクリプションを省略し [C] に委ねる

[C] カスタム kind 監視（--kind 指定時）
    Filter base: .kinds([指定kinds]).since(now)
    + mention_only=true なら: .pubkey(target) を追加

[D] キーワード監視（--keyword 指定時）
    Filter: .kind(TextNote(1)).since(now)
    ※ ローカルでキーワードマッチ
```

### 4.3 イベント処理ルーティング（変更後）

```
受信イベント
 ├─ Kind::ChannelMessage(42) → チャンネルメッセージ処理（既存）
 ├─ Kind::TextNote(1)
 │   ├─ mention_only=true かつ p-tag に target → リプライ/メンション通知
 │   ├─ mention_only=false → 無条件通知
 │   └─ keywords マッチ → キーワード通知
 ├─ Kind::Reaction(7) → リアクション通知（既存）
 ├─ Kind::ZapReceipt(9735) → Zap通知（新規）
 └─ その他のカスタム kind → 汎用通知（kind番号とcontent表示）
```

### 4.4 run() シグネチャ変更案

```rust
pub async fn run(
    webhook_url: &str,
    npub_str: Option<&str>,
    channel_id: Option<&str>,
    keywords: &[String],
    extra_kinds: &[u16],      // 新規: --kind で指定されたkind番号
    mention_only: bool,        // 新規: --mention-only フラグ（デフォルト true）
) -> Result<()>
```

---

## 5. 実装ステップ

### Step 1: CLI オプション追加（main.rs）

```
Watch コマンドの定義に以下を追加:
  kinds: Vec<u16>      // --kind
  mention_only: bool   // --mention-only / --no-mention-only
```

### Step 2: watch::run() シグネチャ更新

```
extra_kinds: &[u16]
mention_only: bool
を引数として追加
```

### Step 3: サブスクリプション構築ロジック更新

```
1. extra_kinds が空の場合 → 従来のサブスクリプション B（kind:1 + kind:7）を維持
2. extra_kinds が非空の場合 → カスタム kind サブスクリプション [C] を構築
   - mention_only=true なら .pubkey(target) を追加
   - mention_only=false なら .pubkey() なし（全イベント受信）
```

### Step 4: イベントルーター拡張

```
match event.kind {
    Kind::ChannelMessage => { /* 既存 */ }
    Kind::TextNote => { /* 既存 */ }
    Kind::Reaction => { /* 既存 */ }
    k if k == Kind::from(9735u16) => { /* Zap処理 */ }
    k if extra_kinds.contains(&k.as_u16()) => { /* 汎用カスタムkind処理 */ }
    _ => continue,
}
```

### Step 5: kind:9735 パーサー実装

```
parse_zap_amount(event) → Option<u64>  // bolt11 → sats
parse_zap_description(event) → Option<(String, PublicKey)>  // description tag パース
format_zap_notification() → String
```

### Step 6: Cargo.toml 更新（必要な場合）

```
lightning-invoice = "0.32"  // bolt11 デコード用（バージョン確認要）
```

> **注意**: `nostr-sdk` に bolt11 パース機能が含まれている場合は不要。

### Step 7: テスト

```
# 基本動作確認（従来互換）
nostaro watch --webhook <URL>

# Zap監視
nostaro watch --webhook <URL> --kind 9735

# kind:1 全受信（メンションなし）
nostaro watch --webhook <URL> --kind 1 --no-mention-only

# 複数kind（カンマ区切り）
nostaro watch --webhook <URL> --kind 1,9735,7

# 組み合わせ
nostaro watch --webhook <URL> --kind 9735 --keyword "bitcoin"
```

---

## 6. 後方互換性

| ケース | 現行動作 | 変更後の動作 |
|--------|---------|------------|
| `nostaro watch --webhook <URL>` | kind:1 + kind:7 監視 | **変更なし** |
| `--kind` 未指定 | デフォルト監視 | **変更なし** |
| `--kind 1 --kind 7` | N/A | kind:1 + kind:7 明示指定と同等 |
| `--kind 9735` | N/A | Zap監視を追加 |
| `--no-mention-only` | N/A | 全イベント受信（注意: 量が多い） |

---

## 7. 注意事項・リスク

### 7.1 `--no-mention-only` の注意

`--no-mention-only` を使うと `.pubkey()` フィルタなしで全リレーから kind イベントを受信するため、**受信量が膨大**になる可能性がある。Discord webhook のレート制限（1チャンネル5req/sec）に注意し、実装時にレート制限ロジックを検討すること。

### 7.2 bolt11 ライブラリ依存

`lightning-invoice` の追加が必要な場合、`Cargo.toml` の変更とビルド確認が必要。まず `nostr-sdk` のソースを確認し、内包されているか確認すること。

### 7.3 kind:9735 の受信タイミング

Zap Receipt はウォレットサービス（例: Coinos）が送信するイベントであり、Zap 実行から数秒〜数十秒の遅延がある場合がある。`created_at` の5分スキップフィルタは維持する。

### 7.4 `description` タグのパースエラー

一部の古いウォレット実装では `description` タグが正しい JSON でない場合がある。`serde_json::from_str` のエラーは `Option::None` として扱い、通知を省略せずに amount のみ表示するフォールバックを実装すること。

---

## 8. 参考リソース

- [NIP-01](https://github.com/nostr-protocol/nips/blob/master/01.md) — 基本プロトコル
- [NIP-57](https://github.com/nostr-protocol/nips/blob/master/57.md) — Zap（kind:9735）
- [NIP-28](https://github.com/nostr-protocol/nips/blob/master/28.md) — チャンネル（kind:42）
- [nostr-sdk docs](https://docs.rs/nostr-sdk/latest/nostr_sdk/)
- [BOLT11 invoice spec](https://github.com/lightning/bolts/blob/master/11-payment-encoding.md)
