# kernel-api ヘルスプローブ運用

## ルーティング方針
- `public_router` には `/health` と `/health/liveness` を配置し、未認証で到達可能です。
- `protected_router` には `/health/readiness` と `diagnostics::routes()` を配置し、`auth_middleware` を通過した認証済みリクエストのみ許可します。
- `readiness` は `TenantContext` を使って DB と Redis の状態を確認するため、テナント解決可能な認証ヘッダーが必須です。

## プローブ設定の要点
- Kubernetes の livenessProbe は未認証の `/health/liveness` を使用してください。
- readinessProbe で `/health/readiness` を使う場合は、サイドカーまたはサービスメッシュで認証ヘッダーを付与してください。

## curl 例
```bash
# 未認証 liveness (200 expected)
curl -i http://127.0.0.1:8080/health/liveness

# 認証付き readiness (200 or 503 expected)
curl -i \
  -H "Authorization: Bearer <PASETO_TOKEN>" \
  -H "x-tenant-id: <tenant-id>" \
  http://127.0.0.1:8080/health/readiness
```

## サイドカー / サービスメッシュ例
```yaml
# 例: Envoy/Istio のヘッダー注入イメージ（概念例）
readinessProbe:
  httpGet:
    path: /health/readiness
    port: 8080
    httpHeaders:
      - name: Authorization
        value: "Bearer ${READINESS_PASETO}"
      - name: x-tenant-id
        value: "${READINESS_TENANT_ID}"
```

## 診断 API との関係
- `diagnostics::routes()` も `protected_router` 配下なので、readiness と同じく `auth_middleware` の認証要件が適用されます。
