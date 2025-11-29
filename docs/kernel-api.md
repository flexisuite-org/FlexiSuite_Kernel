# Kernel API サーフェス（ドラフト）

目的: カーネルが提供する安定インターフェイスを定義し、フロント/ストア/生成サービスが安心して連携できるようにする。

## 共通事項
- すべてのリクエストで `groupId` コンテキストが必須（JWT ペイロード or ヘッダ）。
- チャンネル: `channel=stable|draft` をクエリ/ヘッダで指定。指定なしは `stable`。
- レスポンスには `correlationId` を含め、AuditLog と紐づける。
- 認証: JWT (15m) + Refresh(7d)、デバイス/IPバインド、再利用検知。Auth系は厳しめのIPレートリミット。

## エンドポイント概要

### Auth
- `POST /auth/login` – email/password → access + refresh。
- `POST /auth/refresh` – 再利用検知あり。family 無効化と監査を実施。
- `POST /auth/logout` – refresh revoke。

### Registry / Package
- `POST /registry/packages` – 署名対象の manifest+payload を登録（承認ステータス: draft）。
- 署名フロー: manifest を SHA256 → integrity に保存し、SIGNING_SECRET があれば HMAC 署名を付与。bundleIntegrity があれば manifestIntegrity と組にして署名。Install/Run で integrity と署名を検証。
- `POST /registry/packages/:id/approve` – カーネルが署名し、`approved` に遷移。
- `POST /registry/packages/:id/revoke` – 誤配信対策で失効。
- `GET /registry/packages/:name` – バージョン一覧/メタデータ。
- `GET /registry/packages/:id/download` – bundle/payload ダウンロード（署名+integrity付き）。
- `POST /registry/packages/:id/bundle` – アップロード後に bundleIntegrity（＋SIGNING_SECRET があれば bundleSignature を自動生成）を登録。

### Install
- `POST /install` – { packageId, version, channel? } → 依存解決＋ロック生成をアトミック実行。入力は Zod で検証し、root manifest の integrity を照合。
- `DELETE /install/:installId` – アンインストール。
- `POST /install/:installId/rollback` – 直前ロックに戻す。
- `GET /install` – インストール一覧（groupスコープ）。

### Run / Bundle（本番パッケージのみ）
- `POST /components/:id/run` – APIモードで capabilities を実行（カーネル側の安全ハンドラのみ）。署名/integrity検証と監査を実施。ユーザーコードは走らせない。
- `GET /components/:id/bundle` – クライアント用バンドル取得。`If-None-Match` 等でキャッシュ。

### Draft Sandbox
- `POST /sandbox/drafts/run` – ドラフト用サンドボックスでスクリプトを実行（isolated-vm）。本番データは書き込まない。監査必須。

### Rollout Control
- `POST /rollout` – { lockId, percentage, allowlist?, blocklist? } を設定。
- `GET /rollout/:lockId` – 現在のロールアウト設定を取得。

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
