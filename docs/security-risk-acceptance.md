# セキュリティリスク受容台帳

この台帳は、RustSec、`cargo-audit`、`cargo-deny`、Dependency Review などで検出された供給網リスクや依存ポリシー例外を、修正までの一時的な例外として扱うための記録である。期限なしの ignore、恒久的な allowlist、追跡Issueなしの受容は禁止する。

`scripts/ci/ci-lint-risk-acceptance.sh` は、`.cargo/audit.toml` と `deny.toml` に記録された RustSec ignore と `cargo-deny` の `bans.skip` 例外が本台帳に存在し、期限切れでないことを検証する。

| Finding | Package / path | Reachability | Owner | Approver | Accepted until | Tracking issue | Compensating control | Status |
|---|---|---|---|---|---|---|---|---|
