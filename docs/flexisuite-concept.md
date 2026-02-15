# FlexiSuite コンセプトドキュメント

> **最終更新**: 2026-02-15
> **対象読者**: プロジェクト参加者、AIエージェント、将来の自分
> **Note**: 本文書は概念説明用のドキュメントである。実装上の詳細な契約仕様（MUST/SHOULD等）については、正本である [`docs/implementation_plan.md`](./implementation_plan.md) を参照すること。

---

## 1. FlexiSuiteとは何か

**FlexiSuite** は「AI共生時代の Flexible OS for SaaS」—— AI駆動のアプリケーション開発（Vibe Coding）を民主化するOS級プラットフォームである。

### 背景
FlexiStudy（勉強管理アプリ）でAIがUI/UXを自己書き換えする「カスタムUX」を実証した。得られた知見:

- **カスタムUXには確かな需要があり、最新のAIで実現可能**
- しかしアプリごとの個別実装は非効率 → **汎用的なインフラ（OS）が必要**

### 定義
| 対象 | FlexiSuiteとは |
|---|---|
| **ユーザー** | AIと共に作った「自分だけのアプリ」を、どの端末からでも使えるクラウドOS |
| **AI** | 純粋なロジック・UI生成に集中できる、安全な実行環境 |
| **エコシステム** | 誰かの成果物をインストールし、AIでさらに改造する。分散型エコシステムを推奨し、ユーザー間の知の循環（Bridging）を促す |
| **アプリ** | Kernel上で動作する。カスタムドメインやパス設定により「自分だけの独立したアプリ」としての所有感を持つ |


---

## 2. ビジョンと設計原則

### Mission
**「AI共生時代のアプリケーション開発を、民主化する。」**

> **MDP (Minimal Desirable Product) という考え方**
> FlexiSuiteはMVP（実用最小限）ではなく、**MDP（魅力的最小限）** を目指す。「とりあえず動く」プロトタイプではなく、OSとしての品格（堅牢性、セキュリティ、拡張性）を備えた **"Desirable"** な基盤を最初から構築する。

### Vision
**「The Safe & Seamless Vibe Coding OS」**

- **Zero Setup** — ブラウザを開けばそこが開発・実行環境
- **AI Native** — 標準化されたインターフェースと安全なサンドボックス
- **Ecosystem** — コンポーネントをストアで配布・購入・インストール
- **AI Self-Correction Loop** — エラーやデザイン崩れをAI自身が検知し、修正する「自己修復ループ」をOSレベルで支援
- **Feedback Interface** — KernelはAIを主対象に、認可された開発者にも構造化診断データを提供する。ユーザーは「AIの思考と作業」をダッシュボードで観測でき、安心して任せられる体験（Observability）を提供する。
- **AI Rulebook** — プロジェクトごとに「デザインのルール（Wabi-Sabi等）」や「コンテキスト」を定義し、AIの自己修復を制御する権利をユーザーに与える。デフォルトは「完成された美しさ」だが、例外を許容する柔軟性を持つ。
- **Openness** — Jailbreak不要。SideloadingとSelf-Hostingを公式にサポートし、誰でもKernelを改造・運用できる。Kernel UI自体もFlexiSuiteアプリとして実装される（Dogfooding）。
- **Data Portability** — 「アプリ定義」と「データ」をセットで書き出し、他のFlexiSuiteインスタンスへ完全に移行できる権利（Exit Right）を保証する。



### ビジネスモデル: OSS & Managed Hosting
- **Kernel = OSS**: コア部分はオープンソース（Apache 2.0 / MIT）として公開し、エンジニアの参入障壁を極限まで下げる。
- **Revenue = Hosting**: VercelやAWSのように、最適化されたマネージド環境（FlexiSuite Cloud）の提供で収益化する。「自分でホストすれば無料、任せれば有料」のモデル。

### 3つの設計原則

1. **Safety by Design** — AIがどんなコードを書いても、OSが安全性を担保する（3層信頼モデル）
2. **Scalable by Default** — App is Data, Not Infrastructure（JSON定義 → 動的レンダリング、ユーザーごとのコンテナ不要）
3. **Radical Flexibility within Guardrails** — ガードレールの中で無限の自由（カスタムコンポーネント、外部npm、CDN解決）。そしてガードレールの外へ出る自由（Developer Mode）も保証する。

---

## 3. アーキテクチャ概要

### Core Philosophy: Kernel as a Reviewer
Kernelは人間だけでなく、AIに対して「デバッガ兼レビュアー」として振る舞う。エラーはテキストログではなく、AIが解析可能な構造化データ（JSON/Schema/DOM Metrics）で提供される。

