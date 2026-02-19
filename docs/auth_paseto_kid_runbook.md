# PASETO KID運用Runbook

## 1. 目的
このRunbookは `kernel-api` の `v4.public` トークン検証における `kid` 運用（鍵ローテーション・失効・互換モード）を、実装契約と同じ粒度で明文化する。

## 2. 環境変数契約
- `FLEXI_PASETO_V4_PUBLIC_KEY_B64URL`
  - 既定公開鍵（base64url / no padding / 32-byte Ed25519）を指定する。
  - 3セグメントのLegacyトークン検証に使用される。
- `FLEXI_PASETO_V4_ACTIVE_KID`
  - 現行発行鍵の `kid`。未設定時は `active`。
  - `active` 以外を設定する場合、対応する `FLEXI_PASETO_V4_PUBLIC_KEY_B64URL_<KID_NORMALIZED>` が必須。
- `FLEXI_PASETO_V4_NEXT_KIDS`
  - 次期鍵 `kid` のCSV。
- `FLEXI_PASETO_V4_RETIRED_KIDS`
  - 退役済みだが受理中の `kid` のCSV。
- `FLEXI_PASETO_V4_REVOKED_KIDS`
  - 即時拒否対象 `kid` のCSV。
- `FLEXI_PASETO_V4_ALLOW_LEGACY_NO_KID`
  - 既定値は `false`。
  - `true` の場合のみ、3セグメント（footerなし）トークンを受理候補にする。

### 2.1 KID別公開鍵の命名
- `ACTIVE/NEXT/RETIRED` で受理する `kid` は、対応する公開鍵を以下で設定する。
  - `FLEXI_PASETO_V4_PUBLIC_KEY_B64URL_<KID_NORMALIZED>`
- 正規化ルール:
  - 英数字: 大文字化してそのまま使用
  - 英数字以外: `_` に置換
- 例:
  - `kid = next-a` -> `FLEXI_PASETO_V4_PUBLIC_KEY_B64URL_NEXT_A`

## 3. 実装上の受理/拒否ルール
- 3セグメント token:
  - `FLEXI_PASETO_V4_ALLOW_LEGACY_NO_KID=true` かつ `FLEXI_PASETO_V4_REVOKED_KIDS` が空のときのみ受理。
  - 検証鍵は `FLEXI_PASETO_V4_PUBLIC_KEY_B64URL` を使用。
- 4セグメント token:
  - footer の JSON に `kid` が必須。
  - footer不正（JSON不正、`kid` 欠落、空文字など）は拒否。
  - `kid` は `active/next/retired` のいずれかでなければ拒否。
  - `revoked` に含まれる `kid` は常に拒否。
  - 検証鍵は `kid` に対応する公開鍵を選択して使用。

## 4. 初期化時の整合性チェック
以下は起動時にエラーとして扱われる（Fail-Closed）。
- `active_kid` が空文字。
- `active_kid` が `REVOKED` に含まれる。
- 同一 `kid` が複数カテゴリ（`NEXT/RETIRED/REVOKED`）に重複。
- `NEXT/RETIRED` に列挙された `kid` の公開鍵が未設定。
- `active` 以外の `FLEXI_PASETO_V4_ACTIVE_KID` を設定しているのに、対応公開鍵が未設定。
- 公開鍵が32-byte Ed25519 形式でない。

## 5. ローテーション手順（推奨）
1. 新鍵を生成し、`NEXT_KIDS` と対応公開鍵を投入する。
2. 発行側を新 `kid` に切替える（受理側は旧/新を同時受理）。
3. 旧 `kid` を `RETIRED_KIDS` に移す。
4. 監視期間後、旧 `kid` を `REVOKED_KIDS` に移し拒否へ切替える。
5. Legacy互換が不要になったら `FLEXI_PASETO_V4_ALLOW_LEGACY_NO_KID=false` に固定する。

## 6. 注意点
- `init_auth_config_with_public_key_and_revoked_kids(...)` は安全側のため Legacy許可を有効にしない。
- `KID_NORMALIZED` は衝突し得る（例: `a-b` と `a_b` は同一名になる）。実装はこの衝突を起動時エラーとして拒否するため、`kid` 命名は事前に一意化しておくこと。
- 4セグメント token は Legacyフォールバックされない。移行検証時は `kid` を必ず付与すること。
