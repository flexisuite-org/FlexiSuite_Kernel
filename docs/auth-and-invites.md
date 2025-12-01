# Auth & Invites (Alpha Draft)

目的: FlexiSuite Kernel における「アカウント作成」と「グループ参加」の仕組みを整理し、
Auth/Account UI や Kernel Admin UI が前提とするモデルを定義する。

## アカウントモデルの前提
- アカウントはメールアドレス単位で一意 (`User.email` はユニーク)。
- 1 アカウントは複数のグループに所属可能。
- 認証は JWT (15m) + Refresh (7d) 方針に従う（詳細は docs/security.md を参照）。

## サインアップポリシー（段階的ロールアウト）
- α版:
  - **招待リンク/招待コード必須**。招待がないとアカウントを作成できない。
  - 招待は「アカウント作成用」と「グループ参加用」で別物として扱う。
- 将来:
  - オープンサインアップ（誰でもメールアドレスで登録可能）を許可する。
  - ただしグループへの参加は引き続き招待ベース（もしくは管理者承認）とする。

## 1. アカウント作成用招待 `AccountInvite`

アカウント自体を作るための招待。α版では必須。

### モデル案（Prisma イメージ）

```prisma
model AccountInvite {
  id        String   @id @default(cuid())
  email     String   @unique   // 招待先メール（正規化済み小文字前提）
  code      String              // ユーザーが入力する短いコード or URL トークン
  createdAt DateTime @default(now())
  createdBy String?             // Kernel Admin の userId 等（任意）
  expiresAt DateTime?
  usedAt    DateTime?

  // 将来的に「初期グループも一緒に作る/紐づける」場合のフック
  initialGroupId String?
}
```

### 振る舞い
- サインアップ画面で「メールアドレス + 招待コード」を入力する。
- `AccountInvite` が存在し、`usedAt` が null かつ `expiresAt` が未来であれば有効。
- アカウント作成後に:
  - 該当 `AccountInvite.usedAt` をセット。
  - `User.email` は `AccountInvite.email` と一致させる（1メール=1アカウントを徹底）。
- オープンサインアップ移行後は、このモデルは「クローズド環境用/早期アクセス用」に用途を縮小する。

## 2. グループ参加用招待 `GroupInvite`

既存/新規アカウントを特定のグループへ参加させるための招待。
アカウント有無と関係なく発行できる。

### モデル案

```prisma
enum GroupInviteKind {
  LINK    // 汎用リンク（コードのみ）
  EMAIL   // 特定メールアドレス宛の招待
}

model GroupInvite {
  id        String          @id @default(cuid())
  groupId   String
  kind      GroupInviteKind
  email     String?         // kind=EMAIL のときのみ使用
  code      String          // URL トークン / 招待コード
  createdAt DateTime        @default(now())
  createdBy String?         // inviter userId
  expiresAt DateTime?
  acceptedAt DateTime?
  acceptedBy String?        // userId（参加が確定したアカウント）

  group     Group           @relation(fields: [groupId], references: [id])
}
```

### 利用パターン

1) 汎用招待リンク（`kind = LINK`, `email = null`）
- グループ管理画面から「招待リンクを発行」→ `code` を含む URL を共有。
- ユーザーが URL を踏むと:
  - ログイン済みなら: 「このグループに参加しますか？」を表示し、OK なら `GroupMember` 追加。
  - 未ログインなら: 先にログイン/サインアップへ誘導→戻って参加確認。

2) メールアドレス宛招待（`kind = EMAIL`, `email = 'user@example.com'`）
- グループ管理者が特定メールアドレス宛に招待を送る。
- 招待メール送信自体は別サービス（トリガーのみ Kernel が提供）。
- アカウントが **既に存在する** 場合:
  - ログイン後のランチャー（アプリホーム）で「招待中のグループ」カードを表示。
  - カードから「参加」を押すと `GroupMember` 追加、`acceptedAt` / `acceptedBy` 更新。
- アカウントが **まだ存在しない** 場合:
  - 同じメールアドレスでサインアップしようとすると:
    - 「このメールアドレスには ○○ グループから招待があります、参加しますか？」と表示。
    - OK ならアカウント作成後に `GroupMember` 追加、`acceptedAt` / `acceptedBy` 更新。

## 3. アカウントとグループの関係

- `User` は 1 メールアドレスにつき 1 レコード。重複アカウントは禁止。
- `GroupMember` により多対多:
  - 1 アカウントは複数の `Group` に所属できる。
  - 1 `Group` に対してユーザーごとにロール/権限を付与する（RBAC 詳細は docs/security.md）。

## 4. ランチャー（FlexiSuite ホーム）の役割

- ログイン後に表示される「ランチャー」画面では:
  - 所属しているグループ一覧（`GroupMember` に基づく）と、そのグループにインストール済みのアプリをタイル表示する。
  - 「保留中のグループ招待」一覧を表示する:
    - ログイン中ユーザーのメールアドレスに紐づく
      `GroupInvite(kind = EMAIL, acceptedAt IS NULL, expiresAt > now)`
      を収集して提示する。
  - 各タイルからアプリに遷移。招待カードからは「参加する/辞退する」などのアクションを提供。

## 5. Kernel Admin UI と一般ユーザー UI の境界

- **Kernel Admin UI**:
  - FlexiSuite Kernel 提供元だけが使う管理画面。通常のテナント/ユーザーには公開しない。
  - テナント/グループ/ユーザー/ロール/アプリ/コンポーネント/ロールアウト/監査ログ/エラーなど、
    カーネル全体に対する操作を行う。
  - `AccountInvite` や「全体の GroupInvite 統計」の管理もここから行う。

- **ユーザー/グループ向け管理画面**:
  - 各テナント/グループの管理者・メンバーが使う画面。
  - 自分が所属するグループの範囲内で:
    - グループ設定・メンバー管理
    - そのグループにインストール済みのアプリ/コンポーネント管理
    - 自分のマイ・コンポーネントの確認・適用
    を行う。
  - グループへの招待（`GroupInvite` の発行）もここから操作する。

このドキュメントを前提に、Auth/Account UI・ランチャー・Kernel Admin UI それぞれの
API と画面フローを設計していく。***
