FlexiSuite Kernel – 引き継ぎメモ（2025-11-29 JST）

リポ状態
- ブランチ: main（テスト安定化の途中変更がローカルにあり未プッシュ）
- DB/Redis: `docker compose up -d postgres redis`（5433/6380）、`pnpm prisma migrate deploy` 済み
- CI: `.github/workflows/ci.yml` で Postgres/Redis → migrate deploy → `pnpm test`

実装済みの主な追加
- コンポーネント署名・バンドル署名（SIGNING_SECRET対応）、bundleUpload API、bundle storage 抽象（ローカル保存）
- capability allowlist / allowedCapabilities、capability handlers 集約
- draftモード read-only（Prismaミドルウェア＋default_transaction_read_only）と sandbox optional 化
- PlaygroundLog へのドラフト結果保存（RLS適用）
- pgcrypto 有効化マイグレーション

未解決・作業中の課題（テスト）
- test/install.run.integration.spec.ts: `/components/:id/run` が 422 → install 作成時の lockData が package.integrityHash と一致していない箇所あり
- test/capability.allowlist.spec.ts: 同じく 422（lockData.integrity 不一致）
- test/bundle.upload.spec.ts: 401。bundleUpload 実行時に ownerGroupId と JWT groupId の整合を確認する必要あり

修正のすすめ方
1) すべての componentInstall.create に `lockData: { integrity: pkg.integrityHash }` を必ず設定
2) bundleUpload テスト: beforeEach で truncate→seed→package作成（DRAFT）→同じ user/group の JWT で `/registry/packages/:id/bundleUpload`
3) 再実行: `pnpm test --runInBand`（Postgres/Redis 起動後）

コマンドメモ
- docker compose up -d postgres redis
- pnpm prisma migrate deploy
- pnpm test --runInBand

参考ファイル
- シード/トランケート: `test/helpers/seed.ts`
- Jestセットアップ: `test/jest.setup.ts`
- サンドボックス（optional require）: `src/kernel/runtime/sandbox.ts`
- ストレージ抽象: `src/kernel/components/storage.ts`

次の一手（優先）
- 上記テスト修正を完了して CI を緑化
- bundle ストレージを本番用（S3 等）に差し替える場合は `storage.ts` を差し替え
- capability 実行権限を role/allowlist ベースで強化し、検証を追加

進捗メモ（2025-11-29）
- `/components/:id/run` 422 の主因だった manifest ハッシュの非決定性を解消（stableStringify + hashJson を導入し、検証側も同一化）。
- authHook/contextPlugin をグローバル登録に変更し、JWT の groupId が ALS に届かずクロスグループで 200 になる問題を修正。
- capability 実行時の allowlist ロジックを調整（許可ゼロの場合でも unsupported_capability を返す）。
- bundleUpload の署名が undefined になる問題に対し、test 環境の SIGNING_SECRET デフォルトを config に追加。
- 対象テスト `test/install.run.integration.spec.ts`, `test/capability.allowlist.spec.ts`, `test/bundle.upload.spec.ts` は全て通過。

追加メモ（詳細）
- `/components/:id/run` の 422 は `src/api/routes/components.ts` で `lockData.integrity !== package.integrityHash` を検出した場合に発生。`ComponentInstall.create` を直接呼ぶ箇所（例: `test/install.run.integration.spec.ts`, `test/helpers/seed.ts`）では pkg.integrityHash を使って lockData を必ず埋める。
- 予防策として Prisma Middleware で ComponentInstall.create 時に lockData.integrity が未設定なら対応する package.integrityHash を自動補完する案もあり（テナント条件は維持）。
- `capability.allowlist.spec.ts` の 422 も同じ原因が濃厚。lockData を揃えてから再実行する。
- `bundle.upload.spec.ts` の 401 は JWT 検証失敗または ownerGroupId 不一致が主因になりやすい。確認ポイント: (1) token() が参照する `config.JWT_SECRET` と環境変数が一致しているか（config は import 時に検証）、(2) package.ownerGroupId と JWT の groupId を揃えること（更新クエリで ownerGroupId を where 条件に使用）。
- 部分再現コマンド: `pnpm test --runInBand -- test/install.run.integration.spec.ts test/capability.allowlist.spec.ts test/bundle.upload.spec.ts`（Postgres/Redis 起動 & migrate 済み前提）。

進捗メモ（2025-11-30）
- テストスイート全体を `pnpm test`（`--runInBand --forceExit`）で安定通過。
- ストレージに S3 ドライバを追加（オプション依存、デフォルトはローカル）。環境変数を `.env.example` に追記。
- capability にロール要件を課す `CAPABILITY_ROLE_ALLOWLIST`（JSON）を実装し、テスト追加。
- RLS/JWT 運用メモを `docs/security.md` に追記。
 - `pnpm test` は forceExit なしでもグリーン（Redis 遅延初期化+close で open handle 解消）。S3 クライアント依存を追加済み（@aws-sdk/client-s3）。
