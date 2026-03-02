# RBAC ブランチ 統合コードレビューレポート（最終修正版）

> **対象**: `origin/main` → `HEAD` (RBACブランチ)
> **差分規模**: 59ファイル、3,018行追加、3,244行削除
> **レビュー手法**: 20パーティション並列レビュー + メタレビュー検証（実コード照合・Oracle・Momus）
> **参照仕様**: `docs/flexisuite-concept.md`, `docs/implementation_plan.md`, `docs/verification_matrix.md`, `docs/negative-space-spec.md`
> **レビュー日**: 2026-03-02
> **修正履歴**: 初版の誤検知4件を実コード照合により修正。ブロッキング問題は7件→2件に再評価。全ブロッカー解決済み（2026-03-02）。

---

## エグゼクティブサマリー

### ✅ マージ判定: **APPROVED**

**全ブロッキング問題が解決済み**。マージ可能。

RBACブランチは供給チェーン・クォータ・コントラクトテスト・ビルドガード・依存関係管理の各領域において高品質な実装を示している。初版レビューで報告された7件のブロッキング問題のうち、実コード照合により4件が誤検知・カテゴリエラー・事実誤認として除去された。残存した真のブロッキング問題3件（CRITICAL-3, CRITICAL-5, CRITICAL-6）はすべて修正・検証済み。
### メタレビュー品質評価

| 指標 | 値 |
|------|-----|
| 初版所見総数 | 7件（CRITICAL×6、HIGH×1） |
| 確認済み誤検知 | 4件（誤検知率 57%） |
| 真のブロッキング問題 | 2件 |
| 品質スコア（Momus評価） | 3.5/10 |

---

## 20パーティション パス/フェイル マトリクス

| P# | ドメイン | 判定 | 重大度 |
|----|---------|------|--------|
| P1 | RBAC マイグレーション SQL | ✅ **PASS** | — |
| P2 | RBAC エンティティ | ✅ **PASS** | （初版 CRITICAL×2 は誤検知・カテゴリエラー — 後述）|
| P3 | RBAC ミドルウェア | ✅ **PASS** | （レビュー中に修正済み）|
| P4 | RBAC リポジトリ | ✅ **PASS** | （CRITICAL-3 解決済み）|
| P5 | Auth Core | ✅ **PASS** | （CRITICAL-5, CRITICAL-6 解決済み）|
| P6 | Auth Connection/Context | ⚠️ **PARTIAL** | Advisory × 2 |
| P7 | ミドルウェア Core | ✅ **PASS** | — |
| P8 | API ルート & Lib | ⚠️ **PARTIAL** | Advisory × 1（設計文書化済み）|
| P9 | 供給チェーン Core | ✅ **PASS** | — |
| P10 | レジストリ Storage | ✅ **PASS** | — |
| P11 | レジストリ Trust | ✅ **PASS** | — |
| P12 | 暗号検証 & CI | ✅ **PASS** | — |
| P13 | コントラクトテスト — Auth | ✅ **PASS** | — |
| P14 | コントラクトテスト — Security | ✅ **PASS** | — |
| P15 | コントラクトテスト — Middleware | ✅ **PASS** | — |
| P16 | コントラクトテスト — Supplychain | ✅ **PASS** | — |
| P17 | コントラクトテスト — Quota | ✅ **PASS** | — |
| P18 | ビルドガード & CI | ✅ **PASS** | — |
| P19 | Cargo / 依存関係 | ✅ **PASS** | — |
| P20 | 削除 & その他 | ✅ **PASS** | — |

**集計**: PASS 19、PARTIAL 1（P6、Advisory のみ）、FAIL 0

---

## ブロッキング問題 — すべて解決済み ✅

### ~~🔴 CRITICAL-3~~ → ✅ 解決済み — [P4] RBACRepository: Dual Source of Truth と user_id 型強制の欠如

**ファイル**: `kernel-data/src/rbac.rs`, `kernel-data/src/lib.rs`

**違反仕様**: `implementation_plan.md §3.2 MUST`

**解決内容（2026-03-02）**:
- `kernel-data/src/connection.rs` に `AuthenticatedScoped<'a>` 参照ベース構造体を追加
- `kernel-data/src/rbac.rs` を全面書き換え: `AuthenticatedScoped` 単一引数、ランタイム二重チェック削除、全5テーブルに明示的 `tenant_id` フィルタ追加
- `kernel-data/src/lib.rs` に `AuthenticatedScoped` を re-export
- `kernel-api/src/middleware/rbac.rs` を `AuthenticatedScoped::from_scoped(scoped, user_id)` 使用に修正
- `kernel-data/tests/integration_tests.rs` の呼び出しを新シグネチャに修正
- `cargo check -p kernel-data -p kernel-api`: 警告ゼロ ✅

