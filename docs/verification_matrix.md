# FlexiSuite 検証マトリクス (CI/CD・Runtime・Drill)

本ドキュメントは、`docs/implementation_plan.md` の要件のうち、特に高リスク `REQ-*` を中心に、`PR-Blocking` / `Nightly` / `Operational Drill` の3層で強制する方法を定義する。`REQ-*` 未付与の `MUST/SHOULD` については、第1-4章の領域別テーブルで検証責務を明示し、仕様変更時に追随更新する。

## 0. トレーサビリティ原則

- 高リスク `REQ-*` は、少なくとも1つの自動検証（`PR-Blocking` または `Nightly`）に紐付ける。
- 実地演習が本質の要件（DR、鍵緊急失効）は、`Operational Drill` に加え、`PR-Blocking` の Readiness 検証を必須にする。
- `Nightly` は非PRブロッキングだが運用上必須とし、失敗時は24時間以内にインシデントを起票する。
- `REQ-*` が `implementation_plan` に追加・変更されたPRは、本ファイルの追随更新がない場合にFailさせる。
- `REQ-*` 未付与の `MUST/SHOULD` が追加・変更された場合も、本ファイルの該当領域（第1-4章）の更新を必須とする。

| REQ-ID | 検証ゲート | 主検証ジョブ/手段 | 失敗条件 |
|---|---|---|---|
| `REQ-AUTH-SEC` | PR-Blocking | `ci:lint-sql-security`, `ci:test-contract`（auth suite）, `ci:e2e-frontend-security` | `SECURITY DEFINER` 標準契約違反、`X-Tenant-Id` の本番許容、`401/403` 境界の逸脱 |
| `REQ-AUTH-SOURCE` | PR-Blocking | `ci:test-auth-contract` | Token/Header抽出不備、Debugヘッダの本番受理 |
| `REQ-CONTRACT-VERIFY` | PR-Blocking | `ci:lint-traceability`, `ci:test-contract`, `ci:test-observability` | 契約ドキュメントと実装/検証の不整合、契約テスト欠落、メトリクス契約欠落 |
| `REQ-TENANT-TOKEN-V2` | PR-Blocking + Nightly | `ci:test-auth-contract`, `nightly:test-token-compat`, `nightly:test-token-version-usage` | v2発行不備、`kid` 欠落受理、互換期限超過受理、14日連続ゼロ未達でv1停止 |
| `REQ-KEY-REVOCATION-SLO` | PR-Blocking + Nightly + Drill | `ci:lint-drill-readiness`, `nightly:test-key-revocation-chaos`, 月次失効演習 | Readiness欠落、失効伝播 `p95 > 60s`、失効鍵で検証成功 |
| `REQ-QUOTA-HTTP-CONTRACT` | PR-Blocking | `ci:test-contract`（quota suite） | 判定表と異なるHTTPコード、`Retry-After` 欠落/異常 |
| `REQ-IDEMPOTENCY-HEADER` | PR-Blocking | `ci:test-contract`（idempotency suite） | `Idempotency-Key` 契約逸脱、衝突時 `409` 不履行 |
| `REQ-IDEMPOTENCY-CONFLICT` | PR-Blocking | `ci:test-contract`（idempotency suite） | 衝突検知（同一キー・異Body）の失敗 |
| `REQ-PROTOCOL-FALLBACK-UX` | PR-Blocking | `ci:e2e-frontend-security`, `ci:e2e-worker-protocol-fallback-a11y` | `protocol.error` 時に標準フォールバック/A11y要件未達 |
| `REQ-DIAG-CONSENT` | PR-Blocking + Nightly | `ci:test-contract`（diagnostics consent suite）, `nightly:test-diagnostics-revocation-lag` | 同意なし送信、撤回後5分超で送信継続 |
| `REQ-MANIFEST-TRUST-ROOT` | PR-Blocking + Nightly | `ci:lint-manifest-trust-root`, `ci:test-manifest-signature-contract`, `ci:test-manifest-break-glass`, `nightly:test-manifest-revocation-propagation`, `nightly:test-manifest-retired-window` | 署名検証順序違反、`revoked kid` 受理、`retired` 窓外受理、信頼ルート署名不一致、break-glassの時限/スコープ違反 |
| `REQ-SUPPLYCHAIN-DIGEST-FORMAT` | PR-Blocking | `ci:test-contract` (supplychain) | digest prefix (`sha256-`/`sha384-`) 違反 |
| `REQ-SUPPLYCHAIN-DIGEST-MATCH` | PR-Blocking | `ci:test-contract` (supplychain) | Manifest digest と artifact digest の不一致 |
| `REQ-SIDELOADING-WARNING` | PR-Blocking | `ci:e2e-sideloading-warning` | Developer Mode時の警告・同意フロー欠落 |
| `REQ-SLO-ENV-PROFILE` | PR-Blocking + Nightly | `ci:lint-slo-profile`, `nightly:test-slo-smoke` | `ops/slo_profile.yaml` 未一致でSLO判定 |
| `REQ-DR-REHEARSAL` | PR-Blocking + Drill | `ci:lint-drill-readiness`, 月次ステージング/四半期本番相当演習 | Readiness欠落、RPO/RTO未達、演習記録欠落 |
| `REQ-EVENT-GAP-001` | PR-Blocking | `ci:test-contract`（gap recovery suite） | Gap検出不備、欠番検知漏れ |
| `REQ-EVENT-GAP-002` | PR-Blocking | `ci:test-contract`（gap recovery suite） | FSM遷移不備、回復不能時の書き込み停止不履行 |

