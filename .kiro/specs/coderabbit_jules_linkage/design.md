# design: CodeRabbit → Jules 自動連携 (GitHubコメント状態管理版) v1.0.0

## アーキテクチャ概要

* **Runtime**: GitHub Actions + Deno (v1.x)
* **Trigger**: `pull_request_review`, `pull_request_review_comment`, `issue_comment`
* **Infrastructure**: `.github/workflows/jules-linkage.yml` + `.github/scripts/jules-digest.ts`

## データ構造

### 1. Item (指摘項目)
```typescript
interface ReviewItem {
  id: string;          // CR-<hash> (Stable ID)
  type: 'actionable' | 'nitpick';
  content: string;     // Markdown text
  filePath?: string;
  line?: number;
  url?: string;        // GitHub comment URL
  status: 'open' | 'fixed' | 'skipped' | 'deferred'; // From Digest checkbox & Jules Report
}
```

### 2. Digest Context (状態)
```typescript
interface DigestContext {
  digestCommentId?: number;
  items: Map<string, ReviewItem>; // Key: CR-ID
}
```

## ロジック詳細

### 1. ID生成 (CR-ID)
CodeRabbitのコメント内容から一意なIDを生成する。再実行時も変わらないようにする。
SHA-1ハッシュを使用し、先頭10文字を採用する (`CR-<10chars>`)。
Input: `source_type` + `path` + `line` + `normalized_body` + (`comment_url` fallback to time)

### 2. Digest 更新 (Reconciliation)
* **Given**:
  * `incomingItems`: 今回の実行でCodeRabbitから抽出されたItem群
  * `existingItems`: 現在のDigestコメントからパースされたItem群（チェック状態含む）
  * `julesReport`: Julesが投稿した最新のレポートから抽出されたStatus宣言
* **When**: Update処理実行
* **Then**:
  * `existingItems` に存在し、`incomingItems` にも存在するItem → 状態（Check/Report）を維持して更新。
  * `existingItems` に存在し、`incomingItems` に存在しないItem → そのまま残す（Resolveされるまで消えない）。
  * `incomingItems` の新規Item → 追加。

### 3. Sweep (未完了再送)
* Digest内のItemで `status === 'open'` (未チェックかつReportで言及なし) のものを特定。
* Unresolvedセクションにリストアップ。
* `@Jules` メンションを付与。

## エラーハンドリング
* CodeRabbitのフォーマット変更:
  * パース失敗時はエラーログを出力し、可能な限り生のコメントURLを提示してFallbackする。
* API Rate Limit:
  * GitHub ActionsのTokenを使用するため通常は問題ないが、大量のコメントがある場合はページネーションを考慮（初期は直近50件程度で制限）。
