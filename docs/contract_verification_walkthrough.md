# Contract Test Implementation Walkthrough

Phase 6: Contract Test Implementation の実装と検証が完了しました。
本フェーズでは、Idempotency, Quota, Supply Chain の各モジュールにおける「仕様上の契約」をコードとして定式化し、厳格に検証可能な状態にしました。

## 実装のハイライト (Final Refinement)

### 1. Supply Chain (仕様への厳格な準拠)
- **Digest 形式の修正**: 仕様書 (`implementation_plan.md`) の例示に基づき、`sha384-` (dash) 形式のプレフィックスを主要なデリミタとしてサポートしました。
- **検証の強制**: `verify_manifest` において `expected_artifact_digest` を必須引数 (`&str`) とし、呼び出し側が検証をスキップできないよう契約を強化しました。
- **エラー分類**: 鍵の不一致を `KeyMismatch` として独立させ、トリアージを容易にしました。

### 2. Quota (レイヤー別クリップ規則)
- **System Hard Limit**: 仕様 (`REQ-QUOTA-HTTP-CONTRACT`) に従い、システム保護窓の解除秒数を **1-30秒でクリップ** するロジックを実装しました。これにより、過剰な `Retry-After` によるクライアント側のタイムアウトや、逆に短すぎる再試行による負荷集中を防止します。
- **Pro/Enterprise ガード**: その他のレイヤー（API Rate Limit 等）については、引き続き最大1年のキャップを適用し、安全性を確保しています。

### 3. Idempotency (冪等性)
- **衝突ガードの実装**: `check_idempotency_conflict` 関数により、「同一ターゲット（正規化済み）かつ異なるボディハッシュ」の場合に確実に 409 Conflict を返す契約を確認しました。

## 検証結果

`cargo test -p contract-tests` にて以下のテストが Pass しています：

```text
test idempotency::canonical_request_target::tests::test_idempotency_query_order_conflict_guard ... ok
test idempotency::canonical_request_target::tests::test_idempotency_canonical_request_target ... ok
test quota::http_matrix::tests::test_quota_retry_after_boundaries ... ok
test quota::http_matrix::tests::test_quota_http_matrix ... ok
test supplychain::manifest_checks::tests::test_manifest_break_glass_scope_and_ttl ... ok
test supplychain::manifest_checks::tests::test_manifest_signature_trust_root ... ok
```

## 今後の展望 (Next Steps)

- **Phase 7: Kernel API 実装**: 
  - 本フェーズで構築した `kernel-core` の検証ロジックを、Axum ミドルウェア層に統合します。
  - テストについても、実際の HTTP 要求をシミュレートする統合テストへと移行します。
