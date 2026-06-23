<!--
FlexiSuite Kernel への貢献ありがとうございます。

この本文は、レビュー担当者と、履歴なしで後から読むAIエージェントのための台帳です。
未修正の脆弱性詳細、secret、実在tenant識別子、exploit手順、非公開の運用情報は書かないでください。
脆弱性報告は SECURITY.md に従ってください。
-->

## 概要

<!-- 何を、なぜ変更したか。Kernelとして正しい理由を書いてください。 -->

## 関連Issue / 作業

<!-- merge時にIssueを閉じる場合のみ "Fixes #123" を使ってください。なければ N/A。 -->

- Issue:
- Follow-up:

## 変更種別

<!-- 該当するものをすべて選んでください。 -->

- [ ] Bug fix
- [ ] Feature / capability
- [ ] Refactor / cleanup
- [ ] Documentation
- [ ] CI / repository governance
- [ ] Dependency / supply chain
- [ ] Security-sensitive change
- [ ] Contract / specification change

## 契約・憲法

<!--
implementation_plan.md がSSOTです。REQ-*、MUST、SHOULD の意味を追加・削除・変更する場合は、
同じPRで docs/verification_matrix.md を更新してください。
-->

- REQ-ID / Phase: <!-- REQ-... / Phase ... / N/A（理由） -->
- 影響する MUST/SHOULD:
- Verification Matrix:
  - [ ] このPRで更新した
  - [ ] 更新不要。理由:
- Negative Space:
  - [ ] 禁止された互換性・権威化・観測不能化・レビュー省略を追加していない

## Launch Boundary

<!--
Pre-Launch中の互換性コードは原則禁止です。legacy fallback、deprecated API、旧形式受理、
移行窓、未公開境界向けの互換パスを追加する場合は、下記をすべて埋めてください。
-->

- [ ] Launch Boundary例外なし
- [ ] Launch Boundary例外あり。下記に記録した

```text
basis:
boundary:
deadline:
removal:
metric:
issue:
```

## セキュリティ・運用影響

<!-- 実際に触った領域を確認したうえで N/A と書いてください。 -->

- Tenant isolation / RLS / `TenantContext`:
- Auth / token / key revocation:
- Worker isolation / sandbox / quota:
- Manifest signing / trust root / Developer Mode:
- Diagnostics / PII / consent:
- Dependencies / RustSec / Dependabot:
- GitHub Actions permissions / secrets:
- Risk acceptance: <!-- N/A、または advisory/package/owner/approver/accepted_until/issue/compensating control -->

## 検証

<!--
実行したコマンドだけを書いてください。scaffold-only check を実検証として扱わないでください。
Nightly / Operational Drill は、通常このPR内で長時間演習を実行するのではなく、Readiness証跡を確認します。
-->

```text

```

- 影響する PR-Blocking gate:
- Nightly 影響:
- Operational Drill 影響:
- scaffold-only check:

## Maintainer Merge Checklist

<!--
Draft中は進捗欄として `未依頼` / `pending` を使って構いません。
Ready for review 中は `pending` を残せます。merge前に、各レビュー信号を「clear」「addressed」「レビュー済み指摘を別Issue化」のいずれかで処理してください。
Bot/AIレビューは証拠であり、単独の権威ではありません。
merge前に `未依頼` / `pending` は残せません。未実施レビューを `issue` 化してmerge可能にすることはできません。
必要なレビュー信号が継続的に利用不能な場合は、本PRをmergeする前に `docs/development-workflow.md` の運用変更として別途扱ってください。
-->

| Signal | State | Resolution |
|---|---|---|
| CodeRabbit comments/nitpicks | draft/review: 未依頼 / pending / clear / addressed / reviewed-issue | |
| Devin flags | draft/review: 未依頼 / pending / clear / addressed / reviewed-issue | |
| Local Gemini | draft/review: 未依頼 / pending / clear / addressed / reviewed-issue | |
| Local Codex | draft/review: 未依頼 / pending / clear / addressed / reviewed-issue | |
| Codex Cloud | draft/review: 未依頼 / pending / clear / addressed / reviewed-issue | |

Ready-to-merge gate:

- [ ] 必須レビュー信号に `未依頼` / `pending` が残っていない
- [ ] `reviewed-issue` はレビュー実施後の指摘分離であり、未実施レビューの代替ではない
- [ ] CODEOWNERS対象変更は Code Owner review を受けている、または bootstrap 例外として対象領域・レビュー観点をPR本文に記録している
- [ ] 期限付きリスク受容がある場合は `docs/security-risk-acceptance.md` と追跡Issueに記録している

## レビュアーへのメモ

<!-- 最初に見てほしい箇所、トレードオフ、非対象を短く書いてください。 -->