## 1. 静的検証 (PR-Blocking: Build/Static Analysis)

| 領域 | 要件 | 検証方法 | 失敗条件 |
|---|---|---|---|
| テナント隔離 | `TenantContext` なしDB操作を禁止 | `trybuild` によるコンパイル失敗テスト | `TenantScoped<T>` を経由しないDB操作がコンパイル成功 |
| テナント隔離 | `DatabaseConnection` の外部公開禁止 | 可視性チェック + sealed trait 制約 | 生接続型が `pub` で公開される |
| 認証トークン | `tenant_token` v2 (`kid` 必須) | auth schema lint + unit test | `kid` 欠落トークンを受理 |
| 認証トークン | v1移行窓の機械判定 | migration policy lint | 2リリース/60日・14日連続ゼロ条件の未実装 |
| 特権SQL | `SECURITY DEFINER` 標準テンプレート必須 | SQL Linter (AST優先) | `SET search_path = flexi, pg_catalog, pg_temp` 不在、`REVOKE ... FROM PUBLIC` 不在、`pg_catalog` 修飾欠落 |
| 特権SQL | `flexi.kernel_mode` 利用禁止 | SQL AST lint (migration対象限定) | マイグレーションSQL内に検出 |
| マニフェスト | 配布時は `DistManifest` のみ受理 | `manifest-validator` | `^`/`~` などRange残存、`digest` 欠落、未解決バージョン |
| 信頼ルート | `manifest_trust_root` 構造・署名 | trust-root lint + signature lint | `kid/status/not_after` 欠落、ルート署名不一致 |
| 署名バイパス | break-glass制約（時限・スコープ・監査） | config lint + policy lint | 既定有効、TTL未設定、`(tenant_id, manifest_digest)` 非限定、必須監査項目欠落 |
| CDN整合性 | lockfileのハッシュ固定と署名 | `component.lock` validator + signature lint | hash/sig不一致を許容 |
| イベント契約 | `order_mode` の明示必須 | Rust型検証 + JSON Schema検証 | `order_mode` 欠落または不正値 |
| 冪等性契約 | `Idempotency-Key` と `X-Action-Id` 契約 | OpenAPI lint + handler contract test | ヘッダ仕様違反、`X-Action-Id` 欠落 |
| プロトコル | `protocol.error` エンベロープ項目 | Schema lint | `type/code/request_id` 欠落 |
| フォールバックUI | A11y + i18n要件 | フロント静的lint + i18n key lint | キーボード操作不可、`aria-live` 欠落、locale fallback欠落 |
| 診断PII | payload上限とサニタイズ要件 | スキーマ検証 + 静的ルール | `truncated` 欠落、機密属性許可、画像URLのpath生値 |
| 診断同意 | 既定 `opt-out` | policy default test | 初期状態が `enabled=true` |
| SLO | 測定環境固定 | `ops/slo_profile.yaml` lint | 必須項目欠落、無効値、トラフィックミックス欠落 |
| SLO再現性 | ベンチ入力の固定化（seed/データ分布） | benchmark profile lint | `dataset_seed` / `dataset_version` / `distribution_profile` 欠落、実行時プロファイル不一致 |
| Drill Readiness | DR実演習の準備情報固定 | runbook metadata lint | `runbook_updated_at` / `owner` / `next_drill_at` / `last_drill_report` 欠落 |