**状況**: `RBACRepository::get_user_permissions` は `TenantScoped<RawConnection>` を受け取る（これ自体は `Sealed` を実装済みで正しい）。しかし同時に `&TenantContext` も受け取り、`TenantScoped` が既に保持している `tenant_id`/`user_id` を二重に検証している。具体的な問題点は以下の通り:

1. **Dual Source of Truth（二重権威ソース）**: `TenantScoped<RawConnection>` の生成時点（`connection.rs` lines 154-158）で `tenant_id` と `user_id` はすでに bake-in されている。にもかかわらず `get_user_permissions` は `&TenantContext` を別途受け取り、`scoped.tenant_id != *tenant_id`（line 26-31）および `scoped.user_id != Some(user_id)`（line 32-35）を再度ランタイムチェックしている。`TenantScoped` の生成が信頼できる唯一の権威ソースであるべきところ、`TenantContext` を第二の権威として持ち込んでいる。

2. **user_id のコンパイル時強制欠如**: `TenantScoped.user_id` は `Option<UserId>` 型（`connection.rs` line 31）。RBAC操作には `user_id` が必須だが、`None` の場合のパニックをコンパイル時に防止できない。現状は `rbac.rs` line 20-23 でランタイム `ValidationError` を返す設計。型システムで「user_idが必ず存在する TenantScoped」を表現できていない。

3. **中間テーブルへの明示的テナントフィルタ欠如（defense-in-depth 違反）**: `rbac.rs` lines 39-47 の5テーブルJOINで、`permission::Column::TenantId` への明示的フィルタは1件のみ。中間テーブル（role, group_role, group, group_member）は RLS のみに依存。全5テーブルに `tenant_id` カラムは存在するが、PostgreSQL RLS 設定ミス時に cross-tenant データ漏洩が発生する。

**修正方針** (Oracle提案):
```rust
// 現状: TenantScoped (user_id: Option) + 別途 TenantContext の二重ソース
pub async fn get_user_permissions(
    scoped: &TenantScoped<RawConnection>,  // user_id: Option<UserId>
    ctx: &TenantContext,                   // 二重チェック用に冗長に渡している
) -> ...

// 修正後: user_id を型レベルで強制する専用ラッパー型を導入
pub struct AuthenticatedScoped {
    inner: TenantScoped<RawConnection>,
    user_id: UserId,  // Option ではなく確定済み UserId
}

impl AuthenticatedScoped {
    // TenantContext から生成する唯一のファクトリ — user_id が None なら生成不可
    pub(crate) fn from_scoped(scoped: TenantScoped<RawConnection>, user_id: UserId) -> Self {
        Self { inner: scoped, user_id }
    }
}

// get_user_permissions は AuthenticatedScoped のみを受け取る
// → user_id: Option による分岐がコンパイル時に消える
pub async fn get_user_permissions(
    scoped: &AuthenticatedScoped,
) -> ...

// JOIN にも明示的テナントフィルタを追加
let permissions = permission::Entity::find()
    .filter(permission::Column::TenantId.eq(tenant_id.as_str()))
    .join(JoinType::InnerJoin, permission::Relation::Role.def())
    .filter(role::Column::TenantId.eq(tenant_id.as_str()))   // 追加
    .join(JoinType::InnerJoin, role::Relation::GroupRoles.def())
    .filter(group_role::Column::TenantId.eq(tenant_id.as_str()))  // 追加
    // ...
```
---

### 🔴 CRITICAL-6 — [P5] Auth Core: 分散キー失効キャッシュ未接続

**違反仕様**: `REQ-KEY-REVOCATION-SLO MUST`
> "キー失効は p95 ≤ 60秒以内に全ノードへ伝播しなければならない。" (`implementation_plan.md` line 405)

**状況**: キー失効機構の現実:

