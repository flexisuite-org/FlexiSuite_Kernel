# Design System: FlexiSuite "Lumina"

`frontend_design_skill.md` が定義する世界観と UX 原則を、  
実際のスタイル・トークン・コンポーネントレベルに落とし込むための仕様メモ。

このドキュメントはあくまで **「どう実装するか」** に集中し、  
「なぜそうするか」は `frontend_design_skill.md` を参照する。

---

## 1. Technology Stack & Styling

- **Framework**: Next.js App Router + Tailwind CSS
- **CSS**: Tailwind v4 （`@import "tailwindcss";` + `@theme`）ベース
- **Icon**: Lucide React
- **レイアウト単位**: 8pt グリッド（Tailwind の `2` = 8px, `4` = 16px）

---

## 2. Tokens & Theme

### 2.1 Colors

- Primary Accent
  - `--color-primary: #c2255d`
  - `--color-primary-foreground: #ffffff`
  - 用途: プライマリボタン、リンク強調、アクティブ状態、ブランド要素。

- Neutrals / Background
  - Base: `#ffffff` (`bg-white`)
  - Surface Alt: `#f8f9fa` (`bg-surface-alt` / `bg-slate-50`)
  - Border: `#e2e8f0` (`border-slate-200`)

- Text
  - Primary: `#1e293b` (`text-slate-800`)
  - Secondary: `#64748b` (`text-slate-500`)
  - Muted: `text-slate-400`

### 2.2 Typography

- Font Family（Tailwind config 想定）
  - `fontFamily.sans = ["Inter", "Noto Sans JP", "system-ui", "-apple-system", "BlinkMacSystemFont", "Segoe UI", "sans-serif"]`
- Base
  - Body: `text-sm`〜`text-base`, `leading-relaxed`
  - Heading: `font-semibold`〜`font-bold`, `tracking-tight`
  - Label: `text-sm font-medium`

### 2.3 Radius / Shadow

- Radius
  - ボタン: `rounded-lg`
  - カード: `rounded-xl`
  - アイコンコンテナ: `rounded-md` / `rounded-xl`

- Shadow
  - `shadow-soft`: `0 4px 20px -2px rgba(0,0,0,0.05)`（globals.css 実装）
  - `shadow-glow`: `0 0 15px rgba(194, 37, 93, 0.15)`（ブランド感を出したいときに限定使用）

---

## 3. Core Components

### 3.1 Button (`components/ui/button.tsx`)

- Variants
  - `primary`: `bg-primary text-white hover:bg-primary/90 shadow-sm`
  - `secondary`: `bg-primary/10 text-primary hover:bg-primary/20`
  - `ghost`: `hover:bg-slate-100 text-slate-700`
  - `outline`: `border border-slate-200 bg-white hover:bg-slate-50 text-slate-700`
  - `danger`: `bg-red-500 text-white hover:bg-red-600`
- Sizes
  - `sm`: `h-8 px-3 text-xs`
  - `md`: `h-10 px-4`
  - `lg`: `h-12 px-8 text-lg`
  - `icon`: `h-10 w-10`
- 共通
  - `rounded-lg`, `font-medium`, `focus-visible:ring-2 focus-visible:ring-primary/50`

### 3.2 Card (`components/ui/card.tsx`)

- Container
  - `rounded-xl border border-slate-200 bg-white text-slate-950 shadow-sm`
- Sections
  - Header: `flex flex-col space-y-1.5 p-6`
  - Content: `p-6 pt-0`
  - Footer: `flex items-center p-6 pt-0`

### 3.3 Input (`components/ui/input.tsx`)

- Base
  - `h-10 w-full rounded-lg border border-slate-200 bg-white px-3 py-2 text-sm`
  - `placeholder:text-slate-500`
  - `focus-visible:ring-2 focus-visible:ring-primary/50`

---

## 4. Layout Patterns

### 4.1 Auth Screens (Login / Signup)

- 構図
  - 背景: `bg-slate-50` + ごく薄いブランドグラデーション（`globals.css` 参照）
  - 中央に 1 カード (`max-w-md p-6`〜`p-8`)、上下左右十分な余白 (`py-16` 相当)
- コンテンツ
  - タイトル: プロダクト名 (`FlexiSuite Lumina`) を `text-2xl font-bold text-primary` で。
  - 説明文: 1〜2 行に抑え、`text-slate-500` で補足。
  - フォーム: 垂直方向に `space-y-4`、ラベルは `text-sm font-medium`。
  - エラー: `bg-red-50 text-red-600 p-3 rounded-lg text-sm flex items-center gap-2`。

### 4.2 Dashboard Shell / Sidebar Layout

- ベース
  - 背景: `bg-slate-50`
  - Sidebar: `w-72 bg-white/80 backdrop-blur-xl border-r border-slate-200/60 shadow-soft`
  - Main: 左にサイドバー分の `pl-72`（またはレスポンシブに変化）

- ナビゲーション
  - グループ／セクション見出しには `text-[10px] font-bold text-slate-400 uppercase tracking-widest`。
  - アクティブアイテムは `bg-primary/5 text-primary`, 左側にアクセントバー。

### 4.3 Launcher / App Grid

- ヘッダー
  - タイトル: 現在のグループ名（もしくは「Dashboard」）。
  - サブコピー: インストールアプリ数や次に取るべきアクションを短く記述。
- App Grid
  - グリッド: `grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-4`
  - タイル: `rounded-xl bg-white border border-slate-100 shadow-sm hover:shadow-md hover:border-primary/20 hover:-translate-y-1 transition-all`
  - アプリアイコン: `w-16 h-16 rounded-2xl bg-gradient-to-br from-slate-50 to-slate-100`

---

## 5. Empty States / Feedback

- Empty State
  - 中央揃えのカード (`max-w-md p-8 text-center`) を基本。  
  - アイコンは `bg-primary/10 rounded-full` の円内に配置。
  - メインメッセージ + サブメッセージ + 次のアクションボタン 1つを必ず用意する。

- Pending Invites
  - 背景にごく薄いブランドグラデ (`from-primary/5 to-transparent`) を使用。
  - CTA ボタンは `secondary` ボタンで、目立ちすぎずに案内する。

---

## 6. Motion & Transitions

- 共通
  - `@theme` に定義した `fade-in`, `slide-up`, `scale-in` を必要なところでのみ使用。
  - コンポーネント単位では `transition-all duration-200`〜`300` を基本。

- 推奨パターン
  - ロード中: スピナー (`Loader2`) + `animate-spin text-primary`
  - App タイル: hover 時にわずかな `scale` と `shadow` 強化。
  - グループ切り替え: コンテンツのフェード＋スライド。

---

## 7. 実務での使い方

- 新しい UI を作るとき:
  1. まず `frontend_design_skill.md` で考え方・世界観を確認する。
  2. 次に、この `design_system.md` から色・フォント・コンポーネントパターンを選ぶ。
  3. 既存の `components/ui` / `components/launcher` のスタイルと揃うように Tailwind クラスを設計する。
- 既存 UI をブラッシュアップするとき:
  - 背景色・フォント・余白・アクセントカラーがここで定義した内容とズレていないかをまずチェックする。
  - その上で、動線・階層・アクセシビリティの観点から `frontend_design_skill.md` に立ち戻って見直す。
