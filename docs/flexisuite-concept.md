# FlexiSuite コンセプトドキュメント

> **最終更新**: 2026-02-15
> **対象読者**: プロジェクト参加者、AIエージェント、将来の自分

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
| **エコシステム** | 誰かの成果物をインストールし、AIでさらに改造する。知の循環 |

---

## 2. ビジョンと設計原則

### Mission
**「AI共生時代のアプリケーション開発を、民主化する。」**

### Vision
**「The Safe & Seamless Vibe Coding OS」**

- **Zero Setup** — ブラウザを開けばそこが開発・実行環境
- **AI Native** — 標準化されたインターフェースと安全なサンドボックス
- **Ecosystem** — コンポーネントをストアで配布・購入・インストール

### 3つの設計原則

1. **Safety by Design** — AIがどんなコードを書いても、OSが安全性を担保する（3層信頼モデル）
2. **Scalable by Default** — App is Data, Not Infrastructure（JSON定義 → 動的レンダリング、ユーザーごとのコンテナ不要）
3. **Radical Flexibility within Guardrails** — ガードレールの中で無限の自由（カスタムコンポーネント、外部npm、CDN解決）

---

## 3. アーキテクチャ概要

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
│  ┌──────────┐  ┌───────────┐  ┌───────────────────┐  │
│  │ Kernel   │  │  Store    │  │  User Component   │  │
│  │Provided  │  │ Verified  │  │ (iframe sandbox)  │  │
│  │ (直接)   │  │ (iframe※) │  │                   │  │
│  └──────────┘  └───────────┘  └───────────────────┘  │
│                 ※高信頼スコア時のみ直接昇格              │
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
| Component Build | **SWC** → CDN配信 → Dynamic Import | per-userコンテナ回避 |
| Sandbox (Logic) | **Deno Core + Wasmtime** ハイブリッド | JS/TS互換 + 高性能バイナリ |
| npm依存解決 | **esm.sh** CDN解決 | サーバー側node_modules不要 |
| App Model | **App is Data** (JSON定義) | スケーラビリティの核 |
| Event Bus | **Redis Streams**（抽象化層あり） | 将来NATS/Kafka移行可能 |
| Database | **PostgreSQL** + RLS | テナント隔離の基盤 |
| Cache | **Redis** | セッション・イベント |
| Storage | **S3/MinIO** | アーティファクト保存 |

---

## 4. コアコンセプト

### 4.1 App is Data
アプリケーションはコードではなく**JSON定義**である。単一のUniversal Playerがこの定義を動的にレンダリングする。

- ユーザーごとのコンテナは不要
- コストはDB容量とAPIコール数に比例（線形増加を回避）
- AIはJSON定義を編集するだけでアプリを構築可能

### 4.2 3層信頼モデル

| 層 | 実行方式 | セキュリティ | Kernel API |
|---|---|---|---|
| **Kernel Provided** | 直接実行 | Host CSP準拠 | 全API |
| **Store Verified** | iframe（高信頼時のみ直接昇格） | sandbox CSP | `data.read/write`, `event.emit` |
| **User Imported** | `sandbox="allow-scripts"` | 最小権限 | `data.read` のみ |

### 4.3 テナント隔離
テナント境界は3層で強制される:

1. **コンパイル時** — `TenantScoped<T>` ラッパー型により、tenant_idなしのDB操作はコンパイルエラー
2. **ランタイム** — per-requestトランザクション + `SET LOCAL`（`set_config()` parameterized）
3. **DB層** — PostgreSQL RLS `DEFAULT DENY`（未設定時は全行不可視）

### 4.4 コンポーネントライフサイクル
1. **生成**: AI/開発者がコンポーネントを作成
2. **ビルド**: SWCコンパイル + esm.sh依存解決 + ハッシュ記録（`component.lock`）
3. **審査**: 自動審査（脆弱性・AST解析・バンドルサイズ）+ 条件付き手動レビュー
4. **登録**: 署名付きパッケージとしてKernel Registryに登録
5. **インストール**: テナント単位でインストール。依存解決・ロック生成はKernelが実行
6. **実行**: Universal PlayerがUI配信 / Sandboxがサーバー実行。署名・ポリシー検証は必須

### 4.5 イベントシステム
- **At-least-once配信** + Consumer側べき等
- **entity_seq** による同一Entity内の順序保証（per-entityスコープ）
- **Transactional Outbox** でDB更新とイベント発行の原子性を担保
- **Gap Recovery Protocol**: 30秒タイムアウト → outbox補償読み取り → poison marker → スナップショットリビルド

---

## 5. セキュリティモデル

| 領域 | メカニズム |
|---|---|
| 認証 | PASETO v4 + Argon2id + Refresh Token Rotation |
| テナント隔離 | コンパイル時型強制 + RLS DEFAULT DENY + DBロール分離 |
| UI隔離 | iframe sandbox + nonce handshake + COOP/COEP |
| ロジック隔離 | Deno Core (128MB/5s) + Wasmtime (64MB/2s) |
| コンポーネント改竄防止 | lockfileハッシュ検証 + 署名付きマニフェスト |
| 特権操作 | SECURITY DEFINER関数 + 専用DBロール + 監査ログ |

---

## 6. 実装フェーズ

| Phase | 内容 | 主要成果物 |
|---|---|---|
| **1. Foundation** | Cargo workspace、型定義、RLSマイグレーション | `kernel-core`, `kernel-data` |
| **2. Identity** | Auth (PASETO v4)、RBAC、TenantContext middleware | `kernel-api` |
| **3. Entity** | EntityDefinition/Record CRUD、Schema Evolution、Audit Log | Entity VFS |
| **4. Event** | EventBus + Outbox + DLQ + 順序保証 | Event System |
| **5. Component** | SWCビルド、パッケージ管理、S3ストレージ | `kernel-builder`, `kernel-registry` |
| **6. Runtime** | Deno Core + Wasmtime統合、Permission enforcement | `kernel-runtime` |
| **7. Frontend** | Universal Player、3層UI隔離、CDNプロキシ | Next.js App |
| **8. Ecosystem** | Component Store、審査フロー、Install/Rollback | Store UI |

---

## 7. ドキュメント構成

| ドキュメント | 内容 |
|---|---|
| **本ファイル** (`flexisuite-concept.md`) | プロジェクトの全体像と設計哲学 |
| [`implementation_plan.md`](./implementation_plan.md) | RFC 2119準拠の契約仕様（Codex Review 6ラウンド反映済み） |
| [`archive/`](./archive/) | Node.js時代のレガシードキュメント |
