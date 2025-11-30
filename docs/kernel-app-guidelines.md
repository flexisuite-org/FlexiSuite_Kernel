# Kernel App Guidelines (UI/Frontend)

目的: カーネル上で動く公式/社内アプリを作るときの最低限のルールセット。レイアウトや見た目は自由（ライトテーマ推奨）。

## 技術スタック（推奨）
- Next.js 14 / React 18 / TypeScript
- API通信はすべてカーネルRESTを使用（GraphQLなし）。`NEXT_PUBLIC_KERNEL_API` をベースURLにする。
- トークンは JWT を Bearer で送る。追加の独自セッションは禁止。

## パッケージング
- アプリはコンポーネントパッケージとして登録し、bundleUpload（ZIP推奨）で配布。署名/整合性はカーネル側で検証。
- 直接DB/S3へはアクセスしない。ストレージは `/registry/packages/:id/bundleUpload` 経由。

## 必須で使うAPI
- 認証: `POST /auth/login`
- ヘルス: `GET /health`
- パッケージ: `GET/POST /registry/packages`, `POST /registry/packages/:id/bundleUpload`, `POST /registry/packages/:id/approve`, `POST /registry/packages/:id/revoke`
- インストール: `GET /install`, `POST /install`
- 実行: `POST /components/:id/run` （Draft: `x-flexi-mode: draft` ヘッダ）
- GitHub連携（準備済みエンドポイント）: `POST /integrations/github/webhook`, `POST /integrations/github/build`, `GET /integrations/github/status`
- WebSocket（接続テスト用）: `GET /ws` で要JWT。今は echo のみ。

## 権限・マルチテナント
- JWT に `groupId` を必須。group を跨ぐ操作は禁止。
- capability allowlist / role allowlist を尊重。新しい capability が必要な場合はカーネル側に追加依頼。

## OpenAI等の外部API
- 直接キーを持たせない。カーネル側のプロキシAPIを使う（今後追加予定）。

## UXの緩い指針
- ライトテーマ推奨。可読性重視でアクセント1色。
- JSON表示はモノスペース＋折りたたみがあると良い。
- フェッチ層は1箇所に集約し、401時の再ログイン導線だけ用意すれば十分。