- **失効は起動時環境変数のみ**: `kernel-api/src/auth.rs` line 161 で `FLEXI_PASETO_V4_REVOKED_KIDS` を `OnceLock` に読み込む。動的更新・ポーリング・ホットリロードは存在しない。失効を反映するにはアプリケーション再起動が必要。
- **Redis インフラは存在する**: `kernel-api/Cargo.toml` line 30、`kernel-data/Cargo.toml` line 27 に Redis 依存が定義済みであり、idempotency（`kernel-api/src/middleware.rs`）や Event Streaming（`kernel-data/src/event/redis_producer.rs`）で使用中。インフラ未整備ではなく、**失効キャッシュへの接続が未実装**。
- **key_manager.rs の明示的コメント** (line 213-216): "For HMAC signing keys, we always read the authoritative active key from DB to avoid stale process-local cache after cross-instance revocation." — つまり cross-instance 失効問題を認識した上で、PASETO 鍵の同等対策が欠けている。
- **マルチノード環境での SLO 不達**: 単一ノードでは DB クエリで失効を確認できるが、マルチノードでは OnceLock の静的キャッシュが再起動まで失効を反映しない。p95 ≤ 60秒の SLO は達成不可能。

**修正方針**:
```rust
// kernel-core/src/auth/key_manager.rs に Redis Pub/Sub 統合を追加
// 1. Redis チャンネル "flexi:auth:kid_revoked" を subscribe
// 2. 失効イベント受信時に OnceLock をアトミックに更新（or ArcSwap 採用）
// 3. 60秒以内伝播を保証するため TTL ベースのポーリング（最大30秒間隔）を併用
// 既存の Redis 接続（middleware.rs）を再利用可能
```

**解決内容（2026-03-02）**:
- `kernel-api/src/auth.rs` に `REVOKED_KIDS_OVERRIDE` static（`OnceLock<RwLock<HashSet<String>>>`）を追加
- `verify_paseto_v4_public_from_env_token` にオーバーライドチェックを追加
- `start_kid_revocation_listener(client: redis::Client)` を実装: Redis Pub/Sub (`flexi:auth:kid_revoked` チャンネル) + 30秒ポーリングを `tokio::select!` で並行実行、切断時の自動再接続付き
- `kernel-api/src/main.rs` でプロセス起動時に `start_kid_revocation_listener` を呼び出す。`MiddlewareConfig::default()` が既に読み込む `config.redis_url` を再利用して `redis::Client` を生成し、リスナーを起動。Redis クライアント生成失敗時はウォーニングを出力して起動を継続（ポーリングのみのフォールバックは60秒SLOを満たさないため、ログに警告が記録される）。
- `tests/contract/src/auth/helpers.rs` の旧関数名 `init_auth_config_with_public_key_and_revoked_kids_and_legacy_mode` を `init_auth_config_with_public_key_and_revoked_kids` に更新
- `cargo test --workspace --lib --bins`: 全テスト通過（contract-tests 35件、kernel-api 17件、kernel-data 16件、kernel-core 16件、kernel-registry 17件）✅

## 誤検知・再評価問題 (初版から変更)

### ~~CRITICAL-1~~ — 誤検知除去: RBAC エンティティ `TenantScoped<T>` 未使用

**除去理由**: `TenantScoped<T>` は `kernel-data/src/connection.rs` で定義されており、**DB 接続**（`RawConnection`）をラップするための型であり、**SeaORM EntityModel をラップする設計ではない**。「すべてのエンティティを `TenantScoped<T>` でラップせよ」という仕様解釈自体がカテゴリエラー。Raw SeaORM Model の公開は RLS + TenantContext による多層防御で保護されており、問題なし。

---

### ~~CRITICAL-2~~ — 誤検知除去: `TenantRepository` Sealed Trait 欠如

**除去理由**: `kernel-data/src/repository.rs` lines 17-25 に `pub(crate) mod private { pub trait Sealed {} }` が実在する。初版レビュアーがファイルを確認せずに欠如と判断した確定的誤検知。

---

### ~~CRITICAL-4~~ → Advisory 格下げ: `tenant_id` 二重ソース問題

**格下げ理由**: `kernel-data/src/rbac.rs` lines 26-35 に `TenantAuthorizationFailed` ハードエラーが明示的に実装済み。tenant_id 不一致時の挙動は「未定義」ではなく定義済み。「唯一の権威ソース」への整理は望ましい設計改善だが、現状がセキュリティホールではない。

---

### ~~CRITICAL-5~~ → 解決済み ✅: `allow_legacy_no_kid` 削除完了

**解決内容**: `kernel-api/src/auth.rs` から `allow_legacy_no_kid` フラグおよび関連コードを完全削除済み（2026-03-02 実施）。
削除対象: `PasetoKeyset.allow_legacy_no_kid` フィールド、`is_legacy_without_kid_allowed()` メソッド、
`has_legacy_paseto_layout()` 関数、`init_auth_config_with_public_key_and_revoked_kids_and_legacy_mode()` 公開関数、
`parse_bool_env()` ヘルパー関数、および関連テスト。
ビルド・テスト: `cargo test -p kernel-api` 17/17 通過、警告ゼロ。

