# spec: CodeRabbit → Jules 自動連携 (GitHubコメント状態管理版) v1.0.0

## 1. 概要と目的

* **Actionable** と **Nitpick** の抽出: CodeRabbitのレビューから指摘事項を抽出し、`@Jules` への指示として再構成する。
* **状態管理**: GitHubコメント（Digest）上のチェックリストをSoT（Source of Truth）とする。
* **完遂の保証**: 指摘事項が Resolve（Fix/Skip/Defer）されるまで、Julesに繰り返し提示する（Sweep）。
* **判断の明示**: Julesは全ての指摘に対して、修正(Fix)、見送り(Skip)、次期対応(Defer)のいずれかを理由と共に明示しなければならない。

## 2. ユーザー体験とインターフェース

### 2.1 CodeRabbit Digest for Jules (Digestコメント)
PRに1つだけ存在する固定コメント。Actionsが作成・更新する。

* **ヘッダー**: `## CodeRabbit Digest for Jules`
* **コンテンツ**:
  * **Actionable**: CodeRabbitの "Actionable" セクションの内容。
  * **Nitpick**: CodeRabbitの "Nitpick" セクションの内容（原文維持）。
  * **各項目（Item）**:
    * チェックボックス `- [ ]` 付き。
    * 安定ID `CR-<hash>` 付与。
    * 該当箇所の参照リンク。

### 2.2 Jules Review Result (レポートコメント)
Julesが作業結果を報告するコメント。

* **ヘッダー**: `### Jules Review Result`
* **区分**:
  * `#### Fixed`: 修正済み項目。
  * `#### Skipped (with rationale)`: 修正しない項目と理由。
  * `#### Deferred (with rationale)`: 後回しにする項目と理由。

## 3. システム挙動

### 3.1 安定ID (CR-ID) 生成
同一指摘を同一IDで追跡するため、以下の要素のハッシュ（SHA1先頭10桁等）から `CR-<10chars>` を生成する。

* `source_type`: (review_comment | review_body | issue_comment)
* `path`: ファイルパス（あれば）
* `line`: 行番号（あれば）
* `normalized_body`: 本文（trim済み、連続空白正規化）
* `comment_url`: コメントURL（または作成日時）

### 3.2 Actions トリガー
* イベント:
  * `pull_request_review` (submitted)
  * `pull_request_review_comment` (created)
  * `issue_comment` (created)
  * `pull_request` (synchronize) - Sweep用
* ゲート: `run-jules` ラベル、または CodeRabbit 由来の新規コメント検出時。

### 3.3 データフロー (Update Digest)
1. **Collect**: GitHub APIでレビュー、コメントを取得。CodeRabbit由来のものをフィルタ。
2. **Extract**: Actionable/Nitpickセクションを抽出、Item化。
3. **Reconcile**:
   * 既存Digestのチェック状態、Julesレポートの宣言（Skip/Defer）を読み取る。
   * 既存Itemの状態は維持。（未完了ならそのまま）
   * 新規Itemを追加。
4. **Post**: Digestコメントを更新（なければ作成）。

### 3.4 Sweep (未完了再送)
* トリガー: レビュー完走後、定期実行、など。
* 判定: Digest上で `[ ]` かつ、Julesレポートで言及がないItemは「未完了」。
* アクション: Digestを更新し、**"Unresolved Items"** セクションに未完了項目を再掲。冒頭に `@Jules` を付与してメンションを飛ばす。

## 4. Julesの行動ルール

1. **悉皆性**: 提示された全Itemに対して Fix/Skip/Defer のいずれかを決定する。
2. **Fix**: コードを修正し、コミットする。Digestのcheckboxを埋めるか、レポートでFixed宣言。
3. **Skip/Defer**: 理由（Rationale）を必ず添える。Deferの場合はIssue化が望ましい。
4. **Resolve条件**:
   * Minor/Trivial 指摘は、理由を述べた上でResolve（GitHub上のResolve Conversation）してよい。
   * 重大な指摘は勝手にResolveしない。

## 5. 制約事項

* **ループ防止**: PRあたりの最大サイクル数（例: 3）を設定。超過時は人間へエスカレーション（`needs-human` ラベル等）。
* **権限**: `GITHUB_TOKEN` またはBot用PATを使用。

## 6. 実装タスク（予定）

* [ ] `src/actions/jules-digest.ts`: ロジック実装
* [ ] `.github/workflows/jules-linkage.yml`: ワークフロー定義
* [ ] テスト: パーサー、ID生成、Digest更新ロジックの単体テスト
