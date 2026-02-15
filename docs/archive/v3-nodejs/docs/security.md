# Security Patterns & Best Practices (Fastify 5 / Prisma 5.9)

## 1. Authentication
- **Password**: Argon2id (memoryCost>=64MiB, timeCost>=3, parallelism>=4)
- **Access Token**: JWT HS256, 15m, payload `{ userId, groupId, roles, jti }`
- **Refresh Token**: 7d, rotation + reuse detection, stored hashed (deterministic SHA-256) in DB.
- **MFA**: TOTP (speakeasy/otplib) for admin/privileged actions.

### Refresh Token ハッシュの例（deterministic）
```ts
import crypto from 'crypto';
const hashToken = (t: string) => crypto.createHash('sha256').update(t).digest('hex');
```

## 2. Session Management / CSRF
- Refresh 用 Cookie は HttpOnly+Secure+SameSite=Strict。フロントで CSRF トークンを送る（double-submit）。
- リフレッシュはローテーション必須。再利用が検知されたら同一 family を全 revoke。

## 3. Authorization / RBAC
- RolePermission を group スコープで管理。`deny` > `allow`。
- Wildcard は限定的に（例: `entity:*:read`）。
- すべてのハンドラで `groupId` コンテキストを必須にする。

## 4. Multi-Tenancy Isolation
### Prisma ミドルウェア（CRUD 全対応の例）
```ts
const scopedModels = ['EntityRecord','EntityDefinition','AppInstall','GroupMember','Role','Permission','RolePermission','AuditLog'];
prisma.$use(async (params, next) => {
  const groupId = (params as any).context?.groupId;
  if (!groupId) return next(params); // 認証不要エンドポイントはパス

  if (scopedModels.includes(params.model || '')) {
    params.args ??= {};
    const a = params.action;
    if (['findUnique','findFirst','delete','deleteMany','update','updateMany'].includes(a)) {
      params.args.where = { ...(params.args.where || {}), groupId };
    }
    if (['findMany'].includes(a)) {
      params.args.where = { ...(params.args.where || {}), groupId };
    }
    if (['create','createMany','upsert'].includes(a) && params.args.data) {
      if (Array.isArray(params.args.data)) {
        params.args.data = params.args.data.map((d: any) => ({ ...d, groupId }));
      } else {
        params.args.data = { ...params.args.data, groupId };
      }
    }
  }
  return next(params);
});
```

### Postgres RLS（実スキーマに合わせた例）
```sql
ALTER TABLE "EntityRecord" ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation_entity_record ON "EntityRecord"
  USING ("groupId" = current_setting('flexi.current_group', true));
```
- リクエストごとに: `SET LOCAL flexi.current_group = '<groupId>';`

## 5. Secrets Management
- 環境変数: `JWT_SECRET`, `REFRESH_TOKEN_SECRET`, `DATABASE_URL`, `REDIS_URL` など。値はログに出さない。
- シークレットは定期ローテーション。ローテーション時は古いトークン family を失効させる。

## 6. Audit Logging
- 認証/権限変更/削除系は必ず `AuditLog` に記録（actor, groupId, resource, action, success, metadata, correlationId）。
- ログは PII をマスク。トークンやパスワードは記録しない。

## 7. Transport / Headers
- 本番は Nginx で TLS 終端、HSTS 有効化。
- Fastify 5 用 helmet を有効化し、CSP/HSTS/XFO/XCTO を設定。

## 8. Rate Limiting & Abuse Controls
- @fastify/rate-limit v10 （Fastify 5 対応）。IP+ユーザーキーで重要エンドポイントを制限。
- ログイン/リフレッシュは別枠で厳しめに設定。

## 9. Logging & Monitoring
- Pino JSON。エラーは stack + correlationId を付与。
- /metrics を Prometheus スクレイプ。アラート: p95>1s、error rate>1%、DB/Redis ダウン。

## 10. コンポーネント署名/整合性
- manifest ハッシュは **stableStringify**（オブジェクトキーをソート）で決定的に計算し、`hashJson()` で sha256 を取る。署名検証も同じストリングで実施すること。
- lockData.integrity は package.integrityHash と一致させる。差異があると `/components/:id/run` は 422 を返す。
- bundleUpload ではアップロードデータの sha256 を検証し、`SIGNING_SECRET` が存在する場合は `{ manifestIntegrity, bundleIntegrity }` で HMAC 署名を付与する。
- capability 実行は `CAPABILITY_ALLOWLIST` に加え、`CAPABILITY_ROLE_ALLOWLIST`（例: `{"echo":["admin"]}`）でロール要件を課せる。満たさない場合は結果に `forbidden` を返し処理をスキップする。

## 11. RLS 運用メモ
- Postgres 側で RLS を有効化し、ポリシーは `current_setting('flexi.current_group')` を参照する形で定義。
- Fastify hook (`contextPlugin`) でリクエストごとに `set_config('flexi.current_group', groupId, true)` を実行し、Prisma ミドルウェアでも `groupId` 無しの操作を拒否。
- draft モードでは `default_transaction_read_only=on` をセットして書き込みを抑止する（PlaygroundLog など例外は明示）。

## 12. JWT / Refresh ローテーション手順
- アクセストークン: HS256、exp=15m。ペイロードは `{userId, groupId, roles, jti}`。
- リフレッシュ: 7d。発行時に familyId を付与し、`tokenHash` を DB に保存。再発行時は旧トークンを revoke。
- ローテーション時のフロー例:
  1) `/auth/refresh` で旧トークン（家族内で最新）を提示。
  2) 検証後、新しい refresh + access を発行し、旧 refresh を revoke。
  3) 再利用が来た場合は family 全体を revoke し、ユーザーに再ログインを要求。
