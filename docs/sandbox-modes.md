# Sandbox Modes（ドラフト）

目的: カスタムUXの生成・試験・本番実行を安全に分離し、本番体験を途切れさせないためのサンドボックス運用モードを定義する。ここでいうサンドボックスは「ユーザーの現在のアプリ環境＋必要なデータを枝として複製した検証用環境」であり、本番ユーザーが普段使うアプリそのものではない。

## モード一覧
- **draft**: AI/開発者が生成直後に使う「サンドボックス枝」。サンドボックス作成時点のコード/インストール済みコンポーネント/必要なデータをカーネル側で複製し、独立した `groupId`（または同等の隔離単位）として扱う。`POST /sandbox/drafts/run` では isolated-vm 実行も行うが、アプリとしての振る舞いも含め「ドラフトコンポーネントを差し替えた自分のアプリ」を安全に試す場とする。
- **staging**: （オプション）試験運用（小規模ユーザー/allowlist）。本番に近い環境での限定ロールアウト。リソース制限は中程度。
- **stable**: 本番。署名+ロック済みコンポーネントのみ。本番ユーザーは stable なコンポーネント群を共有利用し、サンドボックス枝は一切参照しない。`/components/:id/run` はユーザーコードを実行せず、メタとバンドル配布のみ。ハッシュ不一致は即拒否し、旧安定版にフォールバック。

## リソース制限の推奨値
- draft: memory 64–128MB, timeout 200–500ms, allowNetwork=false, allowedModules=最小限。
- staging: memory 128–256MB, timeout 500–800ms, allowNetwork=false（必要時のみ allowlist）。
- stable: policy依存だが、ネットワークはデフォルト禁止。必要最小限だけ allowlist。

## データ書き込みポリシー
- draft:
  - 理想モデル: サンドボックス作成時に、対象グループ/アプリで必要なデータを「サンドボックス用グループ」にコピーし、そのコピーに対して自由に書き込みを許可する。本番グループ側のデータは汚染されない。
  - 現状実装: `setRlsContext(..., mode='draft')` で `flexi.current_group`/`flexi.current_user` に加えて `default_transaction_read_only=on` を Postgres セッションにセットし、Prisma ミドルウェア（`write_not_allowed_in_draft`）により PlaygroundLog を除く本番テーブルへの create/update/delete/upsert を拒否する。PlaygroundLog だけが例外として書き込みを許可し、将来的にはグループ単位のデータコピー方式に移行する想定。
- staging: 本番スキーマだが限定的なエンティティのみ書き込み許可、またはシャドー書き込み＋比較。
- stable: 通常の本番書き込み。RLSとAuditが必須。

## ルーティングとチャネル
- APIパラメータ `channel=draft|stable`（省略時 stable）。feature-flag/allowlist で draft/staging に振り分け。
- RolloutRule でパーセンテージ/allowlist/blocklist を設定し、draft を限定配信。

## 監査/計測
- 実行ログは channel 別に分離集計。AuditLog に channel, policyId, componentId, integrity 検証結果を記録。
- メトリクス: draft/stable 別のレイテンシ・失敗率・リソース使用。

## エラーとフォールバック
- integrity/署名不一致: 即拒否し、stable ロックがあればそちらへフォールバック。
- 依存解決失敗: インストールを中断し、既存ロックを維持。
- リソース超過: sandbox 停止＋監査。必要ならポリシー再評価。

## サンドボックス枝の作成フロー

1. 認証済みユーザーが `POST /sandbox/groups` を叩き、任意の `appId`・`ttlHours`（省略時 24h）を渡す。
2. `contextPlugin` 経由でリクエストコンテキストの `groupId`（元グループ）を取得し、`createSandboxForGroup` を呼び出す。
   - 現行では元グループの `type`/`settings`/`parentId` を再利用し、名称に `[sandbox]` を付けた新しい `Group` を作る。
   - `SandboxSession` レコード (`sourceGroupId`, `sandboxGroupId`, `appId`, `expiresAt`) を登録し、TTL で有効期限を管理。
   - TODO: AppInstall や EntityRecord などのコピー処理は未実装（将来的にはアプリ単位のコピー対象選択フローを接続）。
3. API は `{ sandboxGroupId, sessionId }` を返却し、`SandboxSession` から対応するコピー履歴を追跡できる。