## 2. 契約テスト (PR-Blocking: Integration/E2E)

| 領域 | テストケース | 検証観点 | 期待結果 |
|---|---|---|---|
| 認証/RLS | `test_auth_replay_attack` | 同一Nonce再利用拒否 | 2回目は認証失敗 |
| 認証/RLS | `test_rls_fail_closed` | 未認証時のFail-Closed | 読み取り結果0行 |
| 認証トークン | `test_tenant_token_v2_kid_required` | `kid` なし拒否 | 認証失敗 |
| 認証トークン | `test_tenant_token_v1_transition_window` | v1互換期間挙動 | 期間内受理・期間後拒否 |
| 冪等性 | `test_idempotency_conflict` | 同一キー・異なる本文 | `409 Conflict` |
| 冪等性 | `test_action_id_reuse` | 同一キー・同一本文の再送 | 既存 `action_id` を返却 |
| 冪等性 | `test_action_status_lookup` | `GET /actions/{action_id}` | `PENDING/COMPLETED/FAILED` が整合 |
| 冪等性 | `test_idempotency_canonical_request_target` | canonical化仕様（path/query） | 並び順差/重複キー/末尾スラッシュ差異を正規化して同一判定 |
| 冪等性 | `test_idempotency_query_order_conflict_guard` | canonical化後の本文不一致検知 | query順序差のみでは衝突とせず、本文差異時のみ `409` |
| クォータ | `test_quota_http_matrix` | 429/503判定表 および `X-Violation-Type` ヘッダ | レイヤ別に規定コードおよび `X-Violation-Type` を返却 |
| クォータ | `test_retry_after_contract` | `Retry-After` 算出 および上限クリップ制約 | 欠落なく非負秒で返却（CBやSystem Hard Limitのクリップ制約を含む） |
| クォータ | `test_quota_circuit_breaker_branch_contract` | Circuit Breaker動作の検証 | 評価順序の遵守と `X-Violation-Type: circuit_breaker` の返却 |
| イベント順序 | `test_event_ordering_entity` | `order_mode=entity` | `entity_seq` 順に処理 |
| イベント順序 | `test_event_ordering_causality` | `order_mode=causality` | `causality_seq` 順に処理 |
| イベント順序 | `test_event_mode_mix_forbidden` | 同一 `entity_id` のモード混在禁止 | 作成時に拒否 |
| Gap Recovery | `test_gap_recovery_found` | outbox補償読み取り成功 | 再送後に処理再開 |
| Gap Recovery | `test_gap_recovery_rebuild_required` | 欠番回復不能時 | `rebuild_required=true` 設定 + 書き込み停止 |
| Gap Recovery | `test_gap_recovery_rebuild_sla` | 停止解除SLA | 60秒以内起動、30分超過でエスカレーション |
| Worker Protocol | `test_worker_protocol_mismatch` | 互換性不一致時の失敗動作 | `protocol.error` 送信後 `terminate` |
| Worker UX | `test_protocol_fallback_screen` | 互換性不一致時の表示 | 標準フォールバック画面を表示 |
| Worker UX | `test_protocol_fallback_a11y_i18n` | A11y + locale fallback | `aria-live` 通知、キーボード操作、locale順序を満たす |
| Worker UX | `test_canvas_fallback_accessibility_floor` | OffscreenCanvas非対応時のUX下限 | 再読込/戻る/サポート導線にキーボード到達可能、状態説明が読み上げ可能 |
| Worker UX | `test_canvas_fallback_metric_emission` | 互換性劣化メトリクス | `worker_canvas_fallback_total` が増分される |
| 診断PII | `test_diagnostics_scrub` | URL/トークン/属性サニタイズ | 機密値が伏字化される |
| 診断PII | `test_diagnostics_image_url_minimization` | 画像URL最小化 | `origin` のみ保持、pathはハッシュ化 |
| 診断PII | `test_diagnostics_payload_limit` | 512KB超過時挙動 | 安全に切り詰め + `truncated=true` |
| 診断同意 | `test_diagnostics_opt_out_default` | 初期ポリシー | 送信不可 |
| 診断同意 | `test_diagnostics_policy_revocation` | 同意撤回反映 | 5分以内に送信停止 |
| 診断API | `test_diagnostics_report_query_contract` | `report/query` 責務分離 | `report` は登録、`query` は取得のみ許可 |
| サプライチェーン | `test_manifest_signature_trust_root` | 署名検証順序 | digest一致後に署名検証、`revoked kid` 拒否 |
| サプライチェーン | `test_manifest_retired_acceptance_window` | `retired` 受理窓の厳格化 | `retired` はgrace window内のみ受理し、窓外は拒否 |
| サプライチェーン | `test_manifest_break_glass_scope_and_ttl` | break-glass制約 | 60分超過で自動無効化、`(tenant_id, manifest_digest)` 外利用を拒否、監査ログ必須項目を記録 |
| サプライチェーン | `test_sideloading_warning_contract` | Developer Mode挙動 | 未署名受理時の警告表示とIsolation強制の維持 |
| フロント配信 | `test_coop_coep_headers` | COOP/COEP契約 | 必須ヘッダを返却 |
| フロント配信 | `test_cdn_proxy_corp_fallback` | CORP不足時プロキシ経由 | 直接配信を拒否しプロキシへ迂回 |