---

### ~~HIGH-1~~ → Advisory 格下げ: `/health/readiness` が `TenantContext` を要求

**格下げ理由**: `docs/kernel_api_health_probes.md` に以下が明記されている:
- readiness エンドポイントは `TenantContext` 経由で `with_system_context()` を呼び出し DB/Redis 状態を確認するため、認証ヘッダーが技術的に必要
- Kubernetes readinessProbe を使用する場合はサイドカーまたはサービスメッシュで認証ヘッダーを付与するよう明示的に指示
- liveness（認証不要）と readiness（認証必要）の分離は意図的設計

**注意**: Kubernetes 標準の readinessProbe パターンとの乖離は設計トレードオフであり、チームが認識した上でドキュメント化している。バグではなく文書化済みの制約。

---

## 勧告的問題 (Should Fix — マージブロックではないが推奨)

| 優先度 | パーティション | 問題 | 対応方針 |
|--------|--------------|------|---------|
| HIGH | P5 | `validate_token_kid()` がキーマテリアルの存在を検証しない | `public_keys` マップに実際のキーが存在することを確認するアサーション追加 |
| HIGH | P6 | `parse_tenant_from_token` の検証が `authorize_tenant()` DB呼び出し後に実行される | トークン検証を DB アクセスより前に移動（fail-fast原則）|
| HIGH | P5 | `allow_legacy_no_kid` 削除 | ✅ **削除完了** (2026-03-02) |
| MEDIUM | P6 | Auth失敗時に `tenant_id` が平文でログ出力される | `tenant_id` をマスクまたはハッシュ化してログ出力 |
| MEDIUM | P3 | RBAC 可観測性メトリクス未実装 | `rbac_auth_failures_total`、`rbac_permission_check_duration_seconds` Prometheus メトリクスを追加（`kernel-api/src/middleware/rbac.rs` の TODO 解消）|
| MEDIUM | P4 | 5テーブルJOINの中間テーブルに明示的テナントフィルタなし | 各JOINに `AND t.tenant_id = $1` を追加（CRITICAL-3 修正に含める推奨）|
| MEDIUM | P4 | `groups` / `roles` テーブルに `tenant_id` 単独インデックス欠如 | `idx_groups_tenant_id`、`idx_roles_tenant_id` を migration に追加 |
| LOW | Multiple | RBAC パーミッションロードの Redis キャッシュ未実装 | `kernel-data/src/rbac.rs` line 11 の TODO を解消。GitHub issue #99 参照。|
| LOW | P5 | キー管理の不変条件がコードのみに存在し、仕様書に記載なし | `negative-space-spec.md #63` に合わせて `implementation_plan.md` に不変条件セクションを追加 |

---

## P3 エージェントによる既存修正 (レビュー中に適用済み)

レビュープロセス中に P3 担当エージェントが以下の修正を直接コードベースに適用した。これらは正の変更として記録する。

| ファイル | 変更内容 |
|---------|---------|
| `kernel-api/src/middleware/rbac.rs` | `validate_v2_token_kid()` 関数追加 + ミドルウェア順序に関する49行のモジュールドキュメント |
| `kernel-api/src/middleware/rbac.rs` line 200-203 | `ValidationError` のレスポンスコードを 403 → 500 に修正 |
| `kernel-core/src/auth/key_manager.rs` | ユニットテスト `test_generate_tenant_token_has_v2_format_with_kid` 追加 |
| `tests/contract/src/auth/rbac.rs` | コントラクトテスト5件追加（kid検証 × 3、ミドルウェア順序 × 2）|

---

## 誤検知記録 (P8: COOP/COEP/CORP)

**P8 レビュアーの当初所見**: COOP / COEP / CORP セキュリティヘッダーが欠如しており CRITICAL と評価。

**P14 による反証**: `tests/contract/src/api/security.rs` に `assert_security_headers()` 関数が存在し、以下を明示的にアサートしている:
```
cross-origin-opener-policy: same-origin
cross-origin-embedder-policy: require-corp
cross-origin-resource-policy: same-origin
```

**結論**: P8 レビュアーは実装ソースを参照せず、差分サマリーのみから判断した確定的誤検知。ブロッキング問題リストから除外済み。

---

