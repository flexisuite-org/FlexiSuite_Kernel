# FlexiSuite Kernel: Implementation Specification v0.1

> **本文書の区分**: 「比喩」と「契約仕様」を明確に分離する。
> - **太字の MUST / SHOULD / MAY** は [RFC 2119](https://datatracker.ietf.org/doc/html/rfc2119) 準拠の仕様である。
> - それ以外の記述（OS比喩など）は設計意図の説明であり、実装上の制約ではない。

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
  - アプリケーションはトランザクション開始直後に `SET LOCAL flexi.tenant_token = 'signature:ts:nonce:tenant_id'` を実行し、続いて `SELECT flexi.authorize_tenant()` を呼び出さ**なければならない (MUST)**。
  - `authorize_tenant()` 関数内で署名・時刻（許容誤差±5秒）を検証し、**`flexi_nonce` テーブルでNonceの一意性を確認（使用済みなら拒否）** した上で、成功時のみ `flexi.current_tenant` を `SECURITY DEFINER` 権限で設定する。使用済みNonceはテーブルに記録し、5秒経過後に自動削除（または定期削除）する。
  - 直接 `SET flexi.current_tenant` を行うことは禁止される（ロール権限で制限 **かつ (AND)** `authorize_tenant` 以外からの設定値はRLSで無視する多層防御とする）。
- PostgreSQL RLSポリシーは、設定された `current_setting('flexi.current_tenant', true)` を単純比較**しなければならない (MUST)**。
  - 述語: `tenant_id = current_setting('flexi.current_tenant', true)::uuid`
    （※ `current_setting` が未設定または空文字の場合はUUIDキャストエラーによりクエリが失敗するか、不一致により行が返却されない Fail-Closed 構成とする。`IS NOT NULL` 等の冗長なチェックは不要）
- `TenantContext` を受け取らないpublicなDB関数は **存在してはならない (MUST NOT)**。

### 3.2 DB接続の非漏洩設計 (Sealed Architecture)
- 生の `DatabaseConnection` 型は **privateモジュール内に隠蔽しなければならない (MUST)**。外部crateからの直接アクセスを**許可してはならない (MUST NOT)**。
- DB操作の公開インターフェースは `TenantRepository` トレイト（sealed trait）経由のみと**しなければならない (MUST)**。
- SeaORMの `ConnectionTrait::execute_unprepared()` 等の生SQL実行経路は、`TenantScoped` ラッパー内の `pub(crate)` メソッドとしてのみ公開**しなければならない (MUST)**。
- CIパイプラインにおいて、tenant条件を含まないクエリを静的解析で **検出すべきである (SHOULD)**。
- **RLS fail-closed (最終防衛線)**: 静的解析をすり抜けた場合でも、RLSポリシーが未認可アクセスを**拒否しなければならない (MUST)**。`current_setting('flexi.current_tenant', true)` が未設定または不正な形式の場合、UUIDキャストエラーによりクエリが失敗するか、不一致により行が返却されない（Fail-Closed）。

### 3.3 KernelContext (テナント横断操作)
バックグラウンドジョブ、マイグレーション、メンテナンスタスク等、テナント横断アクセスが正当に必要なケースがある。
- テナント横断操作には `KernelContext` を使用**しなければならない (MUST)**。`KernelContext` は `TenantContext` とは別の型であり、明示的に構築**しなければならない (MUST)**。
- **RLSバイパス方式**: カスタムGUCの直接 `SET` は禁止する。代わりに `SECURITY DEFINER` 関数経由でのみバイパスを許可する。
  ```sql
  -- flexi_kernel_admin ロールが所有する SECURITY DEFINER 関数
  CREATE OR REPLACE FUNCTION flexi.enable_kernel_mode(reason TEXT)
  RETURNS VOID AS $$
  BEGIN
    PERFORM set_config('flexi.kernel_mode', 'true', true);  -- local
    -- 監査ログに記録
    INSERT INTO flexi.kernel_audit_log (invoked_by, reason, created_at)
    VALUES (current_user, reason, now());
  END;
  $$ LANGUAGE plpgsql SECURITY DEFINER SET search_path = flexi;

  -- APIロールにはEXECUTE権限を付与しない
  REVOKE EXECUTE ON FUNCTION flexi.enable_kernel_mode FROM flexi_api;
  GRANT EXECUTE ON FUNCTION flexi.enable_kernel_mode TO flexi_kernel_admin;
  ```
- 対応するRLSポリシーに `current_setting('flexi.kernel_mode', true) = 'true'` を OR条件として追加**しなければならない (MUST)**。`flexi_api` ロールからは `SET flexi.kernel_mode` も `enable_kernel_mode()` も実行不可であるため、SQLインジェクション経由の悪用を防止する。
- `KernelContext` を使用する全操作は、操作者・対象・理由を**監査ログに記録しなければならない (MUST)**（上記SQL関数内で自動記録）。
- `KernelContext` はAPIハンドラから直接生成**してはならない (MUST NOT)**。バックグラウンドタスクランナー経由のみで構築可能とし、タスクランナーは `flexi_kernel_admin` ロールの専用コネクションプールを使用**しなければならない (MUST)**。

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
    let token = crypto::generate_signed_token(ctx.tenant_id);
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

#### 実行モデル: Remote Rendering
- **React Reconciler**: Worker内でReactを実行し、Virtual DOMの更新差分のみを軽量JSONとしてMain Threadに送信するカスタムReconcilerを実装**しなければならない (MUST)**。
- **UIコンポーネント**: 開発者は標準的なReact（JSX/Hooks）を使用できる。ただし `window`, `document` への直接アクセスは不可能。
- **例外処理 (Canvas/計測)**:
  - **Canvas**: `OffscreenCanvas` を使用し、Worker内で描画処理を完結させ**なければならない (MUST)**。Main ThreadへはBitmap転送またはplaceholder canvasの制御権委譲を行う。
  - **DOM計測**: `useElementSize`, `useScrollPosition` 等の非同期プロキシフックを提供し、Main Thread側で計測した値をWorkerへ返却する仕組みを実装**しなければならない (MUST)**。

#### 信頼層別ケイパビリティマトリクス (実行環境は全Worker)

| ケイパビリティ | Kernel Provided | Store Verified (Worker) | User Imported (Worker) |
|---|---|---|---|
| Main Thread | **アクセス可** | **不可** (Proxyのみ) | **不可** (Proxyのみ) |
| DOM操作 | 直接操作可 | Reconciler経由 | Reconciler経由 |
| Canvas | 直接操作可 | `OffscreenCanvas` | `OffscreenCanvas` |
| Kernel API | 全API | `data.read/write`, `event.emit` | `data.read` のみ |
| 通信 | 自由 | 宣言済みドメインのみ | 不可 |

#### Store Verified 信頼スコア定義 (AI-Driven Review)
- **スコア算出式** (満点: **90**): `trust_score = (check_pass × 40) + min(verified_installs / 1000, 20) + age_factor + max(20 - report_count × 10, 0)`
  - `check_pass`: AI自動審査(Deep Scan)全項目合格 = 1, 不合格 = 0 (最大: 40)
  - `verified_installs`: **署名検証済み**インストール数（Kernel APIからの正規インストールのみカウント）(最大: 20)
  - `age_factor`: `min(公開日からの経過日数 / 30, 10)` — 新規パッケージの即時昇格防止
  - **減衰**: 30日間インストールがない場合、`verified_installs` を30%減衰**すべきである (SHOULD)**
- **昇格条件**: `trust_score ≥ 80` で "Verified" バッジ付与（※実行環境はWorkerのままで変わらないが、検索順位等で優遇される）。
- **審査プロセス**: 全て **AIエージェントによる自動審査** を基本とする。人間によるレビューは、AIが判定不能とした場合の例外フロー、または事後監査のみとする。

### 4.2 Worker通信プロトコル
WorkerとMain Thread間の通信は厳密に型定義され、検証されたメッセージのみを許可する。

- **Message Schema**: 全ての `postMessage` ペイロードは、事前に定義されたスキーマ（Zod等）でランタイム検証**しなければならない (MUST)**。
- **Transferable Objects**: `ArrayBuffer`, `MessagePort`, `OffscreenCanvas` 等の転送可能オブジェクトを積極的に活用し、コピーコストを削減**すべきである (SHOULD)**。
- **DoS防止**: Workerからのメッセージ頻度・サイズを監視し、規定値を超えた場合はWorkerを強制終了するサーキットブレーカーを実装**しなければならない (MUST)**。

### 4.3 COOP/COEP と SharedArrayBuffer
- `Cross-Origin-Opener-Policy: same-origin` は **Player HTML配信に適用しなければならない (MUST)**。
- `Cross-Origin-Embedder-Policy: require-corp` は **Player HTMLおよびWorker配信面に適用しなければならない (MUST)**。
- Kernel APIレスポンス（JSON API）には COEP を**適用すべきである (SHOULD)**。ただし外部連携で支障がある場合は省略して**よい (MAY)**。
- 外部CDN（esm.sh等）から配信されるアセットには `crossorigin` 属性と、CDN側の `Cross-Origin-Resource-Policy: cross-origin` ヘッダが **必要である (MUST)**。CDNがCORPヘッダを返さない場合、Kernel側のプロキシ経由で配信**しなければならない (MUST)**。

#### CDNプロキシ仕様
CDNが `CORP` ヘッダを返さない場合のフォールバックプロキシの動作を定義する。
- **キャッシュキー**: `{cdn_origin}:{path}:{query_hash}` の決定論的キーを使用**しなければならない (MUST)**。コンテンツアドレスドキャッシュ（SHA-256ハッシュベース）を**併用すべきである (SHOULD)**。
- **キャッシュTTL**: CDN側の `Cache-Control` を尊重し、未指定時は **24h** をデフォルトと**すべきである (SHOULD)**。
- **整合性検証 (ビルドパイプライン側 MUST)**: コンポーネントビルド時に依存モジュールのハッシュを **lockfile (`component.lock`) に記録しなければならない (MUST)**。プロキシはフェッチしたコンテンツのSHA-384をlockfile記録値と照合し、不一致時は配信を**拒否しなければならない (MUST)**。署名付きマニフェスト (`manifest.json.sig`) による改竄検知を**実装しなければならない (MUST)**。
- **整合性検証 (ブラウザ側 SHOULD)**: `<script>` / `<link>` タグで読み込まれるリソースにはSRI属性を**付与すべきである (SHOULD)**。ただし `import()` によるモジュールグラフではブラウザSRIが利用不可のため、ビルドパイプライン側検証を正とする。
- **再検証**: `If-None-Match` / `If-Modified-Since` による条件付きリクエストを**サポートしなければならない (MUST)**。
- **許可リスト**: プロキシ対象CDNは明示的なallowlist制とし**なければならない (MUST)**。初期値: `["esm.sh", "cdn.skypack.dev", "unpkg.com"]`。
- **レスポンスヘッダ**: プロキシは `Cross-Origin-Resource-Policy: cross-origin` を**付与しなければならない (MUST)**。

### 4.4 Store Verified 審査基準

| チェック項目 | 基準 | 自動/手動 |
|---|---|---|
| 既知脆弱性 | `npm audit` 相当、Critical/High = **不合格** | 自動 |
| ネットワークアクセス | `fetch`/`XMLHttpRequest`/`WebSocket` の呼び出し先を宣言制 | 自動検出 + 手動確認 |
| DOM操作範囲 | 自コンポーネントのサブツリー外への操作 = **不合格** | 自動 (AST解析) |
| バンドルサイズ | 上限 **500KB** (gzip後) | 自動 |
| 信頼スコア | 上記チェック結果 + ダウンロード数 + 報告件数の加重スコア | 自動算出 |

### 4.5 Sandbox Runtime 制限

| リソース | Deno Core | Wasmtime |
|---|---|---|
| メモリ | **128MB** per isolate | **64MB** per instance |
| CPU時間 | **5,000ms** per invocation | **2,000ms** per invocation |
| ネットワーク | デフォルト **不可**。`permissions.network` で許可制 | **不可** |
| ファイルシステム | **不可** | **不可** |
| Kernel API | `kernel.data.*`, `kernel.event.emit` のみ | 同左 |

---

## 5. Event System (信頼性契約)

### 5.1 MUST Requirements
- 配信保証: **At-least-once (MUST)**。Consumer はべき等でなければならない。
- 各イベントは一意の `event_id` (UUIDv7) を **含まなければならない (MUST)**。
- 各イベントは同一Entity内の単調増加連番 `entity_seq` を **含まなければならない (MUST)**。
- Consumer は `event_id` による重複排除を **実装しなければならない (MUST)**。
- Consumer は `entity_seq` の順序で処理し、欠番検出時は回復プロトコルに従い**処理しなければならない (MUST)**。

#### entity_seq 採番方式
`SELECT MAX() + 1` はロック競合・空集合エッジケースがあるため、専用カウンターテーブルを使用する。
```sql
CREATE TABLE entity_event_seq (
    entity_id UUID PRIMARY KEY,
    last_seq  BIGINT NOT NULL DEFAULT 0
);

-- 採番: atomic upsert
INSERT INTO entity_event_seq (entity_id, last_seq)
VALUES ($1, 1)
ON CONFLICT (entity_id)
DO UPDATE SET last_seq = entity_event_seq.last_seq + 1
RETURNING last_seq;
```
- outboxテーブルに `UNIQUE(entity_id, entity_seq)` 制約を**設けなければならない (MUST)**。
- **順序保証スコープ**: `entity_seq` による順序保証は **同一 `entity_id` 内のみ (MUST)** とする。異なる `entity_id` 間のグローバル順序は保証しない（シャードが異なる可能性があるため）。
- **Cross-entity因果順序が必要な場合**: オプションとして `causality_key` を定義して**よい (MAY)**。`causality_key` を指定したイベントは `events:{hash(causality_key) % N}` にルーティングされる。
  - **混在禁止**: 同一 `entity_id` に対して `causality_key` 付きイベントと `causality_key` 無しイベントを混在させ**てはならない (MUST NOT)**。混在した場合、同一 `entity_id` のイベントが異なるシャードにルーティングされ、`entity_seq` の順序保証が崩壊するため。`causality_key` を使用する `entity_id` は、当該エンティティの全イベントで同一の `causality_key` を使用**しなければならない (MUST)**。
  - `causality_key` 使用時は、専用の `causality_seq` を採番**しなければならない (MUST)**（`entity_seq` は per-entity であり cross-entity 順序には使用不可）。
  ```sql
  CREATE TABLE causality_event_seq (
      causality_key TEXT PRIMARY KEY,
      last_seq      BIGINT NOT NULL DEFAULT 0
  );
  -- 採番: entity_event_seq と同じ atomic upsert 方式
  ```
  - Consumer は `causality_seq` で順序判定し、gap recovery (§5.4) も同様に適用**しなければならない (MUST)**。
  - `causality_key` 未指定時は `entity_id` がルーティングキーとなり、`entity_seq` が順序キーとなる。

### 5.2 SHOULD Requirements
- **Transactional Outbox Pattern**: DB更新とイベント発行の原子性を担保する**べきである (SHOULD)**。
  - `outbox` テーブルにイベントを書き込み、別プロセスがポーリングしてRedis Streamsに発行。
- **Retry**: 指数バックオフ (初回1s, max 60s, 最大5回) を**適用すべきである (SHOULD)**。
- **Dead Letter Queue (DLQ)**: 最大リトライ超過イベントはDLQに移動し、アラート発火**すべきである (SHOULD)**。

### 5.3 順序保証 (成立条件の明文化)
- 同一Entity内のイベントは `entity_seq` 順で処理を保証**しなければならない (MUST)**。
- **成立条件**:
  - Redis Streamsのキーを `events:{hash(entity_id) % N}` の固定シャード方式とし**なければならない (MUST)**。N は初期値 **64** とし、運用負荷に応じて調整する。同一 `entity_id` は常に同一シャードにルーティングされる。
  - 同一シャード内のイベントは、単一Consumer（Consumer Group内で `XREADGROUP` + 単一consumer名）で処理**しなければならない (MUST)**。
  - Consumer障害時のリバランスは、pending entriesの `XCLAIM` で処理し、順序は `entity_seq` で復元**しなければならない (MUST)**。

### 5.4 Gap Recovery Protocol (欠番回復)
`entity_seq` に欠番が検出された場合、無期限ブロックを防ぐ回復プロトコルを定義する。
1. **タイムアウト**: 欠番検出後 **30秒** 待機しても到着しない場合、回復フェーズに入る **(MUST)**。
2. **補償読み取りとOutbox保持期間**: 
   - `outbox` テーブルのイベント保持期間は **7日間** と**しなければならない (MUST)**。7日を超えたイベントはアーカイブまたは削除してよい。
   - 回復フェーズでは outbox テーブルから該当 `entity_id` + `entity_seq` を直接クエリし、イベントの存在を確認**しなければならない (MUST)**。
3. **判定**:
   - outboxに存在 → Redis Streams配信リトライ（Outboxポーラーに再発行を指示）
   - outboxに不在（かつ作成日時が保持期間内） → **poison marker** として記録し、当該seqをスキップ。
   - outboxに不在（かつ作成日時が保持期間外、または判定不能） → **回復不能 (Unrecoverable Gap)** とみなす。
4. **回復不能時の処置**: poison marker処理ではなく、直ちに **スナップショットリビルド** を発動**しなければならない (MUST)**。古い欠番をスキップしても現在状態との整合性は保証できないため。
5. **スキップ後の状態整合性**: poison markerによるスキップ後、当該Entityの状態は不完全である可能性がある。スキップが発生した場合、当該Entityに対して**スナップショットリビルドワークフロー**を発動**しなければならない (MUST)**:
   - ソースオブトゥルース（Entity Record現在値）から状態を再構築
   - リビルド完了まで当該Entityへの書き込みを一時停止**すべきである (SHOULD)**
   - リビルド完了後、`reconciled_at` タイムスタンプを記録し、以降のイベント処理を再開する

### 抽象化
```rust
#[async_trait]
pub trait EventBus: Send + Sync {
    async fn publish(&self, topic: &str, event: Event) -> Result<(), KernelError>;
    async fn subscribe(&self, topic: &str, handler: EventHandler) -> Result<(), KernelError>;
}
```

---

## 6. Component Composition Schema

「アプリ」は独立した実体ではなく、**コンポーネントの集合と設定**として定義される。

### Composition Model
- **Route Component**: URLルーティングに紐付くトップレベルコンポーネント。
- **Dependencies**: 各コンポーネントが必要とする他コンポーネントやカーネル機能の宣言。
- **Configuration**: コンポーネントツリーに対する外部からのprops注入設定。

### Manifest Example (v1.0)
```jsonc
{
  "schemaVersion": "1.0",
  "id": "app_dashboard_v1",
  "kind": "composition", // app | component
  "name": "My Dashboard",
  "entrypoint": "layout.tsx", // Worker内で実行されるルートコンポーネント
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
- この定義ファイル（`manifest.json`）自体もバージョン管理され、ストアで流通可能な単位となる。
- Universal Playerは、このManifestを解決・ロードし、Worker内で `entrypoint` を実行する。

---

## 7. SLO (Service Level Objectives)

> **計測条件**: 全SLOは以下の条件下で計測する。
> - 同一リージョン内（クライアント ↔ サーバー間ネットワーク遅延を除外）
> - 認証処理込み（トークン検証を含む）
> - コールドスタートは除外（別途 cold start SLO で管理）
> - 計測期間: ローリング7日間

| メトリクス | Target (Prod) | Target (Alpha/Beta) | 計測方法 |
|---|---|---|---|
| API p95 latency (warm) | **≤ 50ms** | ≤ 200ms | Prometheus histogram |
| API p99 latency (warm) | **≤ 200ms** | ≤ 500ms | Prometheus histogram |
| Deno sandbox cold start | **≤ 100ms** | ≤ 300ms | Kernel内部計測 |
| Deno sandbox warm invocation | **≤ 20ms** | ≤ 50ms | Kernel内部計測 |
| Wasm sandbox cold start | **≤ 10ms** | ≤ 50ms | Kernel内部計測 |
| Component build time | **≤ 3s** | ≤ 10s | Builder計測 |
| Event delivery latency | **≤ 500ms** (p95) | ≤ 2000ms | Event Bus計測 |
| Availability | **99.9%** (月間) | 99.0% | Uptime monitor |

#### ベンチマークプロファイル (SLO検証条件)
全SLOターゲットは以下のプロファイル下で pass/fail を判定**しなければならない (MUST)**。

| パラメータ | 値 |
|---|---|
| 同時接続数 | 100 concurrent connections |
| RPS (sustained) | 1,000 req/s |
| ペイロードサイズ | EntityRecord: 4KB JSON (中央値) |
| テナント数 | 50 tenants (均等分散) |
| DB行数 | 100万行/テナント (Entity Records) |
| テスト時間 | 5分間 sustained load |
| ツール | `k6` or `wrk2` (coordinated omission防止) |

---

## 8. Implementation Phases

### Phase 1: Foundation
- Cargo workspace初期化
- `kernel-core`: 型定義、`TenantContext`、`TenantScoped<T>`、エラー型、トレイト
- `kernel-data`: PostgreSQL接続、SeaORM entity、RLSマイグレーション
- **前提**: なし（最初のフェーズ）

### Phase 2: Identity & Access
- Auth: Argon2id + PASETO v4 + Refresh Token Rotation
- RBAC: Role, Permission, GroupMember
- `kernel-api`: 認証エンドポイント、TenantContext middleware
- **前提**: Phase 1（型定義・DB接続）

### Phase 3: Entity System
- EntityDefinition / EntityRecord CRUD
- Schema Evolution (Lazy Migration)
- Audit Log (EntityHistory)
- **前提**: Phase 2（認証・テナントmiddleware）

### Phase 4: Event System
- EventBus trait + Redis Streams実装
- Transactional Outbox
- Retry / DLQ
- 順序保証 (固定シャードルーティング + entity_seq + 単一Consumer)
- **前提**: Phase 3（EntityRecord — イベントの対象）

### Phase 5: Component System
- `kernel-builder`: SWC + esm.sh依存解決
- `kernel-registry`: パッケージ管理
- S3/MinIO artifact storage
- **前提**: Phase 2（認証）、Phase 3（Entity — メタデータ保存）

### Phase 6: Runtime
- `kernel-runtime`: Deno Core統合
- `kernel-runtime`: Wasmtime統合
- Permission model enforcement
- **前提**: Phase 3（Kernel API経由のデータアクセス）、Phase 4（イベント発行）

### Phase 7: Frontend (Universal Player)
- **Worker-based Isolation**:
  - `react-reconciler` によるCustom Renderer実装
  - Workerスレッド管理・メッセージング基盤
  - OffscreenCanvas / DOM計測プロキシ実装
- Component Composition Loader
- COOP/COEP + CDNプロキシ
- Kernel API統合
- **前提**: Phase 5（コンポーネント配信）、Phase 6（ランタイム実行）

### Phase 8: Ecosystem
- Component Store UI
- 審査フロー（自動 + 条件付き手動）
- Install / Update / Rollback
- **前提**: Phase 5（Registry）、Phase 7（Universal Player）
