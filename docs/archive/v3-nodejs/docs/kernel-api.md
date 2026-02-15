# Kernel API サーフェス（ドラフト）

目的: カーネルが提供する安定インターフェイスを定義し、フロント/ストア/生成サービスが安心して連携できるようにする。

## 共通事項
- 原則すべてのリクエストで `groupId` コンテキストが必須（JWT ペイロード or ヘッダ）。Auth/Invite など一部のエンドポイントは例外として group コンテキスト無しで呼び出せる。
- チャンネル: `channel=stable|draft` をクエリ/ヘッダで指定。指定なしは `stable`。
- レスポンスには `correlationId` を含め、AuditLog と紐づける。
- 認証: JWT (15m) + Refresh(7d)、デバイス/IPバインド、再利用検知。Auth系は厳しめのIPレートリミット。

## エンドポイント概要

### Auth
- `POST /auth/signup` – （α版では）`accountInviteCode` 必須。`email/password` とコードを受け取り、AccountInvite を検証して User を作成し、必要に応じて初期グループに紐づけつつ access+refresh を返す。
- `POST /auth/login` – email/password → access + refresh。
- `POST /auth/refresh` – 再利用検知あり。family 無効化と監査を実施。
- `POST /auth/logout` – refresh revoke。
- `GET /auth/me` – 認証済みユーザーのプロフィールと memberships を返す。例: `{ userId, email, roles, memberships: [{ groupId, name, type, role }] }`。

### Invites / Onboarding

#### AccountInvite（アカウント作成用招待）
- `POST /auth/account-invites` – Kernel Admin 限定。body: `{ email, expiresAt?, initialGroupId? }`。AccountInvite を作成し `{ code, expiresAt }` を返す。メール送信は別サービス/ジョブ。
- `GET /account-invites/:code` – サインアップ前にコードの有効性を検証し、対応する `email` と `initialGroupId`（あれば）を返す。

#### GroupInvite（グループ参加招待）
- `POST /group-invites` – グループ管理者向け。body: `{ groupId, kind: "LINK" | "EMAIL", email?, expiresAt? }`。GroupInvite を作成し `{ id, code }` を返す。
- `GET /group-invites/pending?email=…` – ログイン中ユーザーの email に紐づく未承諾招待（kind=EMAIL, acceptedAt=null, expiresAt>now）一覧を返す。
- `GET /group-invites/link/:code` – 汎用招待リンク用。対象グループ情報・期限・既に受諾済みかどうかを返す（未認証でも参照可）。
- `POST /group-invites/:code/accept` – 認証済みユーザーが招待を受諾し、GroupMember を作成する。レスポンスは `{ accepted: true, groupId, roles[] }` など。
- `POST /group-invites/:code/decline` – 招待を辞退し、今後の一覧から除外する。

### Launcher / User Home
- `GET /launcher/groups` – ログインユーザーが所属するグループと、その各グループにインストール済みのアプリ/コンポーネントのサマリを返す。例: `[{ groupId, name, type, installs: [...] }]`。
- `GET /groups/:groupId/installs` – 特定グループのインストール詳細を返す。`set_config('flexi.current_group')` により RLS を守りつつ、ComponentInstall + ComponentPackage のメタ情報を返却。
- `GET /invites/pending` – ログインユーザー自身に紐づく GroupInvite 一覧を返す（email はサーバ側で User.email から解決）。ランチャーで「招待中グループ」カードを描画する際に利用。

### Registry / Package
- `POST /registry/packages` – 署名対象の manifest+payload を登録（承認ステータス: draft）。
- 署名フロー: manifest を SHA256 → integrity に保存し、SIGNING_SECRET があれば HMAC 署名を付与。bundleIntegrity があれば manifestIntegrity と組にして署名。Install/Run で integrity と署名を検証。
- `POST /registry/packages/:id/approve` – カーネルが署名し、`approved` に遷移。
- `POST /registry/packages/:id/revoke` – 誤配信対策で失効。
- `GET /registry/packages/:name` – バージョン一覧/メタデータ。
- `GET /registry/packages/:id/download` – bundle/payload ダウンロード（署名+integrity付き）。
- `POST /registry/packages/:id/bundle` – アップロード後に bundleIntegrity（＋SIGNING_SECRET があれば bundleSignature を自動生成）を登録。将来的には GitHub 等のリポジトリ/Artifact からカーネル側が直接バンドルを取得するフローをサポートし、Cloudflare Pages 的な「リポ更新→Kernel側も自動更新」を実現する。

