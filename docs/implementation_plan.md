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

## 2. System Architecture

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
│                 ※初期版はiframe既定、                    │
│                  高信頼スコア時のみ直接昇格              │
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
- API境界の全リクエストハンドラは、認証トークンから `tenant_id` を抽出し、`TenantContext` 構造体に注入**しなければならない (MUST)**。
- `TenantContext` が未設定の状態でDB操作を実行した場合、**パニックではなくコンパイルエラーで防止しなければならない (MUST)**。Rustの型システム（`TenantScoped<T>` ラッパー型）で強制する。
- 全リクエストは **per-requestトランザクション境界内で処理しなければならない (MUST)**。`SET LOCAL flexi.current_tenant` はトランザクション開始直後に **発行しなければならない (MUST)**。トランザクション外での `SET LOCAL` は効果がないため、トランザクション境界なしでのDB操作を**禁止しなければならない (MUST NOT)**。
- PostgreSQL RLSポリシーは、`current_setting('flexi.current_tenant', true)` を全マルチテナントテーブルに **適用しなければならない (MUST)**。`missing_ok=true` により未設定時はエラーではなく `NULL` を返し、述語は `WHERE tenant_id = current_setting('flexi.current_tenant', true)::uuid AND current_setting('flexi.current_tenant', true) IS NOT NULL` の形式と**しなければならない (MUST)**。
- `TenantContext` を受け取らないpublicなDB関数は **存在してはならない (MUST NOT)**。

