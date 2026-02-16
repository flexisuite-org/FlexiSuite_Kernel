# Phase 3 Implementation Review Report

`main`ブランチと比較した `feat/phase3` ブランチの徹底的なレビュー結果を報告します。

## 1. 概要
本フェーズでは、**Entity System** と **Data Access Layer (DAL)** の基盤が実装されました。特に、PostgreSQLの **Row-Level Security (RLS)** を利用したテナント分離の強制メカニズムに焦点が当てられています。

## 2. 評価項目と結果

### 2.1 建築設計の整合性 (Architectural Alignment)
- **Tenant Isolation**: `with_tenant_tx` を通じた `set_config` と `authorize_tenant()` の呼び出しが徹底されており、設計通り「Fail-Closed」な分離が実現されています。
- **Sealed Trait**: `TenantRepository` が `private::Sealed` トレイトを使用しており、外部クレートによる不正な実装を防止しています。
- **Type Safety**: `TenantScoped<T>` ラッパーにより、テナントコンテキストが必須であることが型レベルで保証されています。

### 2.2 セキュリティ分析 (Security Analysis)
| 項目 | 評価 | 詳細・リスク |
|---|---|---|
| **Nonce一意性** | ⚠️ 要注意 | `flexi_nonce` のPKが `(nonce, created_at)` となっています。同一 `nonce` で異なる時刻のリクエストが送られた場合、ユニーク制約をすり抜ける可能性があります。リプレイ攻撃耐性を高めるには `nonce` 単一のPKまたは別の一意制約を検討すべきです。 |
| **署名検証** | ℹ️ 暫定 | 現在は `mock_sig` による固定値検証です。Phase 3の段階としては許容範囲内ですが、実運用に向けて `pgcrypto` 等のリサーチが tech debt として記録されています。 |
| **セキュリティ定義者** | ✅ 優良 | `authorize_tenant` 関数が `SECURITY DEFINER` かつ `search_path` を固定して定義されており、権限昇格攻撃への耐性があります。 |

### 2.3 テスト品質 (Testability)
- `testcontainers` を利用した統合テスト `test_tenant_isolation_rls` が実装されており、実際に別テナントのデータが見えないことが検証されています。
- ただし、テストコード内でマイグレーションSQLを一部手動コピーしている箇所があり、将来的なスキーマ変更時のメンテナンスコストが懸念されます。

## 3. 推奨アクション

1.  **Nonce制約の修正**: `flexi_nonce` テーブルのプライマリキーを `nonce` のみにするか、一連の攻撃シナリオ（同一Nonceの再利用）を確実にブロックする制約構成に変更することを推奨します。
2.  **マージへの影響**: 現在の実装は `implementation_plan.md` の主要な MUST 要件を満たしており、上記のセキュリティ上の指摘事項を Issue または後続タスクとして管理することを条件に、マージ可能な品質に達していると判断します。

---
**Reviewer:** Antigravity  
**Date:** 2026-02-16