### Install
- `POST /install` – { packageId, version, channel? } → 依存解決＋ロック生成をアトミック実行。入力は Zod で検証し、root manifest の integrity を照合。
- draft 実行 (`/sandbox/drafts/run`) は `default_transaction_read_only=on` + ミドルウェアで書き込みを拒否（PlaygroundLog を除く）。
- `DELETE /install/:installId` – アンインストール。
- `POST /install/:installId/rollback` – 直前ロックに戻す。
- `GET /install` – インストール一覧（groupスコープ）。

### Admin – Tenants / Users / Roles
- `POST /groups` – 新しいテナント/グループを作成。
- `GET /groups` – 全テナント/グループ（またはフィルタ済み）の一覧を取得。
- `PATCH /groups/:id` – グループ名や設定（plan, flags, hierarchy など）を更新。
- `POST /groups/:id/deactivate` – グループを論理停止し、新規ログイン/書き込みを抑止。
- `GET /users` – 条件付きでユーザー一覧検索（email, group, status など）。
- `GET /users/:id` – 特定ユーザーの詳細と memberships を取得。
- `PATCH /users/:id` – ロールやステータスの更新。
- `POST /users/:id/force-logout` – 該当ユーザーの refresh token family を revoke し、強制ログアウト。
- `GET /roles` / `POST /roles` – RBAC の Role 一覧取得/作成。
- `POST /roles/:id/permissions` – Role に対する Permission セットを付け替える。

### Run / Bundle（本番パッケージのみ）
- `POST /components/:id/run` – APIモードで capabilities を実行（カーネル側の安全ハンドラのみ）。署名/integrity検証と監査を実施。ユーザーコードは走らせない。
- `GET /components/:id/bundle` – クライアント用バンドル取得。`If-None-Match` 等でキャッシュ。

### Draft Sandbox
- `POST /sandbox/drafts/run` – ドラフト用サンドボックスでスクリプトを実行（isolated-vm）。本番データは書き込まない。監査必須。

### Rollout Control
- `POST /rollout` – { lockId, percentage, allowlist?, blocklist? } を設定。
- `GET /rollout/:lockId` – 現在のロールアウト設定を取得。

### Audit / Logs
- `GET /audit-logs` – AuditLog テーブルに対する検索インターフェイス。フィルタ（期間, groupId, userId, resource, action, success など）を受け取り、ページングされた結果を返す。
- `GET /metrics` – 既存の Prometheus エンドポイント。Kernel Admin UI からグラフに利用。
- `GET /health` – 基本的なヘルスチェック（DB, Redis など）。

## エラーハンドリング（要約）
- 認証失効/再利用: 401 + family revoke, AuditLog。
- 署名/ハッシュ不一致: 422 で拒否し、フォールバックがあれば `X-Fallback-Lock` を返却。
- 依存解決失敗: 409 + 具体的な未解決依存/peer不一致を列挙。
- RLS違反: 403。

## 監査とメトリクス
- AuditLog: login/refresh/reuse-detect/register/approve/revoke/install/rollback/run を全て記録（actor, groupId, resource, action, success, correlationId）。
- Metrics: auth失敗率、install成功率、runレイテンシ、サンドボックス失敗率、queue lag。

## セキュリティ/ポリシー要点
- RLS: `current_setting('flexi.current_group')` を必須にし、Prismaミドルウェアで where/data に groupId を強制。
- ポリシー: ComponentPolicy により memory/timeout/allowNetwork/allowedModules を適用。UI配信はネットワーク不可のチェックのみで軽量化可。
- 署名鍵: カーネルが保持。署名リクエストはカーネル内で完結し、外部に鍵を出さない。

## 今後の拡張フック
- A/B / Canary: rollout 設定を `run` と `bundle` のレスポンスで反映し、特定割合のみ draft を返す。
- Feature Flags: group/user 単位のフラグをヘッダで受け、ポリシー切替を可能にする。
