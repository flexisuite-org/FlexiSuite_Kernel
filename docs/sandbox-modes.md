# Sandbox Modes（ドラフト）

目的: カスタムUXの生成・試験・本番実行を安全に分離し、本番体験を途切れさせないためのサンドボックス運用モードを定義する。

## モード一覧
- **draft**: AI/開発者が生成直後に使う。`POST /sandbox/drafts/run` で isolated-vm 実行。本番データには書き込まない（プレイグラウンドスキーマ/一時テーブル）。リソース制限を最も厳しく、監査を詳細に。
- **staging**: 試験運用（小規模ユーザー/allowlist）。本番データは読み取りのみ可、書き込みは影響を最小化するルールで制限。リソース制限は中程度。
- **stable**: 本番。署名+ロック済みコンポーネントのみ。`/components/:id/run` はユーザーコードを実行せず、メタとバンドル配布のみ。ハッシュ不一致は即拒否し、旧安定版にフォールバック。

## リソース制限の推奨値
- draft: memory 64–128MB, timeout 200–500ms, allowNetwork=false, allowedModules=最小限。
- staging: memory 128–256MB, timeout 500–800ms, allowNetwork=false（必要時のみ allowlist）。
- stable: policy依存だが、ネットワークはデフォルト禁止。必要最小限だけ allowlist。

## データ書き込みポリシー
- draft: プレイグラウンドスキーマ/一時テーブルへリダイレクト（現状は PlaygroundLog に保存）。RLSは保持。Prismaミドルウェアで draft モード時の書き込みをブロック（PlaygroundLog 以外）。
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