### Kernel / Userland 分離
- **Kernel（Rust）**: 認証・ストレージ・イベント・コンピュートのプリミティブを提供。ユーザーは直接触らない
- **Userland**: サンドボックス化されたJS/TSまたはWasmでビジネスロジックを実行
- **Universal Player（Next.js）**: 単一インスタンスがアプリ定義（JSON）を動的レンダリング

### システム構成
```
┌──────────────────────────────────────────────────────┐
│                  Universal Player                     │
│              (Next.js - 単一インスタンス)                │
│                                                       │
│  ┌──────────┐  ┌───────────────────────────────────┐  │
│  │ Kernel   │  │        Worker Isolation           │  │
│  │Provided  │  │  ┌────────────┐   ┌────────────┐  │  │
│  │ (Direct) │  │  │Store Comp. │   │ User Comp. │  │  │
│  │          │  │  │ (Worker)   │   │ (Worker)   │  │  │
│  └──────────┘  └───────────────────────────────────┘  │
│                 ※全User/StoreコンポーネントはWorker隔離      │
└────────────────────────┬─────────────────────────────┘
                         │ REST / WebSocket
┌────────────────────────┴─────────────────────────────┐
│                     Rust Kernel                       │
│  ┌────────────┐ ┌────────────┐ ┌──────────────────┐  │
│  │kernel-core │ │kernel-data │ │ kernel-runtime   │  │
│  │(Auth,RBAC) │ │(Entity VFS)│ │ (Deno+Wasmtime)  │  │
│  └────────────┘ └────────────┘ └──────────────────┘  │
│  ┌────────────┐ ┌────────────┐ ┌──────────────────┐  │
│  │kernel-api  │ │kernel-build│ │ kernel-registry  │  │
│  │(Axum)      │ │(SWC Comp.) │ │ (Store/Packages) │  │
│  └────────────┘ └────────────┘ └──────────────────┘  │
└────────────────────────┬─────────────────────────────┘
              ┌──────────┼──────────┐
         PostgreSQL    Redis     S3/MinIO
```

### 技術スタック

| 領域 | 技術 | 選定理由 |
|---|---|---|
| Backend | **Rust** (Axum + SeaORM + Tokio) | 安全性・並行性・サンドボックス制御 |
| Frontend | **Next.js** (Universal Player) | コストスケーラビリティ |
| Auth | **PASETO v4** (public) | 暗号選択ミスのリスク排除 |
| UI Isolation | **Worker + OffscreenCanvas** | OSとしての堅牢性・安全な拡張 |
| Component Build | **SWC** → CDN配信 → Dynamic Import | per-userコンテナ回避 |
| Sandbox (Logic) | **Deno Core** + **Wasmtime** | JS/TS互換 + 高性能バイナリ |
| npm依存解決 | **esm.sh** CDN解決 | サーバー側node_modules不要 |
| App Model | **App is Data** (JSON定義) | スケーラビリティの核 |
| Event Bus | **Redis Streams**（抽象化層あり） | 将来NATS/Kafka移行可能 |
| Database | **PostgreSQL** + RLS | テナント隔離の基盤 |
| Cache | **Redis** | セッション・イベント |
| Storage | **S3/MinIO** | アーティファクト保存 |

---

## 4. コアコンセプト

### 4.1 Everything is Components
アプリは「コード」でも「単なるJSON定義」でもなく、**コンポーネントの集合と設定**である。

- 1アプリ = 1つの **Composition Root** + 複数の **Route Component**
- **構成（Composition）**: コンポーネントの入れ子構造とProps設定が「アプリ」の実体
- AIは隔離環境でコンポーネントを作成・修正し、それを組み合わせてアプリを構築する

### 4.2 3層信頼モデル (Worker-based Isolation)
OSの堅牢性を保証するため、すべてのサードパーティUIは **Web Worker** 内で実行される（Remote Rendering）。Main Threadへの直接アクセスは一切許可されない。

| 層 | 実行方式 | セキュリティ | Kernel API |
|---|---|---|---|
| **Kernel Provided** | **Main Thread** (React) | Host CSP準拠 | 全API |
| **Store Verified** | **Web Worker** (Reconciler) | Worker Sandbox | `data.read/write`, `event.emit` |
| **User Imported** | **Web Worker** (Reconciler) | Worker Sandbox (Network制限) | `data.read` のみ |

