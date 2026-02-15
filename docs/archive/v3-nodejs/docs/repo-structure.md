# Repo Structure & App Categories (Draft)

目的: FlexiSuite Kernel と、その上で動く公式アプリ群/コンポーネント群を、将来的にリポ分割しやすい形で整理する。

## 1. モノレポ構成の基本方針

- このリポは「Kernel + 公式アプリ群 + 共通パッケージ」をまとめたモノレポとする。
- 物理構造の例:
  - `src/` … Kernel 本体（Fastify API / Prisma / Runtime 等）
  - `apps/kernel-admin` … Kernel Admin UI（提供元専用）
  - `apps/launcher` … ランチャー（ユーザー/グループ向けホーム）
  - `apps/store` … コンポーネント/アプリのストア UI
  - `apps/flexistudy` … FlexiStudy などプロダクトアプリ（後から移植）
  - `packages/kernel-api-types` … Kernel API の型/Zodスキーマ共有
  - `packages/ui-*` … 共通 UI コンポーネントが増えたらここにまとめる

→ 将来的に分割する場合は、`apps/*` を別リポに移し、`packages/*` を npm パッケージ化することで、比較的楽に切り出せる構造を目指す。

## 2. アプリの分類（レジストリ側で持つメタ情報）

物理ディレクトリはシンプルに `apps/*` で揃え、実際の「種類」は **レジストリ/manifest 側のメタ情報** で区別する。

### アプリ種別（例）

- `core-app`:
  - Kernel とセットで提供される公式アプリ。
  - 例: kernel-admin, launcher, store。
- `product-app`:
  - FlexiStudy のような「1つのサービス」として成立する業務アプリ。
  - Kernel の上で動き、課金やカスタムUXの主役になる。
- `tool-app`:
  - 開発者向けツール、生成/審査/分析用ダッシュボードなど。

### manifest/レジストリでの表現案

- `ComponentPackage` の manifest にフィールドを追加するイメージ:
  - `kind`: `"core-app" | "product-app" | "tool-app" | "component"` など。
  - `tags`: `["flexistudy", "admin", "store"]` のような任意タグ。
- 物理リポ構造とは独立して、ストアや Kernel Admin UI ではこの `kind`/`tags` を見て分類表示する。

## 3. コンポーネントとアプリの関係

- アプリは「1つの大きな固まり」ではなく、コンポーネント（モジュール）の組み合わせとして構成される。
  - デフォルト機能もコンポーネントとして提供され、アプリインストール時に一括インストールされる。
  - マイ・コンポーネントや他者のコンポーネントで差し替え・拡張が可能。
- 1つのリポジトリに「アプリ本体 + 複数コンポーネント/コンポーネントセット」が入っていてよい:
  - GitHub or ZIP インポート → カーネルが manifest 群を読み取り、
    - アプリ本体用パッケージ
    - 同梱コンポーネント群
    に分解して Registry に登録する。
- コンポーネントは複数アプリで共有可能:
  - manifest の `dependencies` と `engine`（対応 Kernel API バージョン）に従って、
    必要なコンポーネントを自動インストール＆整合性チェックできる。

## 4. 境界ルール（将来の分割を楽にするために）

- Kernel とアプリの境界:
  - アプリは **Kernel の REST / WebSocket / イベントのみ** を通じてバックエンドに触れる。
  - 直接 DB/Redis/Prisma にはアクセスしない（全部 Kernel API 経由）。
  - `packages/kernel-api-types` でエンドポイント名・リクエスト/レスポンス型・エラー型を共有する。
- アプリ同士の境界:
  - アプリ間で直接通信せず、Kernel API/イベントを介して連携する。
  - 将来別リポに分けても壊れないように、「横の依存」は Kernel を挟むことを原則とする。

## 5. 今後のTODO

- `packages/kernel-api-types` の実体を作り、既存の API 実装/テストから型を抽出していく。
- 各アプリ (`apps/*`) 用に「このフォルダは Next.js 14 + TS 前提」「Kernel REST をこう使う」という最小ガイドラインを docs に追加。
- FlexiStudy を移植する際に、この構造に乗るようにリポ配置/manifest を調整する。

