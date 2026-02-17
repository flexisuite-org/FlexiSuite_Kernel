# tasks: CodeRabbit → Jules 自動連携 (GitHubコメント状態管理版) v1.0.0

- [x] **Infrastructure Setup**
  - [x] `.github/workflows/jules-linkage.yml` の作成
    - [x] トリガー定義 (review, comment)
    - [x] Denoセットアップ
    - [x] スクリプト実行ステップ

- [x] **Script Implementation (`.github/scripts/jules-digest.ts`)**
  - [x] **Core Utilities**
    - [x] GitHub API Client setup (Octokit)
    - [x] ID Generator implementation (SHA-1)
  - [x] **Parsers**
    - [x] CodeRabbit Review Parser (Actionable/Nitpick extraction)
    - [x] Digest Comment Parser (Existing items & checkbox status)
    - [x] Jules Report Parser (Fixed/Skipped/Deferred extraction)
  - [x] **Logic**
    - [x] Reconciliation Logic (Merge incoming with existing status)
    - [x] Markdown Generator (Build Digest body)
    - [x] Sweep Logic (Identify unresolved items)
  - [x] **Main Loop**
    - [x] Event handling (Dispatch based on event type)
    - [x] API calls (Update/Create comment)

- [x] **Testing & Verification**
  - [x] Unit Tests for Parsers & ID Gen (using `deno test`)
  - [x] Manual verification with a mock PR
