# FlexiSuite 構想ドキュメント（ドラフト）

目的: FlexiSuite を「カスタムUXを量産・配信・実行できるSaaSプラットフォーム」として位置づけ、カーネルとフロント系プロセスの責務分離を明確にする。

## 全体像
- **カーネル**: 唯一のバックエンド境界。認証・署名・RLS・監査・イベント・依存解決・コンポーネント配信/実行ゲートを担当。ユーザーは直接触らない。
- **アプリプロセス（ハブ/認証UI/ストア/業務UI/FlexiStudyなど）**: カーネル上で動くフルスタックWebアプリ。API呼び出しはカーネルのみ。アプリはテナント/スペース/料金プランなど独自のUX概念を持てるが、データの最終的な境界は常に `groupId`（カーネル側）で管理する。
- **コンポーネント/モジュール**: アプリの機能ユニット（拡張機能）。UI要素・ロジック・ウィジェット・セクション・API呼び出しなど粒度はさまざま。単独実行ではなく「アプリプロセスから呼び出される」ことを前提とし、依存関係とポリシーを伴う。
- **デフォルトコンポーネント群**: 各アプリの「標準機能」もコンポーネントとして提供される。アプリインストール時にデフォルトコンポーネント群が一括でインストールされ、ユーザーはそこからマイ・コンポーネントで上書き/拡張していく。

## カスタムUXライフサイクル（責務分割）
1) 生成: AI/開発者がドラフトコンポーネントを作成（上位サービスやローカル開発環境から GitHub 等を経由）。
2) サンドボックス作成: ユーザーの「現在のアプリ環境」をもとにサンドボックスを作成する。コード/インストール済みコンポーネント/必要なデータをカーネル側で複製し、「枝」として独立した `groupId`（または同等の隔離単位）を割り当てる。
3) テスト: サンドボックス環境でドラフトコンポーネントを差し替え/追加し、ユーザー自身が「自分のアプリに入れたらどう動くか」を確認する。本番の `groupId` 側データは汚染されない。
4) 承認: ユーザーがドラフトを採用すると、承認API経由でカーネルに署名付きパッケージとして登録（hash+manifest固定）。これが「マイ・コンポーネント（stable）」になる。
5) インストール: テナント/グループの管理者がマイ・コンポーネントやストア公開コンポーネントをアプリにインストール。デフォルトコンポーネントに対する置き換えや依存解決はカーネルが行い、ロックファイルを生成。
6) 実行: アプリプロセスが UI バンドルを取得してクライアントで実行、または `run` API でサーバー実行。いずれもカーネルで署名・ポリシーを検証し監査する。

## コンポーネントの形態
- **UIバンドル**: `GET /components/:id/bundle` で配布。ESMバンドル＋スコープ化CSS推奨。フロントはCDNから取得し、カーネルAPIでデータを取得。
- **サーバー実行コンポーネント**: `POST /components/:id/run` で実行。sandboxあり/なしをポリシーで選択。`groupId/userId` をコンテキストに注入し、監査ログを必須化。

## パッケージ仕様（提案）
- **manifest**: `name`, `version`, `entry` (server), `bundle` (client), `dependencies` (name@semver+integrity), `integrity` (SHA256), `policyId`, `engine` (kernel API semver), `capabilities`, `uiMount` (任意)。
- **署名**: カーネルの署名鍵で manifest+hash を署名。インストール時・実行時に検証。
- **ロックファイル**: `component-lock.json` に name@version+hash を固定し、再現性を担保。インストールはオールオアナッシングでロールバック可能にする。

## 依存解決とコンフリクト低減
- 名前空間: スコープ付き `@group/component` を採用。
- 互換性: `engine` でカーネルAPIの互換性をチェックし、非互換は拒否。
- 自動インストール: manifest の dependencies をたどり、未インストールを一括取得→ハッシュ/署名検証→ロック生成。失敗時は完全ロールバック。
- クライアント衝突回避: ESM・CSS Modules/Shadow DOM を推奨。サーバー側はサンドボックスでモジュール解決を隔離。

## カーネルが担うべきAPIサーフェス（最小）
- Auth: login/refresh/logout（JWT15m + Refresh7d, 再利用検知, デバイス/IPバインド）。
- Registry: register/get/list/revoke component packages, download bundle（Zip/GitHub Artifact 双方からの取得を想定。GitHub 更新時にカーネル側から Artifact を取りに行くフローを優先）。
- Install: install/uninstall/list/rollback per group; lock生成と依存解決を内蔵。
- Run: `POST /components/:id/run`（サーバー実行）、`GET /components/:id/bundle`（クライアントバンドル）。
- Audit/Event: register/approve/install/run を必ず記録・発火。

## セキュリティとポリシー
- テナント隔離: RLS + Prismaミドルウェアで全テーブルに `groupId` を強制。
- ポリシー: `ComponentPolicy` で memoryMb/timeoutMs/allowNetwork/allowedModules を管理。UI配信は緩め、サーバー実行は厳格。
- 署名/ハッシュ: 署名鍵はカーネルのみ保持。配布・インストール・実行時に integrity を検証。

## レポジトリ/モノレポ方針
- 初期: 1リポ内でカーネルとフロント（ハブ/ストア/業務UI）をワークスペース分割し、API仕様を共通パッケージに切り出す。
- 安定後: カーネルを別リポに抽出（信頼境界）。フロントや生成/審査サービスは独立リポに分離可能。

## 次のアクション案
- 本ドキュメントをベースに `docs/kernel-api.md` を作成し、エンドポイントと manifest のスキーマを具体化。（実施済み）
- Prisma スキーマに Component 系モデル（Package/Install/Policy/Dependency）を追加する下書きを用意。（実施済み）
- `docs/component-manifest.md` で manifest/lock と依存解決の仕様を明文化。（実施済み）
- `docs/sandbox-modes.md` で「環境コピー型サンドボックス」と draft/stable の役割を再定義し、サンドボックスはあくまで開発/検証用の枝であり、本番ユーザーは stable コンポーネント群を共有利用するモデルであることを明記。（要アップデート）
