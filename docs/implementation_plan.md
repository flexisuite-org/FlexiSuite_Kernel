# FlexiSuite Kernel: Implementation Specification v0.1

> **本文書の区分**: 「比喩」と「契約仕様」を明確に分離する。
> - **太字の MUST / SHOULD / MAY** は [RFC 2119](https://datatracker.ietf.org/doc/html/rfc2119) 準拠の仕様である。
> - それ以外の記述（OS比喩など）は設計意図の説明であり、実装上の制約ではない。
> - **SSOT (Single Source of Truth)**: 本文書はFlexiSuite Kernelの契約仕様の唯一の正本である。他のドキュメント（concept.md等）と矛盾する場合は本文書が優先される。契約変更は本文書の更新を先行させなければならない (MUST)。

---

## 0. 契約トレーサビリティと品質ゲート

本仕様は、要件の「記述」と「検証」を分離しない。高リスク要件は `REQ-*` 識別子を持ち、`docs/verification_matrix.md` に少なくとも1つ以上の**自動検証**（`PR-Blocking` または `Nightly`）を紐付け**なければならない (MUST)**。

- **ゲート層**:
  - `PR-Blocking`（Pull Request必須）: 破壊的回帰を即時に検出する静的検証・契約テスト。
  - `Nightly`（定期実行）: フェイルオーバー、カオス、性能劣化、長時間系の検証。
  - `Operational Drill`（CI外）: DR訓練・ローテーション演習など、実環境相当の手順検証。
- 高リスク `REQ-*` のうち、実地演習が本質の要件（DR/鍵緊急失効など）は `Operational Drill` に加え、実施準備の欠落を検出する `PR-Blocking` の Readiness 検証（Runbook更新日、責任者、次回演習予定日、証跡テンプレート）を持た**なければならない (MUST)**。
- `Nightly` は `PR` のマージ可否を直接ブロックしないが、運用上は必須である。`Nightly` 失敗は24時間以内にインシデント化し、恒久対応または期限付き受容判断を記録**しなければならない (MUST)**。
- `docs/verification_matrix.md` は本仕様の従属ドキュメントであり、仕様変更時は必ず追随更新されなければならない **(MUST)**。
- 高リスク `REQ-*` は、`PR-Blocking` もしくは `Nightly` のどちらか1つのみでは不十分である。実行可能な範囲で両方を持つ構成に**すべきである (SHOULD)**。

| REQ-ID | 要件サマリ | 正本セクション |
|---|---|---|
| `REQ-TENANT-TOKEN-V2` | `tenant_token` は `kid` を含むv2形式で発行し、段階移行を行う | 3.1, 4.6 |
| `REQ-AUTH-SOURCE` | `tenant_token` またはデバッグヘッダからのコンテキスト抽出を強制する | 3.1 |
| `REQ-KEY-REVOCATION-SLO` | 緊急失効は全ノードへ迅速伝播し、旧鍵受理を停止する | 4.6 |
| `REQ-QUOTA-HTTP-CONTRACT` | `429/503` の返却条件と `Retry-After` 算出を固定する | 4.7 |
| `REQ-IDEMPOTENCY-HEADER` | `Idempotency-Key` ヘッダ仕様を固定し、衝突時 `409` を保証する | 4.8 |
| `REQ-IDEMPOTENCY-CONFLICT` | 同一キー・異ボディの衝突を検知し `409 Conflict` を返却する | 4.8 |
| `REQ-PROTOCOL-FALLBACK-UX` | `protocol.error` 後のUIフォールバックを標準化する | 4.2 |
| `REQ-DIAG-CONSENT` | 診断データの既定 `opt-out` と明示同意、撤回即時反映を強制する | 8.2, 8.3 |
| `REQ-MANIFEST-TRUST-ROOT` | 配布マニフェスト署名の信頼ルート・失効・検証順序を固定する | 4.3, 6 |
| `REQ-SUPPLYCHAIN-DIGEST-FORMAT` | ダイジェスト形式（`sha256-`等）を強制する | 4.3 |
| `REQ-SUPPLYCHAIN-DIGEST-MATCH` | アーティファクトとマニフェストのダイジェスト一致を強制する | 4.3 |
| `REQ-SIDELOADING-WARNING` | Developer Mode時の非Verified導入に警告・同意・隔離維持を強制する | 4.3 |
| `REQ-SLO-ENV-PROFILE` | SLO計測環境を固定プロファイルで再現可能にする | 7 |
| `REQ-DR-REHEARSAL` | DRはCIではなく定期演習でRPO/RTO実測を継続する | 9 (Phase 8) |
| `REQ-EVENT-GAP-001` | 欠番検知はアウトボックス/コンシューマ層（Redis Streams等）で非連続なseqを観測した際に行う | 5.5 |
| `REQ-EVENT-GAP-002` | `progress_gap_recovery` がFSMを駆動し、検出されたGapを解消する | 5.5 |

---

## 1. 確定済みアーキテクチャ決定

| 項目 | 決定事項 | 根拠 |
|---|---|---|
| Backend | Rust (Axum + SeaORM + Tokio) | 安全性・並行性・サンドボックスの制御性 |
| Frontend | Universal Player (Next.js 単一インスタンス) | コストスケーラビリティ |
| Auth Token | **PASETO v4** (public) | JWT比で暗号選択ミスのリスク排除 |
| Custom Component | SWC Compile → CDN配信 → Dynamic Import | per-userコンテナ回避 |
| Sandbox (Logic) | Deno Core + Wasmtime ハイブリッド | JS/TS互換 + 高性能バイナリ |
| Sandbox (UI) | 3層信頼モデル | セキュリティとUXの両立 |
| npm依存 | esm.sh CDN解決 | サーバー側node_modules不要 |
| App Model | App is Data (JSON Definition) | スケーラビリティの核 |
| Event Bus | Redis Streams（**抽象化層あり**） | 将来NATS/Kafka移行可能 |

---

- **Minimal Desirable Product (MDP)**: MVP（実用最小限）ではなく、OSとしての「品格（堅牢性・セキュリティ・拡張性）」を備えた状態を指す。スコープ外の機能は実装しないが、スコープ内の機能は妥協なく完遂する。品質不足（不安定、脆弱、拡張困難）は一切許容されない。全ての機能はProduction-Readyでなければならない。

## 2. System Architecture

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

---

## 3. Tenant Isolation (テナント境界強制仕様)

### 3.1 MUST Requirements
- API境界の全**認証済み**リクエストハンドラは、認証トークンから `tenant_id` を抽出し、`TenantContext` 構造体に注入**しなければならない (MUST)**。認証不要な公開エンドポイント（ログイン、ヘルスチェック等）はこの限りではない。
- `TenantContext` が未設定の状態で**保護された**DB操作を実行した場合、**パニックではなくコンパイルエラーで防止しなければならない (MUST)**。Rustの型システム（`TenantScoped<T>` ラッパー型）で強制する。
- **署名付きテナント認証 (Authorize Tenant)**:
  - アプリケーションはトランザクション開始直後に `SET LOCAL flexi.tenant_token = 'v2:kid:ts:nonce:tenant_id:signature'` を実行し、続いて `SELECT flexi.authorize_tenant()` を呼び出さ**なければならない (MUST)**。`signature` は `v2:kid:ts:nonce:tenant_id` に対するHMAC署名とする。
  - **移行互換 (REQ-TENANT-TOKEN-V2)**:
    - 旧形式 `v1:signature:ts:nonce:tenant_id` は、v2導入後の **2リリース期間** に限り検証を許可してよい **(MAY)**。
    - **2リリース期間の定義**: 本番環境向けに発行される連番リリースを2回（例: `kernel-v1.12.0` と `kernel-v1.13.0`）とし、暦日では最大60日を上限とする。2回到達または60日経過のいずれか早い方で互換期間を終了**しなければならない (MUST)**。
    - v2導入時点で新規発行は v2のみとし、v1新規発行を許可してはならない **(MUST NOT)**。
    - 互換期間中は `tenant_token_version_usage_total{version=v1|v2}` を収集し、**本番全リージョンで連続14日間 `v1` 使用件数が0** であることを確認してから v1受理を停止**しなければならない (MUST)**。
  - `authorize_tenant()` 関数内で署名・時刻（許容誤差 **±30秒** 固定）を検証し、**`flexi_nonce` テーブルでNonceの一意性を消費・確認（使用済みなら拒否）** した上で、成功時のみ `flexi.current_tenant` を `SECURITY DEFINER` 権限で設定する。使用済みNonceはテーブルに記録し、短期間（TTL 5分）保持する。
  - **Nonce運用要件**:
    - `flexi_nonce` は `created_at` で日次パーティション化し、期限切れパーティションをdropする運用を採用**すべきである (SHOULD)**。
    - **グローバル一意性の強制 (MUST)**: `nonce` は `created_at` が異なってもシステム全体で一意でなければならない。パーティションテーブルの制約回避のため、`BEFORE INSERT` トリガー等で全パーティションにわたる存在チェックを強制し、既に使用済みの場合は `unique_violation` として拒否しなければならない。
    - 少なくとも `nonce`（一意）と `created_at` のインデックスを持たせ、認証処理における線形探索を発生させてはならない **(MUST NOT)**。
    - TTL削除ジョブ（`pg_cron` もしくは同等の外部ジョブ）は1分間隔で実行し、期限切れNonceを5分以内に回収**しなければならない (MUST)**。
  - **NTPドリフト監視**: **AppノードとDBノード**間の時刻ズレを監視し、1秒以上のドリフトが継続する場合はアラートを発報**すべきである (SHOULD)**。許容窓（Window）は設定可能とする。
  - 直接 `SET flexi.current_tenant` を行うことは禁止される（ロール権限で制限 **かつ (AND)** `authorize_tenant` 以外からの設定値はRLSで無視する多層防御とする）。
- PostgreSQL RLSポリシーは、`flexi.authorized_tenant_id()` の戻り値のみを比較に使用**しなければならない (MUST)**。
  - 述語: `tenant_id = flexi.authorized_tenant_id()`
  - **`flexi.authorized_tenant_id()`**: 副作用のない軽量な参照関数として定義する。`current_setting('flexi.current_tenant', true)` の値を返し、未設定または空の場合は `NULL` を返す。**この関数内で署名検証やDB書き込みを行ってはならない (MUST NOT)**（行ごとの再検証による性能劣化と副作用を防ぐため）。
  - **安全性担保**: `flexi.current_tenant` は `SECURITY DEFINER` である `authorize_tenant()` からのみ設定可能であり、通常のSQLユーザーは `SET` 権限を持たないため、信頼できる値として扱ってよい。
  - Fail-Closed: 未認証時は `NULL` を返し、RLSにより全行が不可視となる。
- `TenantContext` を受け取らないpublicなDB関数は **存在してはならない (MUST NOT)**。

### 3.2 DB接続の非漏洩設計 (Sealed Architecture)
- 生の `DatabaseConnection` 型は **privateモジュール内に隠蔽しなければならない (MUST)**。外部crateからの直接アクセスを**許可してはならない (MUST NOT)**。
- DB操作の公開インターフェースは `TenantRepository` トレイト（sealed trait）経由のみと**しなければならない (MUST)**。
- SeaORMの `ConnectionTrait::execute_unprepared()` 等の生SQL実行経路は、`TenantScoped` ラッパー内の `pub(crate)` メソッドとしてのみ公開**しなければならない (MUST)**。
- CIパイプラインにおいて、tenant条件を含まないクエリを静的解析で **検出すべきである (SHOULD)**。
- **RLS fail-closed (最終防衛線)**: 静的解析をすり抜けた場合でも、RLSポリシーが未認可アクセスを**拒否しなければならない (MUST)**。`flexi.authorized_tenant_id()` が検証に失敗した場合は `NULL` を返し、RLSにより全行が不可視となる（Fail-Closed）。

### 3.3 KernelContext (テナント横断操作)
バックグラウンドジョブ、マイグレーション、メンテナンスタスク等、テナント横断アクセスが正当に必要なケースがある。
- テナント横断操作には `KernelContext` を使用**しなければならない (MUST)**。`KernelContext` は `TenantContext` とは別の型であり、明示的に構築**しなければならない (MUST)**。
- **RLSバイパス方式**: カスタムGUC (`flexi.kernel_mode`) の使用は廃止する。**`SECURITY DEFINER` 関数内でのみ特権操作を完結させなければならない (MUST)**。
  - 特権が必要な操作（例: 全テナント集計、システムメンテナンス）は、それぞれ専用の `SECURITY DEFINER` 関数として実装し、`flexi_kernel_admin` ロールにのみ `EXECUTE` 権限を付与する。
  - 汎用的な "Kernel Mode" フラグは、SQLインジェクションや設定漏れによる権限昇格リスクがあるため、**導入してはならない (MUST NOT)**。
  - `KernelContext` はアプリケーションコード上の概念として保持し、DB層では「特定の特権関数を呼び出す権限」として表現される。
- `KernelContext` を使用する全操作は、操作者・対象・理由を**監査ログに記録しなければならない (MUST)**（特権関数内で自動記録）。
- `KernelContext` はAPIハンドラから直接生成**してはならない (MUST NOT)**。バックグラウンドタスクランナー経由のみで構築可能とし、タスクランナーは `flexi_kernel_admin` ロールの専用コネクションプールを使用**しなければならない (MUST)**。
- **`SECURITY DEFINER` 標準テンプレート**:
  - 特権関数は、必ず標準テンプレート（`SECURITY DEFINER` + `SET search_path = flexi, pg_catalog, pg_temp`）で定義**しなければならない (MUST)**。
  - 組み込み関数・システムテーブル参照は `pg_catalog.` で明示修飾**しなければならない (MUST)**。
  - 作成直後に `REVOKE ALL ON FUNCTION ... FROM PUBLIC` を実行し、必要最小ロールにのみ `GRANT EXECUTE` を付与**しなければならない (MUST)**。
  - 所有ロールは `NOLOGIN` かつ最小権限の専用ロールとし、アプリ接続ロールと共有してはならない **(MUST NOT)**。
  - CIはマイグレーションSQLを検査し、標準テンプレート未適用の `SECURITY DEFINER` 関数を**必ず失敗**させなければならない (MUST)。

### 3.4 実装パターン
```rust
// === モジュール構造 ===
// kernel-data/src/connection.rs (private)
mod connection {
    pub(crate) struct RawConnection(DatabaseConnection);  // 外部非公開
}

// === 公開インターフェース ===
// sealed trait: 外部crateで実装不可
pub trait TenantRepository: private::Sealed {
    async fn get_entity(&self, id: &str) -> Result<EntityRecord, KernelError>;
}

impl TenantRepository for TenantScoped<RawConnection> { ... }

// === per-request transaction ===
pub async fn with_tenant_tx<F, R>(
    pool: &DbPool,
    ctx: &TenantContext,
    f: F,
) -> Result<R, KernelError>
where
    F: FnOnce(&TenantScoped<RawConnection>) -> Future<Output = Result<R, KernelError>>,
{
    let txn = pool.begin().await?;
    
    // MUST: HMAC署名セット & authorize_tenant()による認証実行
    // (複文実行はドライバ依存のリスクがあるため、明示的に2回実行またはパイプライン化する)
    let token = crypto::generate_signed_token_v2(ctx.tenant_id);
    txn.execute(
        Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SET LOCAL flexi.tenant_token = $1",
            [token.into()],
        )
    ).await.map_err(|e| KernelError::TenantAuthorizationFailed(e))?;

    txn.execute(
        Statement::from_string(
            DbBackend::Postgres,
            "SELECT flexi.authorize_tenant()".to_owned(),
        )
    ).await.map_err(|e| KernelError::TenantAuthorizationFailed(e))?;

    let scoped = TenantScoped::new(txn);
    // MUST: commit前のエラー時は明示的にrollbackを実行し、失敗を監査ログに記録
    match f(&scoped).await {
        Ok(result) => {
            match scoped.commit().await {
                Ok(()) => Ok(result),
                Err(commit_err) => {
                    // MUST: commit失敗時、トランザクションの結果は「不明 (Unknown Commit Outcome)」となる。
                    // DB側でcommitされたがACKが返ってこなかった可能性があるため、安易なリトライは危険である。
                    // エラーを呼び出し元に伝播し、監査ログに "COMMIT_UNKNOWN" として記録する。
                    // アプリケーション層は、冪等性が担保された操作であればリトライしてよいが、
                    // そうでない場合は不整合調査が必要となる。
                    tracing::error!(
                        tenant_id = %ctx.tenant_id,
                        "commit failed (outcome unknown): {commit_err}"
                    );
                    Err(KernelError::CommitUnknown(commit_err))
                }
            }
        }
        Err(e) => {
            // アプリケーションエラーによるロールバック
            if let Err(rollback_err) = scoped.rollback().await {
                tracing::error!("rollback failed: {rollback_err}");
            }
            tracing::warn!(tenant_id = %ctx.tenant_id, "tx rolled back: {e}");
            Err(e)
        }
    }
}
```

---

## 4. Security Model (検証可能な強制点)

### 4.1 UI隔離 (Worker-first Architecture)
OSとしての堅牢性を担保するため、サードパーティコード（Store Verified含む）は全て **Web Worker内で実行しなければならない (MUST)**。Main Thread（UIスレッド）への直接アクセスは **許可してはならない (MUST NOT)**。

> **Visual Fidelity Strategy (表現力への回答)**:
> 「Worker隔離＝貧弱なUI」というトレードオフを回避するため、Kernelは **High-Fidelity UI Primitives (Kernel Provided)** をMain Threadに実装し、Workerへ提供する。
> - アニメーション、トランジション、ガラスモーフィズム等の「重い/リッチな」描画は、Kernel提供コンポーネントが担当する。
> - Workerは「何をどう表示するか」の指示（State/Props）のみを送り、実際の描画負荷とセキュリティリスクはMain Thread側の検証済みコードが引き受ける。
> - これにより、**「OSレベルで安全かつ、見た目はPremium」** な体験を保証する。


#### 実行モデル: Remote Rendering
- **React Reconciler**: Worker内でReactを実行し、Virtual DOMの更新差分のみを軽量JSONとしてMain Threadに送信するカスタムReconcilerを実装**しなければならない (MUST)**。
- **UIコンポーネント**: 開発者は標準的なReact（JSX/Hooks）を使用できる。ただし `window`, `document` への直接アクセスは不可能。
- **例外処理 (Canvas/計測)**:
  - **Canvas**: `OffscreenCanvas` を使用し、Worker内で描画処理を完結させ**なければならない (MUST)**。Main ThreadへはBitmap転送またはplaceholder canvasの制御権委譲を行う。
  - **非対応環境フォールバック**: `OffscreenCanvas` が利用できない環境では、機能制限モードとして描画をスキップし、代替プレースホルダを表示する動作を許容して**よい (MAY)**。
  - **非対応環境のUX下限**: `OffscreenCanvas` 非対応時でも、主要操作（再読込/戻る/サポート導線）はキーボードのみで到達可能でなければならず、状態説明はスクリーンリーダーで通知可能にしなければならない **(MUST)**。加えて、`worker_canvas_fallback_total` を計測し、互換性劣化を監視できるようにしなければならない **(MUST)**。
  - **DOM計測**: `useElementSize`, `useScrollPosition` 等の非同期プロキシフックを提供し、Main Thread側で計測した値をWorkerへ返却する仕組みを実装**しなければならない (MUST)**。

#### 信頼層別ケイパビリティマトリクス (実行環境は全Worker)

| ケイパビリティ | Kernel Provided | Store Verified (Worker) | User Imported (Worker) |
|---|---|---|---|
| Main Thread | **アクセス可** | **不可** (Proxyのみ) | **不可** (Proxyのみ) |
| DOM操作 | 直接操作可 | Reconciler経由 | Reconciler経由 |
| Canvas | 直接操作可 | `OffscreenCanvas` | `OffscreenCanvas` |
| Kernel API | 全API | `data.read/write`, `event.emit` | `data.read` のみ |
| 通信 | 自由 | 許可リスト制（要宣言） | **不許可**（Default Deny） |

#### 既存ライブラリ互換性 (Worker Compatibility Matrix)
既存のReactエコシステムに対する互換性と対応策を以下と定義する。

| ライブラリ種別 | 互換性ステータス | 具体例 | 判定根拠 / 対応策 |
|---|---|---|---|
| **Headless UI** | ✅ **検証済み (Verified)** | Radix UI, TanStack Table, Downshift | CI `e2e-headless-compat` で動作確認済み。推奨。 |
| **Logic Only** | ✅ **検証済み (Verified)** | Lodash, Date-fns, Zod | Unit Test で動作確認済み。そのまま使用可。 |
| **Styling** | ✅ **検証済み (Verified)** | Tailwind CSS, CSS Modules | Build Test で動作確認済み。そのまま使用可。 |
| **DOM Dependent** | ⚠️ **要検証 (Experimental)** | Framer Motion, React Spring | `window` 依存部の排除またはPolyfillが必要。E2E検証必須。 |
| **Heavy UI** | ❌ **非対応 (Incompatible)** | Google Maps, Monaco Editor | `window`/`document` への密結合により動作不可。Kernelによる代替提供が必要。 |


#### Store Verified 信頼スコア定義 (AI-Driven Review)
- **スコア算出式**:
  - `trust_score_raw = (check_pass * 30) + verified_installs_score - (uninstall_rate_30d * 100) - (crash_rate_7d * 200) - permission_penalty - decay_penalty`
  - `trust_score = clamp(trust_score_raw, 0, 100)`
  - `verified_installs_score = clamp(log10(weighted_verified_installs + 1) * 10, 0, 30)`
  - `uninstall_rate_30d`, `crash_rate_7d` は **0.0..1.0** の実数で表現し、パーセント表記を混在させてはならない **(MUST NOT)**。
  - `permission_penalty`: ハイリスク権限（Network/Write/Exec）1つにつき **10点減点**。
  - `decay_penalty`: 最終更新から6ヶ月経過で **5点/月減点**、または30日間アクティブ利用ゼロで **10点減点**。
  - 観測母数が `installs_30d < 30` の場合は「暫定スコア」として扱い、自動昇格の判定対象にしてはならない **(MUST NOT)**。
- **昇格条件**: `trust_score ≥ 80` で "Verified" バッジ付与（※実行環境はWorkerのままで変わらないが、検索順位等で優遇される）。
- **審査プロセス**:
  - **自動審査 (Deep Scan)**: 静的解析に加え、サンドボックス内での**動的解析（挙動観測）** を実施**しなければならない (MUST)**。
  - **人手審査 (Manual Gate)**: 以下の条件に該当する場合は、**人手による審査を必須とする (MUST)**。
    - 「高リスク権限（ネットワーク、データ書き込み）」を要求する場合
    - 新規作者の初回パッケージ
    - 短期間でのインストール数急増（バイラル検知）
  - **継続的監視**: 公開後もSBOMベースで依存ライブラリの脆弱性を監視し、問題発覚時は自動的に警告・停止を行う。

### 4.2 Worker通信プロトコル
WorkerとMain Thread間の通信は厳密に型定義され、検証されたメッセージのみを許可する。

- **Message Schema**: 全ての `postMessage` ペイロードは、事前に定義されたスキーマ（Zod等）でランタイム検証**しなければならない (MUST)**。
  - **Handshake**: 接続確立時に `protocol_version` と `capabilities` を交換**しなければならない (MUST)**。
  - **Compatibility**: Kernelは `min_supported_version` を定義し、Worker側のバージョンがこれを下回る場合は `postMessage` の独自エラーイベント `{"type":"protocol.error","code":"PROTOCOL_VERSION_MISMATCH","min_supported":"x.y.z","actual":"a.b.c"}` を送信後に `worker.terminate()` で停止**しなければならない (MUST)**（Fail-Fast）。
  - **Fallback UX (REQ-PROTOCOL-FALLBACK-UX)**: `protocol.error` 発生時、Universal Playerは空白画面にしてはならない **(MUST NOT)**。`min_supported`, `actual`, `request_id` を表示する標準フォールバック画面（再読込導線 + サポート導線）を描画**しなければならない (MUST)**。
  - **Fallback Accessibility/I18n (REQ-PROTOCOL-FALLBACK-UX)**:
    - フォールバック画面は初期フォーカスを見出しへ移動し、主要アクション（再読込/サポート）をキーボードのみで操作可能にしなければならない **(MUST)**。
    - エラー要約は `aria-live="assertive"` で通知し、スクリーンリーダーで `code` と `request_id` を読み上げ可能にしなければならない **(MUST)**。
    - 表示言語は `tenant_locale -> user_locale -> en-US` の順で解決し、未解決時は英語フォールバックを使用しなければならない **(MUST)**。
  - **Error Envelope**: Worker通信のエラーは `type`, `code`, `message`, `request_id`, `timestamp` を含む共通フォーマットで返却**しなければならない (MUST)**。
  - WebSocket経路が存在する場合、WS close code はその経路でのみ利用し、Worker `postMessage` と混同してはならない **(MUST NOT)**。
- **Transferable Objects**: `ArrayBuffer`, `MessagePort`, `OffscreenCanvas` 等の転送可能オブジェクトを積極的に活用し、コピーコストを削減**すべきである (SHOULD)**。
- **DoS防止**: Workerからのメッセージ頻度・サイズを監視し、規定値を超えた場合はWorkerを強制終了するサーキットブレーカーを実装**しなければならない (MUST)**。

### 4.3 COOP/COEP と SharedArrayBuffer
- `Cross-Origin-Opener-Policy: same-origin` は **Document (Player HTML)** に適用**しなければならない (MUST)**。
- `Cross-Origin-Embedder-Policy: require-corp` は **Document** に適用**しなければならない (MUST)**。Worker/Moduleの読み込み成立条件は、配信されるスクリプトレスポンスが `CORP/CORS` 要件を満たすこととする。
- Worker自体にCOOPを要求する実装方針を仕様化してはならない **(MUST NOT)**（ブラウザ実装差異による誤解を防ぐため）。
- **Kernel API (JSON)** レスポンスへの COEP 付与は **推奨される (SHOULD)** が、外部連携等の理由で省略して**よい (MAY)**。
- 外部CDN（esm.sh等）から配信されるアセットには `crossorigin` 属性と、CDN側の `Cross-Origin-Resource-Policy: cross-origin` ヘッダが **必要である (MUST)**。CDNがCORPヘッダを返さない場合、Kernel側のプロキシ経由で配信**しなければならない (MUST)**。

#### CDNプロキシ仕様
CDNが `CORP` ヘッダを返さない場合のフォールバックプロキシの動作を定義する。
- **キャッシュキー**: `{cdn_origin}:{path}:{query_hash}` の決定論的キーを使用**しなければならない (MUST)**。コンテンツアドレスドキャッシュ（SHA-256ハッシュベース）を**併用すべきである (SHOULD)**。
- **キャッシュTTL**: CDN側の `Cache-Control` を尊重し、未指定時は **24h** をデフォルトと**すべきである (SHOULD)**。
- **整合性検証 (ビルドパイプライン側 MUST)**: コンポーネントビルド時に依存モジュールのハッシュを **lockfile (`component.lock`) に記録しなければならない (MUST)**。プロキシはフェッチしたコンテンツのSHA-384をlockfile記録値と照合し、不一致時は配信を**拒否しなければならない (MUST)**。
  - **Distribution Manifest**: 配布用マニフェストには、Semver Rangeではなく**解決済みバージョンと整合性ハッシュ (Digest)** を記載**しなければならない (MUST)**。Range指定は開発時のみ許可される。
  - 署名付きマニフェスト (`manifest.json.sig`) による改竄検知を**実装しなければならない (MUST)**。
- **整合性検証 (ブラウザ側 SHOULD)**: `<script>` / `<link>` タグで読み込まれるリソースにはSRI属性を**付与すべきである (SHOULD)**。ただし `import()` によるモジュールグラフではブラウザSRIが利用不可のため、ビルドパイプライン側検証を正とする。
- **再検証**: `If-None-Match` / `If-Modified-Since` による条件付きリクエストを**サポートしなければならない (MUST)**。
- **許可リスト**: プロキシ対象CDNは明示的なallowlist制とし**なければならない (MUST)**。初期値: `["esm.sh", "cdn.skypack.dev", "unpkg.com"]`。
- **レスポンスヘッダ**: プロキシは `Cross-Origin-Resource-Policy: cross-origin` を**付与しなければならない (MUST)**。
- **Dependency Recovery Protocol (AI-Assisted Repair)**:
  - コンポーネントのリンク切れ（404/410）検知時、Kernelはユーザーに通知し、AIによる「代替コンポーネント探索と置換」の承認を求めるUIを提供**すべきである (SHOULD)**。勝手な置換は行わない。

#### Manifest署名信頼連鎖 (REQ-MANIFEST-TRUST-ROOT)

配布マニフェスト検証における「何を信頼するか」を固定し、実装差による検証抜けを防ぐ。

- **Trust Root 配布**:
  - 信頼ルートは `ops/trust/manifest_trust_root.json`（`version`, `generated_at`, `keys[]`）を正本とし、`keys[]` は `kid`, `alg`, `public_key`, `status(active|retired|revoked)`, `not_before`, `not_after` を含まなければならない **(MUST)**。
  - `manifest_trust_root.json` 自体はオフラインRoot鍵で署名した `manifest_trust_root.json.sig` を伴わなければならない **(MUST)**。
- **検証順序**:
  - Player/Registryは `DistManifest` 受理時に `manifestDigest` の一致を検証し、その後 `manifestSignature` を `kid` で引いた公開鍵で検証しなければならない **(MUST)**。
  - `kid` が `revoked` の場合は即時拒否、`retired` は猶予期間内のみ受理、`active` のみ通常受理とする **(MUST)**。
  - 署名検証失敗・`kid` 不明・有効期限外のいずれかに該当する場合はインストールを拒否し、監査ログへ `MANIFEST_SIGNATURE_INVALID` を記録しなければならない **(MUST)**。
- **鍵ローテーション/失効**:
  - 配布鍵は `active + next` の2世代同時配布を維持し、`next` への切替時は最低24時間の重複検証期間を持たなければならない **(MUST)**。
  - **FlexiSuite Cloud（Managed本番）** では、署名検証を無効化するオプション（`--unsafe-skip-signature-verification` 等）を提供してはならない **(MUST NOT)**。
  - **Self-Hosted環境** では、互換検証/障害対応に限り、`break-glass` 手順として危険オプションを実装してもよい **(MAY)**。ただし以下をすべて満たさなければならない **(MUST)**:
    - 既定値は常に無効であり、明示操作でのみ有効化できること。
    - 有効期間は最大 **60分** とし、期限到達で自動的に無効化されること（恒久化を許可してはならない **(MUST NOT)**）。
    - 適用範囲は `(tenant_id, manifest_digest)` 単位に限定し、システム全体へ一括適用してはならない **(MUST NOT)**。
    - 有効化時に `operator`, `approver`（別主体）, `reason`, `ticket_id`, `expires_at` を必須入力とし、`MANIFEST_SIGNATURE_BYPASS_ENABLED` / `MANIFEST_SIGNATURE_BYPASS_DISABLED` / `MANIFEST_SIGNATURE_BYPASS_USED` を監査ログへ記録すること。
    - 管理UIと運用ダッシュボードで、適用中のバイパスを常時可視化すること。

#### Developer Mode & Sideloading (The Openness Contract, REQ-SIDELOADING-WARNING)
OSSとしての自由とエコシステムの健全性を両立させるため、以下の機能を仕様化する。

- **Developer Mode (Distributed Ecosystem)**:
  - FlexiSuiteは、公式ストアに依存しない**分散型エコシステム (Decentralized Ecosystem)** を推奨する。
  - ユーザーは自身のテナントに対し、明示的な操作（管理画面でのトグル + 再認証 + 警告同意）を経て **Developer Mode** を有効化できる **(MAY)**。

  - 有効化時のみ、`DistManifest` における以下の制限緩和を許可する：
    - **Self-Signed Components**: ストア審査を経ない自己署名コンポーネントのインストール。
    - **Unsigned Components**: 署名なしコンポーネントのインストールは **Self-Hosted環境でのみ** 許可してよい **(MAY)**。FlexiSuite Cloudでは許可してはならない **(MUST NOT)**。
    - **Local Registry**: `localhost` やプライベートネットワーク内のレジストリからのコンポーネント解決。
  - FlexiSuite CloudでDeveloper Modeを有効化した場合でも、署名検証そのものは維持し、テナント明示登録済みの追加信頼鍵（tenant-scoped trust root）でのみ自己署名を受理しなければならない **(MUST)**。
- **Sideloading UX**:
  - 非Verifiedコンポーネント（野良アプリ）のインストール時は、**「信頼されていない発行元」である旨の警告**と、**「Kernelの保証対象外」であることの同意**を求めなければならない **(MUST)**。
  - 警告画面では、要求される権限（Network, etc.）を強調表示しなければならない **(MUST)**。
- **Isolation Enforcement**:
  - Developer Modeであっても、**Worker隔離（Main Threadアクセス禁止）やResource Quota等の「他者への危害」に関わる制限は緩和してはならない (MUST NOT)**。自由は「自分のテナント内」に限定される。

### 4.4 Store Verified 審査基準

| チェック項目 | 基準 | 自動/手動 |
|---|---|---|
| 既知脆弱性 | `component.lock` + SBOM (OSV-based), Critical/High = **不合格** | 自動 + 継続監視 |
| ネットワークアクセス | `fetch`/`XMLHttpRequest`/`WebSocket` の呼び出し先を宣言制 | 自動検出 + 手動確認 |
| DOM操作範囲 | 自コンポーネントのサブツリー外への操作 = **不合格** | 自動 (AST解析) |
| バンドルサイズ | 上限 **500KB** (gzip後) | 自動 |
| 信頼スコア | 下記スコア算出式参照 | 自動算出 (監査可能) |

#### 信頼スコア算出 (Auditability Required)
- 信頼スコアの計算は本章4.1の定義を正本とし、同一式・同一係数で評価**しなければならない (MUST)**。
  - `trust_score = clamp((check_pass * 30) + verified_installs_score - (uninstall_rate_30d * 100) - (crash_rate_7d * 200) - permission_penalty - decay_penalty, 0, 100)`
  - **Verified Tenant Weight**: `weighted_verified_installs` は検証済みテナント（支払実績あり、運用期間3ヶ月以上など）の寄与を重み付けした値を使う。
  - **Sybil Resistance**: 短期間での大量インストール、同一IP/Deviceからの操作は異常検知し、スコア計算から除外または手動審査に回す。
  - **監査**: スコアの算出根拠（入力値、係数、最終スコア、クリップ前後の値）を監査ログとして記録・開示可能でなければならない。

### 4.5 Sandbox Runtime 制限 (Time Limits Contract)
リソース制限はHTTPリクエスト処理とバックグラウンドジョブで異なる適用範囲を持つ。

| リソース | Deno Core | Wasmtime | 適用範囲/備考 |
|---|---|---|---|
| メモリ | **128MB** per isolate | **64MB** per instance | 超過時は即時 `OOM Kill` |
| **CPU時間** | **5,000ms** | **2,000ms** | 純粋な演算時間 (I/O待機を含まない)。超過時は `CPU Limit Exceeded` |
| **Wall Clock** | **30,000ms** | **30,000ms** | HTTPリクエストのタイムアウト (Gatewayで強制) |
| ネットワーク | デフォルト **不可** | **不可** | `permissions.network` で許可リスト制 |
| ファイルシステム | **不可** | **不可** | Kernel API経由のみ許可 |

- **Verification Requirement**: `Nightly` ジョブにおいて、CPU負荷ループとI/O待機（sleep）を組み合わせたテストケースを実行し、CPU時間制限とWall Clock制限が正しく分離して機能することを検証**しなければならない (MUST)**。

### 4.6 Key Management & Rotation Protocol
KMS (Key Management Service) を前提とした鍵ライフサイクルを規定する。

- **Token-KID Contract (REQ-TENANT-TOKEN-V2)**:
  - 署名検証系トークン（`tenant_token` を含む）は `kid` を必須項目として持たなければならない **(MUST)**。
  - 検証器は `kid` で鍵を選択し、`active|next|retired|revoked` 状態を評価**しなければならない (MUST)**。
- **Key States**:
  - `active`: 現在の署名に使用中。
  - `next`: 次期鍵。配布済みだが署名には未使用（検証は可能）。
  - `retired`: 署名には使用不可だが、検証期間（Grace Period）中は有効。
  - `revoked`: 緊急失効。即時に無効化。
- **Auto Rotation**: 30日ごとに自動ローテーションを実施し、`next` へ移行する。
- **Dual Verification Window**: ローテーション後、最低24時間は `retired` 鍵での検証を許可し、伝播遅延による障害を防ぐ。
- **Emergency Revocation**: セキュリティ侵害時は直ちに `revoked` ステータスへ移行し、全インスタンスへ `kid` ブラックリストを配信**しなければならない (MUST)**。
- **Revocation Propagation SLO (REQ-KEY-REVOCATION-SLO)**: `revoked` への遷移から全APIノードでの拒否反映までの遅延は、`p95 <= 60秒` を維持**しなければならない (MUST)**。

### 4.7 Resource Quotas & Fairness (Noisy Neighbor Protection)
テナント間の公平性を担保するため、以下のハードリミットを強制する。

- **Tenant Isolation Limits**:
  - **Concurrent Isolates**: 最大 10 isolates / tenant
  - **CPU Budget**: 1,000ms / sec (Burst 3,000ms) - Token Bucket方式
    - **Note**: CPU Budgetは「CPU使用時間」であり、I/O待機（DB/Network）を含まない。
  - **Memory Hard Limit**: 128MB per isolate (超過時は即時OOM Kill)
- **Control Plane Limits**:
  - **Build Queue**: 最大 5 concurrent builds / tenant
  - **API Rate Limit**: 1,000 req/min (429 Too Many Requests を返却)
- **Global Hard Ceiling**:
  - Tier別プラン（Pro/Enterprise）で上記制限を緩和する場合でも、**システム全体の安定性を守るための「絶対上限 (System Hard Limit)」** を設け、これを突破することは**許可してはならない (MUST NOT)**。
- **優先順位 (衝突解決順)**:
  - 制御判定は `System Hard Limit` → `Tenant Budget` → `API Rate Limit` の順で評価**しなければならない (MUST)**。
  - 上位レイヤで拒否された場合、下位レイヤの判定は行わない（短絡評価）**しなければならない (MUST)**。
- **制御動作**:
  - クォータ超過時は `Retry-After` ヘッダを付与し、以下の判定表に従って `429` または `503` を返却**しなければならない (MUST)**。
  - **HTTP判定表 (REQ-QUOTA-HTTP-CONTRACT)**:

| 判定レイヤ | 返却コード | `Retry-After` 算出 |
|---|---|---|
| `Tenant Budget` 超過 | `429 Too Many Requests` | トークンバケット再充填までの秒数（切り上げ） |
| `API Rate Limit` 超過 | `429 Too Many Requests` | レート窓リセットまでの秒数 |
| `System Hard Limit` 超過 | `503 Service Unavailable` | システム保護窓の最短解除見込み秒数（1-30秒でクリップ） |
| テナント隔離サーキットブレーカー作動 | `503 Service Unavailable` | ブレーカー半開放予定までの秒数 |

  - **Circuit Breaker (Burst Protector) 設計上の注意**:
    - FlexiSuiteのサーキットブレーカーは、エラー率ではなく**リクエスト流量（バースト）**に基づいて作動する。
    - 設定されたトークンバケット（default: 10 req/s, capacity 100）を使い切った場合に「遮断（Open）」状態となる。
    - これは、特定のテナントによる急激なトラフィック増大からシステム全体を保護するための、サーキットブレーカースタイルの復旧挙動（Open → Half-Open → Closed）を持つバースト制限レイヤーである。
  - 特定テナントの負荷がシステム全体に波及する場合、当該テナントの全リクエストをサーキットブレーカーで遮断する権限をKernelが持つ。
- **Client Retry Contract**:
  - SDK/クライアントは `Retry-After` が存在する場合これを最優先し、未指定時のみ指数バックオフ（`base=250ms`, `factor=2`, `jitter=full`, `max=30s`）を用いる**べきである (SHOULD)**。
  - `409 Conflict`（冪等性キー衝突）に対して自動再試行してはならない **(MUST NOT)**。

### 4.8 API Idempotency Contract (CommitUnknown Recovery)
書き込みを行う全APIは、ネットワーク分断による `CommitUnknown` 状態から安全に回復するため、以下の契約に従う。

- **Header Contract (REQ-IDEMPOTENCY-HEADER)**:
  - 書き込みAPI（`POST`, `PUT`, `PATCH`, `DELETE`）は、`Idempotency-Key` リクエストヘッダを受理し、仕様に従って評価**しなければならない (MUST)**。
  - `Idempotency-Key` は 1..128 文字のASCII可視文字とし、制御文字を含めてはならない **(MUST NOT)**。
  - HTTPヘッダ名の解釈は大文字小文字非依存だが、ログ・監査では `Idempotency-Key` を正規表記として記録**しなければならない (MUST)**。
- **Idempotency Scope**: 冪等性キーの一意性は `(tenant_id, method, canonical_request_target, idempotency_key)` の複合タプルで判定**しなければならない (MUST)**。
  - **Canonical Request Target**:
    - `canonical_path`: originを除いたパスを用い、`/` 以外の末尾スラッシュのみ削除する。小文字化・URLデコードは行ってはならない **(MUST NOT)**。
    - `canonical_query`: クエリパラメータはキー昇順、同一キーは値昇順で並べ替える。空クエリは省略し、重複キーは保持する。
    - `canonical_request_target`: `canonical_path` と `canonical_query` を `?` で連結して生成する（クエリが空なら `canonical_path` のみ）。
- **Request Hash Validation**:
  - リクエストごとに `request_hash = SHA256(canonical_request_body)` を計算し、保存されたキーと照合**しなければならない (MUST)**。
  - `idempotency_key` が存在し、かつ `request_hash` が一致しない場合、**409 Conflict** を返却**しなければならない (MUST)**（キーの誤再利用防止）。
  - 空ボディの場合は空文字列のハッシュを使用する。`Content-Type` はハッシュ計算には含めないが、処理ロジックが変わる場合はパス自体を変えるべきである。
- **CommitUnknown**: サーバーからの応答がない（タイムアウト/切断）場合、クライアントは同一キーでリトライ、または結果照会を行わなければならない。
- **Action Handle**:
  - 書き込みAPIは `action_id` (UUIDv7) を生成し、レスポンスヘッダ `X-Action-Id` とレスポンスボディの双方で返却**しなければならない (MUST)**。
  - `action_id` と `idempotency_key` の対応はサーバー側で保持し、クライアントに複合キー再構築を要求してはならない **(MUST NOT)**。
- **Result Inquiry**: 重い処理やすぐにリトライできない場合のために、`GET /actions/{action_id}` エンドポイントを提供し、処理結果（`PENDING | COMPLETED | FAILED`）を返却**しなければならない (MUST)**。
- **Server Behavior**: サーバーは `action_id` と `idempotency_key` の処理結果を **24時間** 保持し、同一キーかつ同一ハッシュの再送に対しては既存 `action_id` の結果を返す（処理の再実行をしない）。
- **Backend Requirement**: multi-instance 本番構成では、冪等性ストアは共有バックエンド（Redis 等）を使用しなければならない **(MUST)**。`InMemoryIdempotencyStore` へのフォールバックは `REQUIRE_REDIS=false` を明示したローカル開発用途に限定し、本番構成で暗黙に許容してはならない **(MUST NOT)**。

---

## 5. Event System (信頼性契約)

### 5.1 MUST Requirements
- 配信保証: **At-least-once (MUST)**。Consumer はべき等でなければならない。
- 各イベントは一意の `event_id` (UUIDv7) を **含まなければならない (MUST)**。
- 各イベントは順序モード `order_mode` を **必ず1つ** 選択**しなければならない (MUST)**。
  - `order_mode = "entity"`: `entity_id` と `entity_seq` が必須。
  - `order_mode = "causality"`: `causality_key` と `causality_seq` が必須。
- `order_mode = "causality"` の場合、`entity_seq` を順序判定に使用してはならない **(MUST NOT)**（参考情報として保持することは許容）。
- Consumer は `event_id` による重複排除を **実装しなければならない (MUST)**。
- Consumer は `order_mode` に対応する順序キー（`entity_seq` または `causality_seq`）で処理し、欠番検出時は回復プロトコルに従い処理**しなければならない (MUST)**。

#### Sequence 採番方式
`SELECT MAX() + 1` はロック競合・空集合エッジケースがあるため、専用カウンターテーブルを使用する。
```sql
CREATE TABLE entity_event_seq (
    entity_id UUID PRIMARY KEY,
    last_seq  BIGINT NOT NULL DEFAULT 0
);

CREATE TABLE causality_event_seq (
    causality_key TEXT PRIMARY KEY,
    last_seq      BIGINT NOT NULL DEFAULT 0
);

-- 採番: atomic upsert (entity)
INSERT INTO entity_event_seq (entity_id, last_seq)
VALUES ($1, 1)
ON CONFLICT (entity_id)
DO UPDATE SET last_seq = entity_event_seq.last_seq + 1
RETURNING last_seq;

-- 採番: atomic upsert (causality)
INSERT INTO causality_event_seq (causality_key, last_seq)
VALUES ($1, 1)
ON CONFLICT (causality_key)
DO UPDATE SET last_seq = causality_event_seq.last_seq + 1
RETURNING last_seq;
```
- outboxには、`order_mode` ごとの一意制約を持たせ**なければならない (MUST)**。
  - `UNIQUE(entity_id, entity_seq) WHERE order_mode = 'entity'`
  - `UNIQUE(causality_key, causality_seq) WHERE order_mode = 'causality'`
- **ルーティング規約 (v1で有効)**:
  - `order_mode = "entity"` のイベントは `events:{hash(format!("{}:e:{}", tenant_id, entity_id)) % N}` にルーティングする。
  - `order_mode = "causality"` のイベントは `events:{hash(format!("{}:c:{}", tenant_id, causality_key)) % N}` にルーティングする。
  - **シャード入力の固定**: マルチテナント間の完全な隔離と名前空間（Entity/Causality）の競合回避のため、シャード計算の入力文字列は `{tenant_id}:{prefix}:{key}` 形式を強制する。
  - 同一 `entity_id` で `entity` モードと `causality` モードを混在させてはならない **(MUST NOT)**。
  - `causality` モードを採用した `entity_id` は、全イベントで同一 `causality_key` を使用**しなければならない (MUST)**。

### 5.2 SHOULD Requirements
- **Retry**: 指数バックオフ (初回1s, max 60s, 最大5回) を適用**すべきである (SHOULD)**。
- **Dead Letter Queue (DLQ)**: 最大リトライ超過イベントはDLQに移動し、アラート発火**すべきである (SHOULD)**。

### 5.3 MUST Requirements (Atomicity & Reliability)
- **Transactional Outbox Pattern**: **アプリケーションの状態変更（Domain Events）を伴う操作**においては、DB更新とイベント発行の原子性を担保**しなければならない (MUST)**。
  - アクセスログやメトリクス等の「失われても整合性に影響しない情報」はこの限りではない。
  - 実装: `outbox` テーブルにイベントを書き込み、同一トランザクション内でコミットする。

### 5.4 順序保証 (成立条件の明文化)
- 順序保証は `order_mode` に対応するキー内でのみ成立**しなければならない (MUST)**。異なるキー間のグローバル順序は保証しない。
- **順序保証スコープ**:
  - Redis Streamsのキーを `events:{hash(ordering_key) % N}` の固定シャード方式とし**なければならない (MUST)**。
  - **N = 64** とし、**v1リリースにおいてはこの値を変更してはならない (MUST NOT)**。将来的に変更が必要な場合は、全データ移行を伴うマイグレーション計画を策定すること。
  - 同一シャード内のイベントは、単一Consumer（Consumer Group内で `XREADGROUP` + 単一consumer名）で処理**しなければならない (MUST)**。
  - **Note parallelization**: スループット向上のため、Consumer内部で `ordering_key` 単位ロック（Lock Striping）を用いた並列処理を行うことは許可されるが、選択された順序キーの整列チェックは厳密に行わなければならない。
  - Consumer障害時のリバランスは、pending entriesの `XCLAIM` で処理し、順序は対応する順序キー（`entity_seq` / `causality_seq`）で復元**しなければならない (MUST)**。
  - **ホットシャード緩和 (v1運用要件)**:
    - `stream_lag_seconds{shard}` が **5分以上** かつ `ingress_rate{shard}` が全シャード中央値の **3倍以上** を15分継続した場合、ホットシャードとして検知しなければならない **(MUST)**。
    - ホットシャード検知時は、当該 `ordering_key` 群に対して優先度付き取り込み制御（低優先度キーへの `429` / 再試行指示）を適用し、全体遅延の拡大を防止しなければならない **(MUST)**。
    - 60分以内に収束しない場合は `SEV-2` 以上でインシデント化し、`N` 変更を含むデータ移行計画の起案を必須とする **(MUST)**。

### 5.5 Gap Recovery Protocol (欠番回復)
`ordering_seq`（`entity_seq` または `causality_seq`）に欠番が検出された場合、無期限ブロックを防ぐ回復プロトコルを定義する。

- **REQ-EVENT-GAP-001**: Gap detection occurs via the outbox/consumer layer when a non-contiguous sequence ID is observed.
- **REQ-EVENT-GAP-002**: `progress_gap_recovery` drives the FSM to resolve detected gaps.

1. **タイムアウト**: 欠番検出後 **30秒** 待機しても到着しない場合、回復フェーズに入る **(MUST)**。
2. **補償読み取りとOutbox保持期間**:
   - `outbox` テーブルのイベント保持期間は、通常イベントは **7日間**、監査・課金等の重要イベントは **90日間** と**しなければならない (MUST)**。保持期間を超えたイベントはアーカイブストレージへ移動し、オンデマンドで復元可能な状態に**すべきである (SHOULD)**。
   - 回復フェーズでは outbox テーブルから該当 `(order_mode, ordering_key, ordering_seq)` を直接クエリし、イベントの存在を確認**しなければならない (MUST)**。
3. **判定 (Gap Recovery State Machine)**:
   | 状態 | 条件 | アクション |
   |---|---|---|
   | **Detected** | seq飛び地を検知 | 30秒待機 (Buffer Period) |
   | **Recovering** | outbox確認: **Found** | Redis再送指示 & 処理再開 |
   | **Skipped** | outbox確認: **Not Found** (期限内) | Poison Marker記録 + `rebuild_required=true` + 当該キー書き込み停止 |
   | **Rebuild** | outbox確認: **Not Found** (期限切れ/不明) | Snapshot Rebuild発動 + 当該キー書き込み停止 |
4. **回復不能時の処置**: `Skipped` または `Rebuild` に遷移した場合、直ちにスナップショットリビルドをキュー投入**しなければならない (MUST)**。
5. **スキップ後の状態整合性**: `rebuild_required=true` の間、当該 `ordering_key` に対する書き込みは停止**しなければならない (MUST)**。
   - ソースオブトゥルース（Entity Record現在値）から状態を再構築。
   - リビルド完了後に `reconciled_at` と `rebuild_required=false` を記録し、以降のイベント処理を再開する。
6. **停止解除SLAとエスカレーション**:
   - `rebuild_required=true` への遷移から **60秒以内** にリビルドジョブを起動しなければならない **(MUST)**。
   - リビルド完了目標は `p95 <= 15分` とし、**30分** を超えて解除できない場合は `SEV-2` 以上でエスカレーションし、手動介入の判断（再投入/部分復旧/一時隔離）を記録しなければならない **(MUST)**。
   - 監視メトリクスとして `event_rebuild_block_seconds` と `event_rebuild_sla_breach_total` を公開しなければならない **(MUST)**。

### 5.6 抽象化
```rust
#[async_trait]
pub trait ReliableProducer: Send + Sync {
    async fn publish(&self, stream: &str, event: EventEnvelope) -> Result<PublishAck, KernelError>;
}

#[async_trait]
pub trait ReliableConsumer: Send + Sync {
    async fn poll(&self, stream: &str, consumer: &str, max_count: usize) -> Result<Vec<Delivery>, KernelError>;
    async fn ack(&self, stream: &str, delivery_id: &str) -> Result<(), KernelError>;
    async fn nack(&self, stream: &str, delivery_id: &str, retry_at: chrono::DateTime<chrono::Utc>) -> Result<(), KernelError>;
    async fn claim_pending(&self, stream: &str, from_consumer: &str, to_consumer: &str, min_idle_ms: u64) -> Result<Vec<Delivery>, KernelError>;
}
```

---

## 6. Component Composition Schema

「アプリ」は独立した実体ではなく、**コンポーネントの集合と設定**として定義される。

### Composition Model
- **Composition Root**: アプリ全体の依存注入・レイアウト境界を担う単一のルート。
- **Route Component**: `path -> component` で紐付くページ単位コンポーネント。1つのCompositionに複数定義できる。
- **Dependencies**: 各コンポーネントが必要とする他コンポーネントやカーネル機能の宣言。
- **Configuration**: コンポーネントツリーに対する外部からのprops注入設定。

### Manifest Examples (v1.0)
開発時と配布時で要件が異なるため、Manifestを2種類に分離する。

- **DevManifest**: 開発者編集用。Semver Rangeを許可。
- **DistManifest**: 配布用。解決済みバージョンとDigestを固定。

#### DevManifest (Range許可)
```jsonc
{
  "schemaVersion": "1.0",
  "id": "app_dashboard_v1",
  "kind": "composition", // composition | component
  "name": "My Dashboard",
  "compositionRoot": "app-shell.tsx", // Worker内で実行される単一ルート
  "routes": [
    { "path": "/", "component": "layout.tsx" },
    { "path": "/sales", "component": "sales-page.tsx" }
  ],
  "dependencies": {
    "components": {
      "chart": "kernel:chart@^1.2.0",
      "datagrid": "store:premium-table@^2.0.0"
    },
    "permissions": [
      "data:sales:read",
      "event:emit"
    ]
  },
  "configuration": {
    "theme": "dark",
    "refreshInterval": 3000
  },
  "security": {
    "integrity": "sha384-...", // パッケージのハッシュ
    "signature": "sig_...",    // 作者またはストアの署名
    "provenance": "https://github.com/user/repo@commit",
    "sbomRef": "./sbom.json"
  }
}
```

#### DistManifest (配布用・固定)
```jsonc
{
  "schemaVersion": "1.0",
  "id": "app_dashboard_v1",
  "kind": "composition",
  "name": "My Dashboard",
  "protected": true, // Userによる削除を防止するSystem Flag
  "compositionRoot": "app-shell.tsx",

  "routes": [
    { "path": "/", "component": "layout.tsx" },
    { "path": "/sales", "component": "sales-page.tsx" }
  ],
  "dependencies": {
    "components": {
      "chart": {
        "source": "kernel:chart",
        "version": "1.2.3",
        "digest": "sha384-abc..."
      },
      "datagrid": {
        "source": "store:premium-table",
        "version": "2.0.4",
        "digest": "sha384-def..."
      }
    },
    "permissions": [
      "data:sales:read",
      "event:emit"
    ]
  },
  "security": {
    "manifestDigest": "sha384-...",
    "manifestSignature": "sig_...",
    "manifestSignatureKid": "store-key-2026-01",
    "trustRootVersion": "2026-02-15"
  }
}
```

### Versioning & Compatibility Policy
- スキーマバージョンは **独自2桁方式 (MAJOR.MINOR)** に従う **(MUST)**。
  - **MINOR** 変更（フィールド追加）: Player は後方互換で処理 **(MUST)**。 
  - **MAJOR** 変更（フィールド削除、型変更）: 旧バージョンとの同時サポート期間 **最低6ヶ月 (MUST)**。
- **Migration Contract**:
  - MAJOR変更時、Kernel は自動マイグレーション関数を提供**しなければならない (MUST)**。
  - 自動変換器の提供がない破壊的変更は**認めてはならない (MUST NOT)**。旧バージョン廃止時には、全ユーザーデータの自動変換バッチを実行**しなければならない (MUST)**。
- **Manifest変換契約**:
  - Build Pipelineは `DevManifest -> DistManifest` への解決・固定化を実行**しなければならない (MUST)**。
  - Player/Registryが受理する配布物は `DistManifest` のみとし、Range指定を含むManifestを本番配布してはならない **(MUST NOT)**。
- この定義ファイル（`manifest.json`）自体もバージョン管理され、ストアで流通可能な単位となる。
- Universal Playerは、このManifestを解決・ロードし、Worker内で `compositionRoot` を起動し、`routes` に従ってRoute Componentを解決する。

---

## 7. SLO (Service Level Objectives)

> **共通計測条件**: 全SLOは以下の条件下で計測する。
> - 同一リージョン内（クライアント ↔ サーバー間ネットワーク遅延を除外）
> - 認証処理込み（トークン検証を含む）
> - 計測期間: ローリング7日間

> **Warm計測条件**:
> - 対象ワーカー/ランタイムが既に初期化済みであること
> - 直前5分以内に同一ワークロード実行実績があること

| Warmメトリクス | Target (Prod) | Target (Alpha/Beta) | 計測方法 |
|---|---|---|---|
| API p95 latency (warm) | **≤ 50ms** | ≤ 200ms | Prometheus histogram |
| API p99 latency (warm) | **≤ 200ms** | ≤ 500ms | Prometheus histogram |
| Deno sandbox warm invocation | **≤ 20ms** | ≤ 50ms | Kernel内部計測 |
| Component build time | **≤ 3s** | ≤ 10s | Builder計測 |
| Event delivery latency | **≤ 500ms** (p95) | ≤ 2000ms | Event Bus計測 |
| Availability | **99.9%** (月間) | 99.0% | Uptime monitor |
| Build Success Rate | **99.5%** | 95.0% | Builder計測 |

> **Cold Start計測条件**:
> - 対象ワーカー/ランタイムが10分以上アイドル、または新規プロセス生成直後であること
> - 1回目呼び出しのみを対象にすること

| Cold Startメトリクス | Target (Prod) | Target (Alpha/Beta) | 計測方法 |
|---|---|---|---|
| Deno sandbox cold start | **≤ 100ms** | ≤ 300ms | Kernel内部計測 |
| Wasm sandbox cold start | **≤ 10ms** | ≤ 50ms | Kernel内部計測 |

#### Error Budget & Alerting Policy
- **Error Budget**: 各SLOに対し `100% - Target` をエラーバジェットとして設定する。
- **Burn Rate Alerting**:
  - **Fast Burn**: バジェット消費速度が異常に速い場合（例: 1時間で月間バジェットの2%消費）、即時ページャー発報（Critical）。
  - **Slow Burn**: 緩やかな悪化傾向（例: 24時間で5%消費）、チケット起票（Warning）。

> **Phase別SLO**: 初期のAlpha/Betaフェーズでは、機能検証と安定化を優先し、レイテンシ目標は緩和値（Target (Alpha/Beta)）を適用する。Prod目標は正式リリース時の必達基準とする。

#### 実行環境プロファイル (REQ-SLO-ENV-PROFILE)
SLOは環境依存でぶれやすいため、測定環境を固定化する。ベンチマーク実行時は `ops/slo_profile.yaml` の値と一致**しなければならない (MUST)**。
`ops/slo_profile.yaml` はインフラ仕様だけでなく、トラフィックミックス・ウォームアップ時間・反復回数も管理対象とし、SLO判定ジョブはプロファイル未一致時に必ず失敗**しなければならない (MUST)**。

| コンポーネント | 固定プロファイル (v1) |
|---|---|
| APIノード | 4 vCPU / 8GB RAM / 1Gbps NIC |
| Workerノード | 8 vCPU / 16GB RAM |
| PostgreSQL | 8 vCPU / 32GB RAM / NVMe (baseline 6000 IOPS) |
| Redis | 4 vCPU / 8GB RAM |
| オブジェクトストレージ | S3互換、同一リージョン、TLS終端有効 |

#### ベンチマークプロファイル (SLO検証条件)
全SLOターゲットは以下のプロファイル下で pass/fail を判定**しなければならない (MUST)**。

| パラメータ | 値 |
|---|---|
| 同時接続数 | 100 concurrent connections |
| RPS (sustained) | 1,000 req/s |
| トラフィックミックス | `read:write:diagnostics = 70:25:5` |
| エンドポイント構成 | `GET /entities` 40%, `GET /entities/{id}` 30%, `POST/PUT/PATCH /entities` 20%, `event.emit` 5%, `diagnostics` 5% |
| ペイロードサイズ | EntityRecord: 4KB JSON (中央値) |
| テナント数 | 50 tenants (均等分散) |
| DB行数 | 100万行/テナント (Entity Records) |
| ウォームアップ | 計測前に2分間のウォームアップを実施（集計対象外） |
| テスト時間 | 5分間 sustained load |
| 反復回数 | 同一条件で5回実行し、4回以上で目標達成時にPass |
| ツール | `k6` or `wrk2` (coordinated omission防止) |

---

## 8. AI Feedback & Self-Correction Interface

AI（エージェント）が自身の生成したアプリのエラーやデザイン崩れを自律的に検知・修正するための「診断インターフェース」を定義する。

### 8.1 Diagnostic Schema
Kernelはエラー発生時や診断要求に対し、以下の構造化データをAIに提供**しなければならない (MUST)**。

```jsonc
{
  "trace_id": "uuid-v7",
  "error_code": "RENDER_ERROR | STYLE_MISMATCH | PERFORMANCE_DEGRADATION",
  "context": {
    "component_id": "cmp_123",
    "props": { ... }, // PII Masked
    "dom_snapshot": "<body>...</body>", // Sanitized & PII Scrubbed
    "metrics": {
      "fcp": 1200,
      "layout_shift": 0.15
    }
  },
  "suggestion": "Check constraint validation for prop 'email'"
}
```

### 8.2 Privacy & PII Scrubbing
- AIへのフィードバックに含まれるDOMスナップショットやPropsは、**PII（個人情報）を自動的にマスキングしなければならない (MUST)**。
  - テキストノード: `***` に置換または汎用プレースホルダへ変換。
  - URL: クエリ文字列とフラグメントを除去し、署名付きURLやトークン文字列を含めてはならない **(MUST NOT)**。
  - 属性: `data-*`, `aria-*`, `class` 等の許可リスト方式で出力し、`value`, `srcset`, `style` 等の高リスク属性は既定で除外**しなければならない (MUST)**。
  - 画像: バイナリデータは含めず、URLは `origin` のみ保持する。`path` はハッシュ化（SHA-256先頭12文字）し、クエリ/フラグメント/署名情報は保持してはならない **(MUST NOT)**。
  - シークレット検知: メール、電話番号、Bearer Token、API Keyパターンの自動検知ルールを適用し、マスク漏れを防止**しなければならない (MUST)**。
- **Payload上限**: 診断データ1件あたりの上限を **512KB** とし、超過時は安全に切り詰めて `truncated=true` を付与**しなければならない (MUST)**。
- **データ境界**: 診断データは一時的なデバッグ目的でのみ保存され、**24時間を超えて保持してはならない (MUST)**。
- **Tenant Consent Contract (REQ-DIAG-CONSENT)**:
  - 診断データのAI提供はテナント単位で **既定 `opt-out`** とし、明示同意が有効化されるまで送信してはならない **(MUST NOT)**。
  - 同意はテナント管理者のみが変更可能であり、撤回時は **5分以内** に新規送信を停止**しなければならない (MUST)**。
  - 診断データは障害診断・自己修復以外（例: 汎用学習データ化）に利用してはならない **(MUST NOT)**。

### 8.3 Feedback API Security
- **Authorization**: 診断APIへのアクセスは、`TenantContext` を持つ認証されたAIエージェント（または開発者）のみに許可**しなければならない (MUST)**。外部公開してはならない。
- **Audit**: AIによる診断要求およびパッチ適用操作は、すべて監査ログに記録**しなければならない (MUST)**。
- **Consent Audit**: 同意の変更（有効化/無効化）は、操作者・理由・時刻・旧値/新値を監査ログに記録し、監査証跡を最低1年間保持**すべきである (SHOULD)**。

### 8.4 Feedback API
- `POST /api/v1/diagnostics/report`: Player/Runtime がサニタイズ済み診断イベントを登録する（生成元はKernel内部コンポーネントのみ）。
- `POST /api/v1/diagnostics/query`: 認可済みAIエージェント/開発者が `trace_id` を指定して詳細診断データを取得する。
- `GET /api/v1/diagnostics/health`: 現在のコンポーネントツリーの健全性スコアを取得する（**認証必須**、`diagnostics:read`）。
- `GET /api/v1/diagnostics/healthz`: 監視/プローブ向けの簡易ヘルス確認を返す（**認証不要**）。
- `GET /api/v1/diagnostics/policy`: テナントの診断同意状態（`enabled`, `updated_at`, `updated_by`）を取得する。
- `PUT /api/v1/diagnostics/policy`: テナント管理者が診断同意状態を更新する（既定 `false`）。
- **Project Rules & Context Layer**:
  - 各プロジェクト/テナントは `.kiro/context.md` (仮) 相当のコンテキストファイルを配置可能とし、AIのエラー修正時の判断基準（Rulebook）として機能させる。


---

## 9. Implementation Phases

### Phase 1: Foundation
- Cargo workspace初期化
- `kernel-core`: 型定義、`TenantContext`、`TenantScoped<T>`、エラー型、トレイト
- `kernel-data`: PostgreSQL接続、SeaORM entity、RLSマイグレーション
- **前提**: なし（最初のフェーズ）

### Phase 2: Identity & Access
- Auth: Argon2id + PASETO v4 + Refresh Token Rotation
- RBAC: Role, Permission, GroupMember
- `kernel-api`: 認証エンドポイント、TenantContext middleware
- **運用安全策（前倒し）**:
  - 手動鍵更新手順（Runbook）とローテーション演習
  - PASETO `kid` 鍵運用Runbook: `docs/auth_paseto_kid_runbook.md`
  - `SECURITY DEFINER` テンプレートのSQLリンタ導入（CI fail-close）
- **前提**: Phase 1（型定義・DB接続）

### Phase 3: Entity System
- EntityDefinition / EntityRecord CRUD
- Schema Evolution (Lazy Migration)
- Audit Log (EntityHistory)
- **運用安全策（前倒し）**:
  - 監査ログの保全ポリシー（WORM相当ストレージへのエクスポート経路）
  - 基本バックアップ（定期スナップショット + 復元手順の定義）
- **前提**: Phase 2（認証・テナントmiddleware）

### Phase 4: Event System
- ReliableProducer / ReliableConsumer 抽象 + Redis Streams実装
- Transactional Outbox
- Retry / DLQ
- 順序保証 (固定シャードルーティング + `entity_seq` / `causality_seq` + 単一Consumer)
- 運用設定ガイド: `docs/event_outbox_redis_producer.md`
- **前提**: Phase 3（EntityRecord — イベントの対象）

### Phase 5: Component System
- `kernel-builder`: SWC + esm.sh依存解決
- `kernel-registry`: パッケージ管理
- S3/MinIO artifact storage
- **前提**: Phase 2（認証）、Phase 3（Entity — メタデータ保存）

### Phase 6: Contract Test & Runtime
- **Contract Verification (REQ-CONTRACT-VERIFY)**:
  - Idempotency正規化・衝突判定ロジックの実装
  - Quota違反時の判定マトリクスとRetry-After付与ロジックの実装
  - Supply Chain マニフェスト・鍵失効チェックの実装
- **Runtime**:
  - `kernel-runtime`: Deno Core / Wasmtime 統合
  - Permission model enforcement
- **前提**: Phase 1-5（基盤・認証・データ）

### Phase 7: Kernel API & Middleware Implementation
- **Goal**: `kernel-core` で定義した契約ロジックを Axum ミドルウェアとして統合し、HTTP フローで強制する。
- **Middleware Chain**: `Auth` → `Idempotency` → `Quota` の順で適用。
- **Security Policy (REQ-AUTH-SEC)**:
  - `X-Tenant-Id` / `X-User-Id` ヘッダは `dev_only` (`feature = "enable_dev_auth"`) とし、既定機能に含めてはならない **(MUST NOT)**。Release ビルドでは当該 feature を有効化してはならず、トークン検証のみを信頼する **(MUST)**。
  - `401 Unauthorized` (Identity 未確定) と `403 Forbidden` (権限/テナント境界不足) を厳格に使い分ける。
- **Idempotency Scope**: `(tenant, method, target, key)` の複合キーで 24h 保持。
- **Quota Priority**: `System Hard Limit` (1-30s clip) → `Tenant` → `API` の順で短絡評価。
- **前提**: Phase 6（契約ロジックおよびランタイム基盤）

### Phase 8: Frontend (Universal Player)
- **Worker-based Isolation**:
  - `react-reconciler` によるCustom Renderer実装
  - Workerスレッド管理・メッセージング基盤
  - OffscreenCanvas / DOM計測プロキシ実装
- Component Composition Loader
- COOP/COEP + CDNプロキシ
- Kernel API統合
- **前提**: Phase 5（コンポーネント配信）、Phase 7（Kernel API/ミドルウェア）

### Phase 9: Platform Reliability
- **Operations**:
  - Backup & Restore Drills の自動化
  - Secret Rotation の自動化
  - Audit Log Archiving の自動化（長期保管）
- **Disaster Recovery**:
  - RPO (Recovery Point Objective): 5分 (WAL archiving)
  - RTO (Recovery Time Objective): 1時間
  - Region Failover Playbook
  - **演習ポリシー (REQ-DR-REHEARSAL)**: DR検証はCIで常時実行してはならない **(MUST NOT)**。代わりに、ステージングで月次演習・本番相当環境で四半期演習を実施し、実測RPO/RTOを記録**しなければならない (MUST)**。
  - **Readiness Gate**: CIでは実演習を実行せず、`Runbook最終更新日`、`責任者`、`次回演習予定日`、`前回演習結果リンク` の4点を `PR-Blocking` で検証しなければならない **(MUST)**。
- **Fairness & Stability**:
  - Tenant Quota Enforcement (Rate Limiter / Circuit Breaker)
  - Global Hard Ceiling Implementation
  - Error Budget / Burn Rate Alerting Setup
- **前提**: Phase 7（全機能実装後の安定化フェーズ）

### Phase 10: Ecosystem
- Component Store UI
- 審査フロー（自動 + 条件付き手動）
- Install / Update / Rollback
- Trust Score Logic Implementation (with Anti-Sybil)
- **前提**: Phase 9（信頼性担保された基盤）

### Phase 11: Future Capabilities (Kernel Capabilities)
- **Kernel Capabilities (RPC Proxy)**: Hardware API (Bluetooth/WebXR等) への安全なプロキシ
- **P2P / Local Network Bridge**: WebRTC / Local Network デバイス連携
- **前提**: Phase 1-10 完了後のロードマップ

---

## 10. 実行体制 (RACI)

| 領域 | Responsible | Accountable | Consulted | Informed |
|---|---|---|---|---|
| 鍵運用・失効 | Security Engineer | Security Lead | Backend Lead, SRE | PM |
| テナント隔離・RLS | Backend Engineer | Backend Lead | DBA, Security | QA |
| SLO/クォータ運用 | SRE | SRE Lead | Backend Lead | PM |
| Worker互換性/UX | Frontend Engineer | Frontend Lead | Designer, QA | Support |
| 診断同意/監査 | Product Engineer | Product Lead | Security, Legal/Privacy | Customer Success |
| DR演習 | SRE + DBA | SRE Lead | Security, Backend | PM, Exec |

## 11. 仕様変更ワークフロー (Kiro準拠)

- 仕様変更は **Requirements → Design → Tasks → Implementation** の順に承認を通過しなければならない **(MUST)**。
- `-y` によるFast-trackは緊急修正時のみ使用し、事後24時間以内に通常ドキュメントを補完**しなければならない (MUST)**。
- `REQ-*` の追加・削除・意味変更は、同一PR内で `docs/verification_matrix.md` の追随更新を伴わなければならない **(MUST)**。

---

## 12. Future Proposals (RFC) - Unimplemented
以下はUX課題解決のための「提案段階」の機能であり、現行実装には含まれず、開発確約も行わない。実装には新たなSpecs策定と承認が必要である。

| 提案機能 | 目的 | ステータス |
|---|---|---|
| **Shared State Hooks** | `useSharedState` により Main Thread/Worker 間のゼロコピーに近い高速同期を実現する。 | **提案 (Proposal)**: Phase 7+ 検討 |
| **Kernel StdLib** | Google Maps等、Workerで動作しない重要ライブラリをKernel特権でホストし、`<StdMap />` 等のラッパーを提供する。 | **提案 (Proposal)**: Phase 9+ 検討 |
| **Optimistic SDK** | `useDataMutation` に楽観的UIロジックを内蔵し、AIがロールバック処理を書かずに済むようにする。 | **PoC待ち**: SDK設計で検証 |
| **Time Travel Replay** | エラー時の操作ログと状態を保存し、サンドボックス内で再現実行可能にするデバッグ基盤。 | **提案 (Proposal)**: Phase 8+ 検討 |
| **Cron / Job Scheduler** | 常駐プロセスの代替としての定期実行ジョブ基盤。 | **要設計 (Draft)**: 未定 |
| **Cross-App Bridge (IPC)** | アプリ間でのデータ連携や機能呼び出しを、権限管理されたチャネルを通じて実現する。 | **要設計 (Draft)**: 未定 |
| **Payment API** | ストアおよびアプリ内課金のための標準決済APIを提供する。 | **要設計 (Draft)**: 未定 |
| **Data Export (Portability)** | テナントデータとアプリ定義を一括zipエクスポート/インポートする標準機能。 | **Phase 8+** |
