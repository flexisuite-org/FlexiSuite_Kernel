# FlexiSuite Kernel – Agent/Contributor Guidelines

このリポで作業する開発エージェントの行動指針をまとめます。最新スタック: **Fastify 5.x**, **Node.js 20**, **PostgreSQL 16 (5433)**, **Redis 7 (6380)**, **App port 9000**。

## 基本姿勢
- 破壊的操作はユーザー明示許可なしに行わない（drop/reset/rm -rf など）。
- 変更は最小差分・可読性重視。意図は短いコメントか PR メモで残す。
- シークレットや `.env` の内容は決して出力しない。
- マルチテナント境界（groupId/RLS）を最優先で守る。
- npm の代わりに pnpm を使用する。

## ツールバージョン
- Fastify: 5.x
- @fastify/helmet: 12.x（Fastify 5 対応）
- @fastify/rate-limit: 10.x（Fastify 5 対応）
- @fastify/cors: 10.x
- Prisma/@prisma/client: 5.9.0
- Node: 20.x, pnpm 10.x

## 主要コマンド
```
# 依存起動
docker compose up -d postgres redis

# Prisma
pnpm prisma migrate dev --name init   # 初回のみ
pnpm prisma generate                  # スキーマ変更時

# 開発起動
pnpm dev  # ポート 9000

# テスト/ビルド
pnpm test
pnpm build
```

## やるべきこと (Do)
- すべての DB 操作に `groupId` を必須にする。RLS を有効化する場合はリクエストごとに `set_config('flexi.current_group', ...)` を設定。
- パスワードは Argon2id。JWT は 15m、Refresh は 7d ローテーション+再利用検知。
- 入力は Zod/Ajv でスキーマ検証。ログは PII をマスク。
- イベント処理は冪等性を確保し、idempotency key を持たせる。

## やってはいけないこと (Don't)
- `.env` や鍵をログ・ドキュメントに載せる。
- `prisma.$executeRaw` をテナント条件なしで使う。
- Fastify 5 非対応プラグインを入れる（v4 用を混ぜない）。
- ポートをデフォルト 5432/6379/3000 に固定して記述する（実環境は 5433/6380/9000）。

## 例: テナントスコープの Prisma ミドルウェア
```ts
prisma.$use(async (params, next) => {
  const ctx = (params as any).context || {};
  const groupId = ctx.groupId;
  if (!groupId) throw new Error('missing groupId');

  const scopedModels = ['EntityRecord','EntityDefinition','AppInstall','GroupMember'];
  if (scopedModels.includes(params.model || '')) {
    params.args ??= {};
    params.args.where = { ...(params.args.where || {}), groupId };
    if (params.args.data) params.args.data.groupId = groupId;
  }
  return next(params);
});
```

## 参考ドキュメント
- `docs/security.md`
- `docs/deploy.md`
- `docs/ops.md`
