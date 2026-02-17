# tasks: CodeRabbit → Jules 自動連携 (GitHubコメント状態管理版) v1.0.0

- [ ] **Infrastructure Setup**
  - [ ] `.github/workflows/jules-linkage.yml` の作成
    - [ ] トリガー定義 (review, comment)
    - [ ] Denoセットアップ
    - [ ] スクリプト実行ステップ

- [ ] **Script Implementation (`.github/scripts/jules-digest.ts`)**
  - [ ] **Core Utilities**
    - [ ] GitHub API Client setup (Octokit)
    - [ ] ID Generator implementation (SHA-1)
  - [ ] **Parsers**
    - [ ] CodeRabbit Review Parser (Actionable/Nitpick extraction)
    - [ ] Digest Comment Parser (Existing items & checkbox status)
    - [ ] Jules Report Parser (Fixed/Skipped/Deferred extraction)
  - [ ] **Logic**
    - [ ] Reconciliation Logic (Merge incoming with existing status)
    - [ ] Markdown Generator (Build Digest body)
    - [ ] Sweep Logic (Identify unresolved items)
  - [ ] **Main Loop**
    - [ ] Event handling (Dispatch based on event type)
    - [ ] API calls (Update/Create comment)

- [ ] **Testing & Verification**
  - [ ] Unit Tests for Parsers & ID Gen (using `deno test`)
  - [ ] Manual verification with a mock PR