## 3. Nightly検証 (非ブロッキングだが必須運用)

| 領域 | テストケース | 検証観点 | 期待結果 |
|---|---|---|---|
| 鍵運用 | `nightly:test-key-revocation-chaos` | 失効伝播遅延 | `p95 <= 60秒` |
| 認証トークン | `nightly:test-token-compat` | v1/v2互換窓検証 | 2リリース/60日規約に一致 |
| 認証トークン | `nightly:test-token-version-usage` | v1/v2利用率可視化 | 14日連続ゼロ判定に必要な時系列を保持 |
| SLO | `nightly:test-slo-smoke` | Warm/Cold分離計測 | 逸脱時アラート発報 |
| SLO | `nightly:test-slo-reproducibility` | 再現性（seed/分布/反復） | 同一 `dataset_seed` / `dataset_version` で結果の許容分散内再現、プロファイル不一致時Fail |
| イベント耐障害 | `nightly:test-claim-pending-failover` | `claim_pending` 再配分 | 順序維持で回復 |
| イベント耐障害 | `nightly:test-hot-shard-detection` | ホットシャード検知 | 閾値超過で制御動作 + アラート |
| 診断同意 | `nightly:test-diagnostics-revocation-lag` | 撤回反映遅延 | 5分以内で収束 |
| サプライチェーン | `nightly:test-lockfile-integrity` | lockfile整合性再検証 | 不一致時Fail |
| サプライチェーン | `nightly:test-manifest-revocation-propagation` | trust root失効伝播 | `revoked kid` 拒否が `p95 <= 60秒` で反映 |
| サプライチェーン | `nightly:test-manifest-retired-window` | `retired` 鍵受理窓 | grace window終了後の受理が0件であること |
| Runtime制限 | `nightly:test-sandbox-cpu-vs-wallclock` | CPU/Time分離検証 | CPU 5s超過でKill, Sleep 20sはPass |

## 4. 運用強制と監視 (Runtime Enforcement)

