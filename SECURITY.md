# セキュリティポリシー

FlexiSuite Kernel は Pre-Launch のオープンソース Kernel 基盤です。セキュリティ対応は後付けの強化ではなく、MDP品質の一部として扱います。

## サポート対象

現時点のセキュリティ対応対象は `main` ブランチの最新状態です。過去コミット、実験ブランチ、`docs/archive/**` は、メンテナが明示しない限りサポート対象のリリースラインとして扱いません。

Pre-Launch であることは、未公開インターフェースへの互換性窓を意味しません。Launch Boundary 例外が必要な場合は、`docs/implementation_plan.md` で定義された証跡が必要です。

## 脆弱性の報告

exploit手順、実在tenant識別子、悪用可能なtenant影響、未公開の緩和策、secret、運用上の機微情報を含む報告は、公開Issue、公開PR、公開Discussionに書かないでください。

GitHub の Security タブから private vulnerability reporting を使ってください。利用できない場合は、公開Issueには「非公開の脆弱性報告経路が必要です」とだけ書き、技術詳細は非公開経路が確立してから共有してください。

非公開報告には、可能な範囲で次の情報を含めてください。

- 影響するコンポーネントまたは信頼境界
- 影響と前提条件
- 最小再現手順
- 該当する依存関係または advisory ID
- 既知の回避策または緩和策

## トリアージ目標

以下は対応目標であり、公開時期の約束ではありません。

- Critical / High: 1営業日以内にトリアージ
- Medium / Low: 5営業日以内にトリアージ
- Nightly security gate failure: 検証マトリクスで要求される場合、24時間以内にインシデントまたはリスク受容記録を作成

## Coordinated Disclosure

必要に応じて非公開で修正を調整し、リスク緩和後に適切な公開記録を出します。内容に応じて、GitHub Security Advisory、RustSec advisory、release note、changelog、または機微情報を除いた追跡Issueを使います。

## 依存関係・Supply Chain Risk

Dependabot、Dependency Review、RustSec、`cargo-audit`、`cargo-deny`、OpenSSF系のリポジトリ健全性チェックはレビュー信号です。これらはメンテナ判断を置き換えませんが、修正・期限付きリスク受容・追跡Issueのいずれかで処理してください。

RustSec advisory や yanked crate を ignore / allowlist する場合は、`.cargo/audit.toml` または `deny.toml` だけで完結させず、`docs/security-risk-acceptance.md` に期限付きで記録します。`scripts/ci/ci-lint-risk-acceptance.sh` は、ignore が台帳に存在し、期限切れでないことを検証します。

リスク受容には最低限、次を記録します。

- advisory ID または package
- affected version と dependency path
- reachability
- owner と approver
- accepted-until date
- compensating control
- tracking issue

恒久的な ignore、期限なし allowlist、無制限の break-glass bypass は認めません。

## Branch Protection

`main` は required status checks を前提に保護します。CODEOWNERS は補助メタデータではなく、保護設定と組み合わせて高リスク領域のレビューを強制するための境界です。

Code Owner review、stale approval dismissal、last-push approval、admin enforcement は、少なくとも2人以上の write 権限を持つ maintainer または安定した GitHub team が存在する状態で有効化します。単独 write maintainer の bootstrap 期間にこれらを有効化して自己承認不能なデッドロックを作ることは、運用上の可用性リスクとして扱います。

## 公開議論の境界

セキュリティ設計、脅威モデル、抽象化した影響範囲、修正済みの問題を公開で議論することは問題ありません。未修正問題の悪用可能な詳細、実在tenant識別子、秘密鍵、token、bypass手順、未公開の緩和策は公開しないでください。