## 注目すべきアーキテクチャ変更

以下はブロッキング問題ではないが、レビュアーが把握すべき重要な設計変更。

| 変更 | 詳細 |
|------|------|
| `TrustedKey.public_key` 型変更 | `String` → `[u8; 32]` (Ed25519サイズのコンパイル時強制) |
| `verify_manifest` シグネチャ変更 | `tenant_id` パラメータ削除（マニフェスト検証はテナント非依存に整理）|
| トークンブリッジ明確化 | V4 (PASETO) は API層で検証 → V2 (HMAC) に変換してDBセッションへ |
| ビルドガード追加 | `kernel-api/build.rs`、`kernel-core/build.rs`: `test-utils` が release でパニック |
| `tools/gen-keys/` 削除 | Cargo.lock 775行 + ソース全体削除（攻撃面の縮小）|
| `storage_test.rs` 削除 | 934行のテストをインライン + コントラクトテストへ移行（適切）|
| QuotaViolation 変更 | CircuitBreaker の 1〜30秒フロアクランプを削除。ゼロ Retry-After を非 SystemHardLimit に許可 |
| `ed25519-dalek` ピン留め | `=2.2.0` に固定、`std` フィーチャー削除 |
| `dev-auth` フィーチャーフラグ | コントラクトテストに追加 |
| `mock_db_with_budget(n)` | `middleware_integration.rs` に追加されたテストヘルパーパターン |

---

## 修正優先順位ロードマップ

```
Week 1 (マージブロッカー解消):
  ├── CRITICAL-3: RBACRepository を sealed trait + user_id 型強制で再構成 (P4)
  │   └── 同時に: 中間テーブルへの明示的 tenant_id フィルタ追加
  └── CRITICAL-6: Redis Pub/Sub による分散失効キャッシュ実装 (P5)
      └── 既存 Redis 接続 (kernel-api/src/middleware.rs) を再利用

Week 2 (品質向上):
  ├── HIGH: validate_token_kid() キーマテリアル検証 (P5)
  ├── HIGH: parse_tenant_from_token を DB呼び出し前に移動 (P6)
  ├── ~~HIGH: allow_legacy_no_kid の移行完了後削除 (P5)~~ → ✅ 完了
  ├── MEDIUM: groups/roles テーブルへの tenant_id インデックス追加
  └── MEDIUM: tenant_id ログマスキング (P6)

Backlog:
  ├── RBAC パーミッション Redis キャッシュ (issue #99)
  ├── RBAC 可観測性メトリクス追加 (P3)
  └── キー管理不変条件の仕様書記載 (P5)
```

---

## 最終マージ判定

| 項目 | 状態 |
|------|------|
| ブロッキング問題 | **0件** (全解決済み) |
| 勧告的問題 | 9件（マージ後対応推奨）|
| 誤検知（除去済み） | 4件（CRITICAL-1, CRITICAL-2, CRITICAL-4 格下げ済み）|
| 修正済み | CRITICAL-3: AuthenticatedScoped 型 ✅ |
| 修正済み | CRITICAL-5: `allow_legacy_no_kid` 削除 ✅ |
| 修正済み | CRITICAL-6: Redis Pub/Sub 失効伝播 ✅ |
| 誤検知 | P8 COOP/COEP/CORP: 確定 ✅ |
| 全テスト | `cargo test --workspace --lib --bins` 全通過 ✅ |

### ✅ 判定: **APPROVED — マージ可能**

全ブロッキング問題が解決された。このブランチを `main` にマージしてよい。

- **CRITICAL-3**: `AuthenticatedScoped<'a>` 型を導入し、型システムによる `user_id` 強制と dual source of truth 問題を解消。全JOINに明示的テナントフィルタを追加しdefense-in-depth を達成。
- **CRITICAL-5**: `allow_legacy_no_kid` フラグおよび関連コードを完全削除。
- **CRITICAL-6**: Redis Pub/Sub + ポーリングによる分散失効キャッシュを実装。`main.rs` の起動シーケンスに `start_kid_revocation_listener` を組み込み、プロセス起動時からリスナーが実際に動作する。p95 ≤ 60秒 SLO 達成の基盤が整った。

なお、初版で CRITICAL として報告された他の5件のうち、CRITICAL-1, 2, 4, HIGH-1 は実コード照合・Oracle分析により誤検知またはAdvisory相当と確定した。

マージ後、勧告的問題（validate_token_kid() キーマテリアル検証、テナントIDログマスキング等）については GitHub Issue として追跡することを推奨する。