| 領域 | 要件 | 強制メカニズム | 監視/アラート |
|---|---|---|---|
| クォータ制御 | 優先順位 `System > CircuitBreaker > Tenant > API` | ミドルウェア短絡判定 | `quota_reject_total{layer=...}` |
| クォータ制御 | 429/503 + `Retry-After` | APIレスポンスガード | `quota_retry_after_missing_total` |
| NTPドリフト | App-DB 時刻差監視 | 定期ジョブ | 1秒超過継続でCritical |
| Nonce運用 | TTL回収遅延防止 | `pg_cron` または外部ジョブ | `nonce_cleanup_lag_seconds` |
| イベント回復 | `rebuild_required` 中の書き込み停止 | 書き込みガード | `event_rebuild_block_seconds` |
| イベント回復 | 停止解除SLA | リビルド監視 + 自動エスカレーション | `event_rebuild_sla_breach_total` |
| イベント偏り | ホットシャード抑制 | 取り込み制御 + 優先度キュー | `event_hot_shard_detected_total` |
| 冪等性保持 | `action_id`/結果 24h保持 | TTL付きストア | 保持切れ前削除の検知 |
| 診断データ | 24h以内削除 | TTLジョブ | `diagnostics_retention_violation_total` |
| 診断同意 | 同意ポリシー遵守 | Policy cache + deny guard | `diagnostics_consent_violation_total` |
| Worker互換性 | `protocol.error` 監視 | error telemetry | `worker_protocol_mismatch_total` |
| Worker互換性 | Canvasフォールバック監視 | fallback telemetry | `worker_canvas_fallback_total` |
| SLO | Warm/Cold 分離計測 | メトリクスラベル分離 | `sandbox_duration_seconds{kind=warm|cold}` |
| trust root | 失効反映監視 | keyset配布監視 | `manifest_trust_root_propagation_seconds` |
| trust root | break-glass運用監視 | policy telemetry + audit | `manifest_signature_bypass_active_total`, `manifest_signature_bypass_expired_total` |
| 信頼スコア | 係数固定 + 監査可能性 | バッチ計算 + 監査ログ | 再計算差分アラート |

## 5. CI ジョブ構成 (推奨)

| ジョブ名 | 実行内容 | ブロッキング |
|---|---|---|
| `ci:lint-traceability` | `REQ-*` 追加/変更時の検証マトリクス追随確認 | 必須 |
| `ci:lint-sql-security` | `SECURITY DEFINER` テンプレート検証、`flexi.kernel_mode` 検出（AST） | 必須 |
| `ci:lint-manifest` | `DevManifest`/`DistManifest` ルール検証 | 必須 |
| `ci:lint-manifest-trust-root` | `manifest_trust_root` 構造・署名・有効期限検証 | 必須 |
| `ci:test-manifest-break-glass` | break-glassの時限無効化、スコープ限定、監査ログ必須項目の契約テスト | 必須 |
| `ci:lint-slo-profile` | `ops/slo_profile.yaml` の妥当性検証 | 必須 |
| `ci:lint-drill-readiness` | DR/失効演習の Readiness メタデータ検証 | 必須 |
| `ci:test-compile-guards` | `trybuild` による型安全ガード検証 | 必須 |
| `ci:test-contract` | 認証・冪等性・クォータ・イベント順序・診断・署名の契約テスト | 必須 |
| `ci:e2e-frontend-security` | COOP/COEP、CDNプロキシ、protocol fallback UI のE2E | 必須 |
| `ci:e2e-worker-protocol-fallback-a11y` | fallback のA11y/i18n E2E検証 | 必須 |
| `ci:e2e-canvas-fallback-metrics` | OffscreenCanvas非対応時のUX下限 + `worker_canvas_fallback_total` 計測検証 | 必須 |
| `ci:test-observability` | メトリクス名/ラベル/アラートルール整合チェック | 必須 |
| `nightly:test-reliability` | 鍵失効・token互換・claim_pending・SLO・整合性の長時間検証 | 必須（非PRブロック） |

## 6. Operational Drill (CI外で必須)

