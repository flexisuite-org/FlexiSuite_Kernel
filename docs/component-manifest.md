# Component Manifest & Lock 仕様（ドラフト）

目的: コンポーネント（UI/サーバー/プラグイン）がカーネル上で安全に配布・依存解決・実行できるよう、manifest と lock の形式を定義する。

## Manifest フィールド
- `name`: スコープ付き名称。例 `@group/app-header`。
- `version`: semver。
- `engine`: 対応する Kernel API バージョン。互換外は拒否。
- `entry`: サーバー実行エントリ（ファイルまたは識別子）。不要なら null。
- `bundle`: クライアント向けバンドルの URI/ID。
- `dependencies`: 必須依存 `{ name, version, integrity }[]`。未解決ならインストール失敗。
- `peerDependencies`: ホストが提供する前提。範囲が満たされなければエラー（ポリシーで警告降格可）。
- `optionalDependencies`: あれば利用。失敗は警告のみ。
- `integrity`: manifest と payload の SHA256（hex）。
- `policyId`: 適用する `ComponentPolicy`（メモリ/タイムアウト/ネットワーク/allowedModules）。
- `capabilities`: 必要権限の宣言（例: `data.read`, `events.publish`）。
- `uiMount` (任意): UIコンポーネントのマウントポイント指定。
- `bundleIntegrity` (任意): バンドル本体の SHA256。署名時は manifestIntegrity と組み合わせて HMAC 化。
- `signature` (任意): HMAC-SHA256(manifestIntegrity, bundleIntegrity) を SIGNING_SECRET で生成。

## ロックファイル `component-lock.json`
- 各インストールで生成される決定的スナップショット。
- エントリ: `name`, `version`, `integrity`, `resolved`(ダウンロードURL/ID), `dependencies`(展開後ツリー)。
- `peerDependencies` はロックに固定せず、解決結果だけ記録し検証に使用。
- 循環検出: DAG でなければ拒否。
- 生成/適用はトランザクション扱い（途中失敗は全ロールバック）。

## 依存解決アルゴリズム（簡潔）
1) manifest から依存グラフ構築 → 循環があれば拒否。
2) semver範囲に合う最新安定版を選択（ポリシーで “最小” も選択可）。
3) すべての resolved パッケージで integrity（SHA256）と署名を検証。
4) peer が満たされない場合はエラー（警告に格下げ可能）。
5) ロックを生成しアトミックに保存。失敗時は既存ロックにフォールバック。

## サーバー実行とクライアント配信
- サーバー実行: 本番コンポーネントはカーネル上でユーザーコードを実行しない（APIモード）。capabilities に応じたカーネル内ハンドラのみ実行。ドラフトは別エンドポイントで sandbox 実行。
- クライアント配信: `bundle` を署名・ハッシュ付きで提供し、CDNキャッシュ可。ESM + スコープ化CSSを推奨。

## 衝突回避の指針
- 名前空間: スコープ付き名でグローバル衝突を抑制。
- クライアント: Shadow DOM/CSS Modules でスタイル衝突を回避。
- サーバー: モジュール解決はサンドボックス内に閉じ、グローバル汚染を禁止。

## エラーハンドリングとフォールバック
- integrity/署名検証に失敗 → インストール/実行を拒否し監査ログを記録。
- 依存解決失敗 → 既存の安定版ロックにフォールバック（なければエラー）。
- ロールアウトはロック単位で実施し、部分的な混在を避ける。
