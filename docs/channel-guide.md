# NIP-28 パブリックチャンネル & Discord 通知ガイド

> nostaro で Nostr のパブリックチャンネル（NIP-28）を操作し、Discord にリアルタイム通知を送る手順書。

---

## NIP-28 とは？

**NIP-28** は Nostr プロトコルにおけるパブリックチャットチャンネルの仕様です。IRC や Discord のチャンネルに近い概念で、誰でもチャンネルを作成・参加・投稿できます。

| kind | 用途 |
|------|------|
| 40 | チャンネル作成（メタデータ） |
| 41 | チャンネルメタデータ編集 |
| 42 | チャンネルメッセージ |

チャンネルは作成時のイベントID（64文字の hex 文字列）で識別されます。

---

## 前提条件

```bash
# nostaro の初期設定が済んでいること
nostaro init

# リレーが設定されていること（確認）
nostaro relay list
```

---

## 1. チャンネル作成

```bash
nostaro channel create --name "my-channel" --about "チャンネルの説明" --picture "https://example.com/icon.png"
```

- `--name` : チャンネル名（必須）
- `--about` : 説明文（任意）
- `--picture` : アイコンURL（任意）

作成に成功すると、チャンネルIDが表示されます:

```
Channel created! ID: 54acbbb29ba14a442d0329f8f80cdac266c2abac3909793e55f67c36d57ffec2
```

このIDは以降すべてのチャンネル操作で使用します。メモしておきましょう。

---

## 2. チャンネル情報の編集

```bash
nostaro channel edit 54acbbb29ba14a442d0329f8f80cdac266c2abac3909793e55f67c36d57ffec2 \
  --name "新しいチャンネル名" \
  --about "更新した説明" \
  --picture "https://example.com/new-icon.png"
```

- `--name` は必須です（変更しない場合も現在の名前を指定）
- `--about`、`--picture` は任意

> ⚠️ チャンネルメタデータの編集は作成者のみ可能です。

---

## 3. チャンネル一覧

```bash
nostaro channel list
```

リレーからチャンネル（kind:40）の一覧を取得して表示します。

---

## 4. チャンネルメッセージの読み取り

```bash
nostaro channel read 54acbbb29ba14a442d0329f8f80cdac266c2abac3909793e55f67c36d57ffec2
```

指定したチャンネルの過去のメッセージ（kind:42）を取得して表示します。

---

## 5. チャンネルへの投稿

```bash
nostaro channel post 54acbbb29ba14a442d0329f8f80cdac266c2abac3909793e55f67c36d57ffec2 "こんにちは！"
```

第一引数にチャンネルID、第二引数にメッセージ本文を指定します。

---

## 6. Discord 通知設定（watch コマンド）

`nostaro watch` は Nostr のイベントをリアルタイムで監視し、Discord Webhook に通知を送ります。

### 6.1 Discord Webhook URL の取得

1. Discord のサーバー設定 → **連携サービス** → **ウェブフック** を開く
2. **新しいウェブフック** を作成
3. 通知を送りたいチャンネルを選択
4. **ウェブフックURLをコピー** をクリック

### 6.2 自分へのメンション・リプライ・リアクションを監視

```bash
nostaro watch --webhook https://discord.com/api/webhooks/YOUR_WEBHOOK_ID/YOUR_WEBHOOK_TOKEN
```

検出するイベント:
- メンション（自分の公開鍵がタグされたノート）
- リプライ
- リアクション（kind:7）— 元の投稿が引用表示されます
- リポスト（kind:6）

### 6.3 特定ユーザーへのイベントを監視

```bash
nostaro watch \
  --webhook https://discord.com/api/webhooks/YOUR_WEBHOOK_ID/YOUR_WEBHOOK_TOKEN \
  --npub npub1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
```

### 6.4 NIP-28 チャンネルを監視

```bash
nostaro watch \
  --webhook https://discord.com/api/webhooks/YOUR_WEBHOOK_ID/YOUR_WEBHOOK_TOKEN \
  --channel 54acbbb29ba14a442d0329f8f80cdac266c2abac3909793e55f67c36d57ffec2
```

チャンネルに投稿された新しいメッセージ（kind:42）が Discord に通知されます。

### 通知の特徴

- 投稿者の **プロフィールアイコン** と **表示名** が Webhook のアバターとして使用されます
- リアクション通知には **元の投稿が引用** 表示されます
- `Ctrl+C` で停止

---

## 7. launchd で常駐化（macOS）

`nostaro watch` をバックグラウンドで自動起動するには、macOS の launchd を使います。

### 7.1 plist ファイルの作成

```bash
cat > ~/Library/LaunchAgents/com.nostaro.watch.plist << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.nostaro.watch</string>
    <key>ProgramArguments</key>
    <array>
        <string>/Users/YOUR_USERNAME/.cargo/bin/nostaro</string>
        <string>watch</string>
        <string>--webhook</string>
        <string>https://discord.com/api/webhooks/YOUR_WEBHOOK_ID/YOUR_WEBHOOK_TOKEN</string>
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
EOF
```

> 💡 `YOUR_USERNAME` と Webhook URL を自分の環境に合わせて書き換えてください。
> nostaro のパスは `which nostaro` で確認できます。

チャンネル監視の場合は `<array>` に以下を追加:

```xml
<string>--channel</string>
<string>54acbbb29ba14a442d0329f8f80cdac266c2abac3909793e55f67c36d57ffec2</string>
```

### 7.2 サービスの登録・起動

```bash
# 登録して起動
launchctl load ~/Library/LaunchAgents/com.nostaro.watch.plist

# 停止して解除
launchctl unload ~/Library/LaunchAgents/com.nostaro.watch.plist
```

### 7.3 ログの確認

```bash
# 標準出力
tail -f /tmp/nostaro-watch.log

# エラー出力
tail -f /tmp/nostaro-watch.err
```

---

## コマンド早見表

| 操作 | コマンド |
|------|---------|
| チャンネル作成 | `nostaro channel create --name "名前"` |
| チャンネル編集 | `nostaro channel edit <ID> --name "新名前"` |
| チャンネル一覧 | `nostaro channel list` |
| メッセージ読み取り | `nostaro channel read <ID>` |
| メッセージ投稿 | `nostaro channel post <ID> "本文"` |
| メンション監視 | `nostaro watch --webhook <URL>` |
| チャンネル監視 | `nostaro watch --webhook <URL> --channel <ID>` |

---

## 関連リンク

- [NIP-28 仕様](https://github.com/nostr-protocol/nips/blob/master/28.md)
- [nostaro GitHub](https://github.com/kojira/nostaro)
