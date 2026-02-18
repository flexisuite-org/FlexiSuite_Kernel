# issue-rbac-update-policy-20260218

## 概要
`kernel-api/src/diagnostics.rs` の `update_policy` ハンドラには、テナント管理者 (tenant-admin) の RBAC 検証が未実装です。現状は一律 `403 Forbidden` を返しており、要件どおりの認可判定を行えていません。

## 背景
- `get_policy` は tenant context でデータアクセスを実施している一方、`update_policy` は管理者権限の判定が未実装。
- 将来の実装者が追跡しやすいよう、コード上の TODO からこのチケットへ辿れる状態にする必要がある。

## 受け入れ条件
1. `update_policy` で tenant-admin 権限を検証し、権限不足時のみ `403` を返す。
2. 権限がある場合はポリシー更新処理を実行する。
3. 認可成功・失敗のテストを追加する。
4. Tenant isolation の前提 (`TenantContext`) を維持し、越境アクセスを発生させない。

## 参照箇所
- `kernel-api/src/diagnostics.rs` の `update_policy`
