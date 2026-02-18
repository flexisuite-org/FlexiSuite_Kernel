# 技術的負債Issue: `kernel-token` へのトークン生成ロジック抽出

**重要度**: 中 (保守性/整合性)
**対象**: `kernel-core` / `kernel-data` / テスト共通ヘルパー
**作成日**: 2026-02-18
**関連コード**:
- `kernel-core/src/auth/key_manager.rs` (`KeyManager::generate_tenant_token`)
- `kernel-data/tests/common/auth.rs` (`TestAuth::generate_tenant_token`)

## 背景
`kernel-data` のテストでは、`KeyManager::generate_tenant_token` と同等のロジック
（`v2:kid:ts:nonce:tenant_id:sig` の組み立てと HMAC 署名）を複製している。

複製の理由は依存循環であり、`kernel-data` が `kernel-core` を dev-dependency に追加すると
`kernel-data -> kernel-core -> kernel-data` となるため、直接呼び出しできない。

## 問題
- トークン仕様変更時に `kernel-core` と `kernel-data` テスト実装が乖離するリスクがある。
- 署名/フォーマットの仕様が 1 箇所に集約されておらず、回帰バグの温床になる。

## 提案
共有ロジックを新規 crate `kernel-token`（仮名）へ抽出し、`kernel-core` と `kernel-data`
の双方から参照可能にする。

## 期待スコープ
- トークン文字列フォーマット (`v2:kid:ts:nonce:tenant_id:sig`) の生成責務を移管。
- 署名対象メッセージ構築と署名（HMAC）処理を移管。
- 解析/検証で再利用可能な最小ユーティリティ（必要範囲のみ）を定義。
- 現在の `KeyManager::generate_tenant_token` 相当の振る舞いを固定する単体テストを同 crate に移管。
- 既存テストヘルパーの重複ロジックを削除し、共有 API を利用する。

## 移行ステップ案
1. `kernel-token` crate を workspace に追加し、トークン生成 API とテストを実装する。
2. `kernel-core::KeyManager::generate_tenant_token` を `kernel-token` の API 呼び出しに置換する。
3. `kernel-data/tests/common/auth.rs` の `TestAuth::generate_tenant_token` を共有 API 利用へ置換する。
4. `kernel-data` 側の重複ロジックを削除し、依存循環が発生しないことを確認する。
5. トークン互換性（フォーマット/署名）回帰テストを `kernel-core` と `kernel-data` 両側で実行する。

## 完了条件
- `kernel-core` と `kernel-data` テストのトークン生成ロジック重複が解消されている。
- 仕様変更時の更新箇所が `kernel-token` に集約されている。
- 既存トークン互換性を維持し、関連テストがすべて通過する。