| 領域 | 頻度 | 実施環境 | 合格条件 |
|---|---|---|---|
| DR演習 (RPO/RTO) | 月次 | ステージング（本番相当データ量） | RPO <= 5分、RTO <= 1時間 |
| DR演習 (リージョンフェイルオーバー) | 四半期 | 本番相当環境 | Playbook手順逸脱なし、主要機能復旧 |
| 鍵緊急失効演習 | 月次 | ステージング | 失効伝播 `p95 <= 60秒` |
| 監査復元演習 | 四半期 | ステージング | 監査証跡の欠落なしで復元可能 |

## 7. フェーズ別導入チェック

### Phase 2 (Identity & Access)
- [ ] `SECURITY DEFINER` SQL Linter を導入し、未準拠SQLでCIをFailさせる。
- [ ] `authorize_tenant()` のNonce再利用拒否テストを追加する。
- [ ] `tenant_token` v2 (`kid` 必須) を実装し、2リリース/60日・14日連続ゼロ条件を検証する。
- [ ] `Idempotency-Key` / `X-Action-Id` の契約テストを追加する。

### Phase 3 (Entity System)
- [ ] 監査ログ保全経路（長期保存）を検証し、リハーサル結果を記録する。
- [ ] 基本バックアップと復元手順を自動テスト可能な形にする。
- [ ] 診断同意ポリシーの永続化と監査証跡を実装する。

### Phase 4 (Event System)
- [ ] `order_mode` の混在禁止をサーバー側で強制し、E2Eで検証する。
- [ ] `rebuild_required` フラグ時の書き込み停止と解除SLAを契約テスト化する。
- [ ] `ReliableConsumer::claim_pending` のフェイルオーバー動作をテストする。
- [ ] ホットシャード検知と抑制ロジックをNightlyで検証する。

### Phase 5 (Component System)
- [ ] `component.lock` のハッシュ検証と `manifest.json.sig` 検証をCIに組み込む。
- [ ] `manifest_trust_root.json(.sig)` の配布・検証をCIに組み込む。
- [ ] `ci:test-manifest-break-glass` を導入し、時限無効化・スコープ限定・監査ログ必須項目を検証する。
- [ ] `DistManifest` 固定化（Range排除）をビルドパイプラインで強制する。
- [ ] SBOM脆弱性検査（Critical/High fail）を導入する。

### Phase 6 (Runtime)
- [ ] Sandboxのメモリ/CPU制限超過時の強制停止を契約テスト化する。
- [ ] **CPU時間 (5s) と Wall Clock (30s) の分離を検証するNightlyジョブ (`nightly:test-sandbox-cpu-vs-wallclock`) を実装する。**
- [ ] ネットワーク許可リスト未宣言時の `deny-by-default` を検証する。
- [ ] Runtime計測を `ops/slo_profile.yaml` と接続する。

### Phase 7-8 (Frontend / Reliability)
- [ ] Worker `protocol.error` のハンドリングとフォールバックUIをE2Eで検証する。
- [ ] fallback UI のA11y（キーボード操作/`aria-live`）とlocale fallbackをE2Eで検証する。
- [ ] `ci:e2e-canvas-fallback-metrics` を導入し、OffscreenCanvas非対応時のUX下限と `worker_canvas_fallback_total` 計測を検証する。
- [ ] COOP/COEP + CDNプロキシCORPフォールバックをE2Eで検証する。
- [ ] Warm/Cold SLOメトリクスを分離し、ダッシュボードに反映する。
- [ ] `nightly:test-slo-reproducibility` を導入し、`dataset_seed` / `dataset_version` 固定時の再現性を検証する。
- [ ] 診断データPIIスクラブ、同意制御、24h削除を監視対象に追加する。
- [ ] DR月次/四半期演習テンプレートと `ci:lint-drill-readiness` を運用に組み込む。

### Phase 9 (Ecosystem)
- [ ] Trust Score算出係数の固定化を監査可能ログとともに実装する。
- [ ] Anti-Sybil検知（短期大量導入/IP偏り）をスコア計算前に適用する。
- [ ] Store Verified の条件付き手動審査フローを運用Runbook化する。
