# Event Outbox / RedisProducer 設定ガイド

## 目的

`kernel-data::event::RedisProducer` の設定ミスによるメッセージロスを防ぎ、
運用時に安全な値を選べるようにする。

## 重要な挙動

- `RedisProducer::new(client)`
  - デフォルトで `stream_maxlen = None`（トリムなし）
  - publish 時に `XADD ... MAXLEN` を付けない
- `RedisProducer::new_with_config(client, publish_timeout, stream_maxlen)`
  - `stream_maxlen = Some(n)` の場合のみ `XADD MAXLEN ~ n` を適用
  - `stream_maxlen = None` はトリムなし

## stream_maxlen のルール

- 最小値: `100`
- `Some(n)` で `n < 100` はエラー
- `None` は「Producer 側で削除しない」ため、信頼性優先のデフォルト

## 推奨値

- 信頼性最優先（標準）: `None`
- Redis メモリ制約が厳しい場合のみ: `Some(10_000)` 以上から検証開始
- `Some(100..999)` は短時間で未処理イベントを押し流すリスクが高く、非推奨

## 運用上の注意

- `MAXLEN ~ n` は近似トリムであり、未 ack メッセージでも削除される可能性がある
- `Some(n)` を使う場合は、以下を同時に監視すること
  - Consumer 遅延（lag）
  - Pending 件数
  - DLQ / リトライ率
- 信頼性要件が高い系（課金、監査、権限変更）は `None` を維持する

## 利用例

```rust
use std::time::Duration;
use kernel_data::event::RedisProducer;

// 推奨: 信頼性優先（トリムなし）
let producer = RedisProducer::new_with_config(
    client,
    Duration::from_secs(5),
    None,
).await?;

// メモリ制約がある場合のみ明示的に上限を設定
let bounded_producer = RedisProducer::new_with_config(
    client,
    Duration::from_secs(5),
    Some(20_000),
).await?;
```

## 変更時チェックリスト

- `stream_maxlen` を `Some` にする理由が SLO/容量で説明できる
- 負荷試験で、ピーク時の lag と loss-free 条件を確認済み
- 監視項目（lag / pending / retry）がダッシュボード化されている