- **DOM操作**: 不可。React Reconciler経由でUI記述のみをHostへ送信
- **Canvas**: `OffscreenCanvas` によりWorker内で直接描画
- **DOM計測**: 非同期プロキシAPI経由で取得
- **表現力の確保 (Visual Fidelity)**: Kernelは標準で高品質なUIコンポーネント（Kernel Provided）を提供するが、**完全に独自の世界観**を作りたい場合は `OffscreenCanvas` / WebGL を用いた「Canvas-based Custom Component」の作成を許可する。これにより、OSの進化を待たずにネイティブ級の表現を発明できる。

- **互換性エラー時UX**: `protocol.error` 時は標準フォールバック画面を表示し、キーボード操作・スクリーンリーダー通知・locale fallbackを保証


### 4.3 テナント隔離
テナント境界は3層で強制される:

1. **コンパイル時** — `TenantScoped<T>` ラッパー型により、tenant_idなしのDB操作はコンパイルエラー
2. **ランタイム** — `HMAC署名` 付きトークン + `authorize_tenant()` による安全なコンテキスト設定
3. **DB層** — PostgreSQL RLS `DEFAULT DENY`（未設定時は全行不可視）


### 4.4 コンポーネントライフサイクル
1. **開発/テスト**: 隔離環境（Sandbox）でAIがコンポーネントを作成・即時プレビュー
2. **コンポーネント化**: 動作確認済みのものをライブラリに保存（バージョン管理）
3. **ビルド**: SWCコンパイル + 依存解決 + ロックファイル生成
4. **登録/配布**: ストアまたはプライベートレポジトリに登録
5. **本番適用**: アプリ構成（Manifest）を更新し、検証済みコンポーネントをロード
6. **実行**: Universal PlayerがWorker内で実行。署名・ポリシー検証は必須

### 4.5 イベントシステム
- **At-least-once配信** + Consumer側べき等
- **order_mode**（`entity` / `causality`）に応じた順序保証（`entity_seq` または `causality_seq`）
- **Transactional Outbox** でDB更新とイベント発行の原子性を担保
- **Gap Recovery Protocol**: 30秒タイムアウト → outbox補償読み取り → poison marker → スナップショットリビルド

---

## 5. セキュリティモデル

| 領域 | メカニズム |
|---|---|
| 認証 | PASETO v4 + Argon2id + Refresh Token Rotation |
| テナント隔離 | コンパイル時型強制 + RLS DEFAULT DENY + DBロール分離 |
| UI隔離 | **Web Worker** + React Reconciler + OffscreenCanvas |
| ロジック隔離 | Deno Core + Wasmtime（例: Deno 128MB/5000ms, Wasmtime 64MB/2000ms） |
| コンポーネント改竄防止 | lockfileハッシュ検証 + 署名付きマニフェスト + trust root (`kid` 失効伝播) |
| System Protection | 基幹アプリ（Kernel UI等）には削除保護フラグを設け、誤操作による「文鎮化」を防ぐ。 |
| 特権操作 | SECURITY DEFINER標準テンプレート（`search_path`固定・`REVOKE PUBLIC`）+ 専用DBロール + 監査ログ |


---

## 6. 実装フェーズ

| Phase | 内容 | 主要成果物 |
|---|---|---|
| **1. Foundation** | Cargo workspace、型定義、RLSマイグレーション | `kernel-core`, `kernel-data` |
| **2. Identity** | Auth (PASETO v4)、RBAC、TenantContext middleware + 鍵運用Runbook + SQLガードCI | `kernel-api` |
| **3. Entity** | EntityDefinition/Record CRUD、Schema Evolution、Audit Log + 監査保全/基本バックアップ | Entity VFS |
| **4. Event** | ReliableProducer/Consumer + Outbox + DLQ + 順序保証（`entity_seq`/`causality_seq`） | Event System |
| **5. Component** | SWCビルド、パッケージ管理、S3ストレージ | `kernel-builder`, `kernel-registry` |
| **6. Runtime** | Deno Core + Wasmtime統合、Permission enforcement | `kernel-runtime` |
| **7. Frontend** | Universal Player、3層UI隔離、CDNプロキシ | Next.js App |
| **8. Reliability** | DR (RPO/RTO)、SLO運用、Global Quota、運用自動化（Backup/Rotation/Archive） | Ops / Reliability |
| **9. Ecosystem** | Component Store、Trust Score、審査フロー | Store UI |

---

## 7. ドキュメント構成

| ドキュメント | 内容 |
|---|---|
| **本ファイル** (`flexisuite-concept.md`) | プロジェクトの全体像と設計哲学 |
| [`implementation_plan.md`](./implementation_plan.md) | **[SSOT]** RFC 2119準拠の契約仕様（開発・実装の正本） |
| [`archive/`](./archive/) | Node.js時代のレガシードキュメント |