### 3.2 DB接続の非漏洩設計 (Sealed Architecture)
- 生の `DatabaseConnection` 型は **privateモジュール内に隠蔽しなければならない (MUST)**。外部crateからの直接アクセスを**許可してはならない (MUST NOT)**。
- DB操作の公開インターフェースは `TenantRepository` トレイト（sealed trait）経由のみと**しなければならない (MUST)**。
- SeaORMの `ConnectionTrait::execute_unprepared()` 等の生SQL実行経路は、`TenantScoped` ラッパー内の `pub(crate)` メソッドとしてのみ公開**しなければならない (MUST)**。
- CIパイプラインにおいて、tenant条件を含まないクエリを静的解析で **検出すべきである (SHOULD)**。
- **RLS fail-closed (最終防衛線)**: 静的解析をすり抜けた場合でも、RLSポリシーが未認可アクセスを**拒否しなければならない (MUST)**。`current_setting('flexi.current_tenant', true)` が `NULL` の場合、RLS述語の `IS NOT NULL` チェックにより全行が不可視となる（`DEFAULT DENY`）。

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
    // MUST: parameterized set_config()
    txn.execute(
        Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT set_config('flexi.current_tenant', $1, true)",
            [ctx.tenant_id.to_string().into()],
        )
    ).await?;
    let scoped = TenantScoped::new(txn);
    // MUST: commit前のエラー時は明示的にrollbackを実行し、失敗を監査ログに記録
    match f(&scoped).await {
        Ok(result) => {
            scoped.commit().await?;
            Ok(result)
        }
        Err(e) => {
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

### 4.1 UI隔離: 3層信頼モデル

| 層 | iframe | CSP | DOM/Cookie | Network |
|---|---|---|---|---|
| **Kernel Provided** | なし | Host CSPに準拠 | フルアクセス | Host経由 |
| **Store Verified** | **初期版: iframe既定** | sandbox CSP | アクセス不可 | postMessage |
| **User Imported** | `sandbox="allow-scripts"` | `default-src 'none'; script-src 'unsafe-inline'` | **アクセス不可** | `postMessage` のみ |

- Store Verified は初期版では **iframe内で実行しなければならない (MUST)**。信頼スコアが閾値以上 **かつ** 追加セキュリティ審査 **かつ** 手動レビュー通過時にのみ、直接レンダリングに昇格して**よい (MAY)**。

#### Store Verified 信頼スコア定義
- **スコア算出式**: `trust_score = (audit_pass × 40) + min(verified_installs / 1000, 20) + age_factor + max(20 - report_count × 10, 0)`
  - `audit_pass`: 自動審査全項目合格 = 1, 不合格 = 0
  - `verified_installs`: **署名検証済み**インストール数（Kernel APIからの正規インストールのみカウント）
  - `age_factor`: `min(公開日からの経過日数 / 30, 10)` — 新規パッケージの即時昇格を防止
  - `report_count`: 未解決の違反報告数
  - **減衰**: 30日間インストールがない場合、`verified_installs` を30%減衰**すべきである (SHOULD)**
- **直接レンダリング昇格閾値**: `trust_score ≥ 80` **かつ** 手動レビュー通過 **(MUST)**
- **異常検知**: インストール数の急増（直近24h > 過去30日平均の5倍）はスコア凍結 + 手動確認を**トリガしなければならない (MUST)**
- **失効条件**: 以下のいずれかで iframe に降格し、再審査を**トリガしなければならない (MUST)**:
  - 新バージョン公開時
  - 違反報告が累計3件以上に到達した時
  - 90日間更新がない場合（定期再審査）

#### 信頼層別ケイパビリティマトリクス
| ケイパビリティ | Kernel Provided | Store Verified (iframe) | Store Verified (直接) | User Imported |
|---|---|---|---|---|
| DOM操作範囲 | Host全体 | 自iframe内 | 自サブツリー内 | 自iframe内 |
| ネットワーク | Host経由 | postMessage経由 | 宣言済みエンドポイントのみ | postMessage経由 |
| Kernel API | 全API | `data.read`, `data.write`, `event.emit` | 同左 + `ui.notify` | `data.read` のみ |
| localStorage | フルアクセス | 不可 | namespaced (`pkg:{id}:*`) | 不可 |
| Cookie | フルアクセス | 不可 | 不可 | 不可 |
| WebWorker | 可 | 不可 | 宣言制 | 不可 |

- User Imported iframe の属性は **`sandbox="allow-scripts"` のみを許可しなければならない (MUST)**。`allow-same-origin` は **付与してはならない (MUST NOT)**。

### 4.2 iframe通信プロトコル (srcdoc対応)
`srcdoc` iframe は `origin: "null"` となるため、origin一致検証だけでは不十分。

- Host → iframe 初期化時に、一意の `handshake_nonce` (crypto.randomUUID()) を `srcdoc` 内に埋め込み**なければならない (MUST)**。
- iframe → Host への全 `postMessage` は、`handshake_nonce` をペイロードに含め**なければならない (MUST)**。
- Host は `event.source === iframe.contentWindow` **かつ** `nonce` 一致を検証し**なければならない (MUST)**。
- nonce不一致のメッセージは無視し、セキュリティログに記録**しなければならない (MUST)**。

```typescript
// Host側の検証ロジック
window.addEventListener('message', (event) => {
  if (event.source !== iframeRef.current?.contentWindow) return;
  if (event.data?.nonce !== expectedNonce) {
    logSecurityEvent('nonce_mismatch', event);
    return;
  }
  // 正当な通信として処理
});
```

### 4.3 COOP/COEP 適用範囲
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
2. **補償読み取り**: outboxテーブルから該当 `entity_id` + `entity_seq` を直接クエリし、イベントの存在を確認**しなければならない (MUST)**。
3. **判定**:
   - outboxに存在 → Redis Streams配信リトライ（Outboxポーラーに再発行を指示）
   - outboxに不在 → **poison marker** として記録し、当該seqをスキップして処理を継続**しなければならない (MUST)**。poison markerはアラートを発火**しなければならない (MUST)**。
4. **poison marker上限**: 同一Entity内でpoison markerが **3件** に到達した場合、当該Entityのイベント処理を停止し、手動介入を**要求しなければならない (MUST)**。
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

## 6. App Definition Schema (互換性ルール)

### Versioning
- スキーマバージョンは **独自2桁方式 (MAJOR.MINOR)** に従う **(MUST)**。（SemVer 3桁とは異なる独自運用。）
  - **MINOR** 変更（フィールド追加、オプショナルプロパティ）: Player は後方互換で処理 **(MUST)**。 
  - **MAJOR** 変更（フィールド削除、型変更）: 旧バージョンとの同時サポート期間 **最低90日 (MUST)**。
- PATCHレベルの変更は存在しない（JSON定義にバグフィックスの概念は適用しない）。

### Migration Contract
- MAJOR変更時、Kernel は自動マイグレーション関数を提供**しなければならない (MUST)**。
  - `migrate_app_definition(v1_schema) -> v2_schema`
- マイグレーション関数はべき等で**なければならない (MUST)**。

### Schema Example (v1.0)
```jsonc
{
  "schemaVersion": "1.0",
  "id": "app_xxxxx",
  "name": "My Dashboard",
  "pages": [
    {
      "path": "/",
      "layout": {
        "type": "grid",
        "columns": 2,
        "children": [
          {
            "component": "kernel:chart",
            "props": { "dataSource": "entity:sales" }
          },
          {
            "component": "user:my-widget",
            "props": { "config": "..." },
            "trustLevel": "user"
          }
        ]
      }
    }
  ],
  "actions": {
    "onSaleCreated": {
      "runtime": "deno",
      "entrypoint": "scripts/on-sale-created.ts",
      "permissions": ["data:sales:read", "data:sales:write", "event:emit"]
    }
  }
}
```

---

## 7. SLO (Service Level Objectives)

> **計測条件**: 全SLOは以下の条件下で計測する。
> - 同一リージョン内（クライアント ↔ サーバー間ネットワーク遅延を除外）
> - 認証処理込み（トークン検証を含む）
> - コールドスタートは除外（別途 cold start SLO で管理）
> - 計測期間: ローリング7日間

| メトリクス | Target | 計測方法 |
|---|---|---|
| API p95 latency (warm) | **≤ 50ms** (CRUD) | Prometheus histogram |
| API p99 latency (warm) | **≤ 200ms** (CRUD) | Prometheus histogram |
| Deno sandbox cold start | **≤ 100ms** | Kernel内部計測 |
| Deno sandbox warm invocation | **≤ 20ms** | Kernel内部計測 |
| Wasm sandbox cold start | **≤ 10ms** | Kernel内部計測 |
| Component build time | **≤ 3s** (単一ファイル) | Builder計測 |
| Event delivery latency | **≤ 500ms** (p95) | Event Bus計測 |
| Availability | **99.9%** (月間) | Uptime monitor |

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

### Phase 2: Identity & Access
- Auth: Argon2id + PASETO v4 + Refresh Token Rotation
- RBAC: Role, Permission, GroupMember
- `kernel-api`: 認証エンドポイント、TenantContext middleware

### Phase 3: Entity System
- EntityDefinition / EntityRecord CRUD
- Schema Evolution (Lazy Migration)
- Audit Log (EntityHistory)

### Phase 4: Event System
- EventBus trait + Redis Streams実装
- Transactional Outbox
- Retry / DLQ
- 順序保証 (固定シャードルーティング + entity_seq + 単一Consumer)

### Phase 5: Component System
- `kernel-builder`: SWC + esm.sh依存解決
- `kernel-registry`: パッケージ管理
- S3/MinIO artifact storage

### Phase 6: Runtime
- `kernel-runtime`: Deno Core統合
- `kernel-runtime`: Wasmtime統合
- Permission model enforcement

### Phase 7: Frontend (Universal Player)
- App Definition → 動的レンダリング
- 3層信頼モデル UI隔離実装 (iframe + nonce handshake)
- COOP/COEP + CDNプロキシ
- Kernel API統合

### Phase 8: Ecosystem
- Component Store UI
- 審査フロー（自動 + 条件付き手動）
- Install / Update / Rollback
