# コントリビューションガイド

FlexiSuite Kernel は Pre-Launch の OS カーネル相当基盤です。小さな変更でも、信頼境界・検証・運用記録を軽く扱わないでください。

## 作業前に読むもの

- `AGENTS.md`
- `docs/implementation_plan.md`
- `docs/flexisuite-concept.md`
- `docs/negative-space-spec.md`
- `docs/verification_matrix.md`
- `docs/development-workflow.md`

仕様判断に迷う場合は `docs/implementation_plan.md` を優先します。

## Issue

Issue は、履歴なしのエージェントがその本文だけで着手できる一次情報として書きます。背景、対象範囲、非対象、関連仕様、完了条件、検証計画を省略しないでください。

脆弱性詳細、exploit、実在tenant識別子、悪用可能なtenant影響、秘密情報、未公開の緩和策は公開Issueに書かず、`SECURITY.md` に従って非公開で報告してください。

## Pull Request

- 外部コントリビューターは fork から PR を作成してください。write 権限を持つメンテナは `main` を最新化してから `codex/...` などの短命ブランチを作成してください。
- 関連Issue、変更理由、検証コマンド、レビュー観点をPR本文に記録してください。
- `REQ-*` または MUST/SHOULD を追加・変更する場合は、同じPRで `docs/verification_matrix.md` を更新してください。
- Pre-Launch中の未公開境界向け互換性コードは原則禁止です。例外が必要な場合は、PR本文に `basis` / `boundary` / `deadline` / `removal` / `metric` / `issue` を記録してください。
- scaffold-only check を実検証として扱わないでください。

## ローカル環境

- Rust は `rust-toolchain.toml` の指定に従ってください。MSRV は `README.md` に記録された 1.85 以上です。
- PostgreSQL と Redis が必要な統合テストでは、必要に応じて `docker-compose up -d postgres redis` を使います。
- GitHub 操作を行うメンテナ作業では `gh auth status` で認証状態を確認してください。

## 代表的な検証コマンド

変更範囲に合わせて、該当するものをPR本文に記録してください。

```bash
cargo fmt --all -- --check
cargo test -p kernel-core --no-default-features
bash scripts/ci/ci-lint-traceability.sh
bash scripts/ci/ci-lint-prelaunch-compat.sh
bash scripts/ci/ci-lint-sql-security.sh
bash scripts/ci/ci-lint-slo-profile.sh
bash scripts/ci/ci-lint-test-utils-scope.sh
bash scripts/ci/ci-lint-risk-acceptance.sh
bash scripts/ci/ci-test-contract-suite.sh auth
bash scripts/ci/ci-test-contract-suite.sh quota
bash scripts/ci/ci-test-contract-suite.sh idempotency
bash scripts/ci/ci-test-contract-suite.sh diagnostics
bash scripts/ci/ci-test-contract-suite.sh supplychain
```

Worker、E2E、manifest break-glass、observability などが `scaffold:*` の間は、CIの成功を実検証完了として扱わず、何が未実装かをPR本文に残してください。

## Review

CodeRabbit、Devin、local Gemini、local Codex、Codex Cloud などのレビュー信号は、修正・レビュー済み指摘の別Issue化・憲法議論のいずれかで処理します。AIやBotのレビューは証拠であり、単独の権威ではありません。

Ready for review 中は `pending` を残せます。merge前に、必須レビュー信号の `未依頼` / `pending` を残さないでください。未実施レビューをIssue化してmerge可能にすることはできません。必要なレビュー信号が継続的に利用不能な場合は、merge前に `docs/development-workflow.md` の運用変更として別途扱います。

Code Owner review の branch protection 強制は、複数の write 権限 maintainer または安定した GitHub team が存在する状態で有効化します。bootstrap 期間でも CODEOWNERS 対象領域の変更はPR本文で明示し、レビュー観点を残してください。

## Security and Supply Chain

依存更新、GitHub Actions、署名、trust root、Developer Mode、RLS、`TenantContext`、Worker隔離に触れる変更は security-sensitive として扱います。リスク受容が必要な場合は、期限・責任者・承認者・補償策・追跡Issueを `docs/security-risk-acceptance.md` に記録してください。
