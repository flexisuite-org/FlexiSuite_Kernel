# AI Agent Design Rules
AI エージェントがユーザーのために UI・フロントエンドを設計・生成するとき、  
**単なるコード生成ではなく「デザイン行為」を行うための規律** をまとめたルールセット。

本ドキュメントは、AI エージェントが “思考 → 判断 → 実装” を一貫して美しく行うために必要な  
デザイン原則・審美観・実装基準を定義する。

---

## 1. Purpose & Context First
AI はまず「何を、誰のために作るのか」を深く理解しなければならない。

- **目的**：この UI はどんな課題を解決する？
- **利用者**：誰が使い、どんな感情・状況にある？
- **ユースケース**：高速操作？視覚的ナビゲーション？没入感？
- **制約**：技術、環境、フレームワーク、アクセシビリティ。

> *UI は目的の奴隷であり、目的なき美しさは不要。  
> Context を理解しないまま作る UI はただの飾りである。*

---

## 2. Aesthetic Direction
AI エージェントは “中途半端な中庸” を絶対に選ばない。  
必ず **明確で強いデザイン方向性** を決めてから手を動かす。

例：
- Brutalist / Raw  
- Editorial / Magazine  
- Organic & Soft  
- Maximalist Chaos  
- Industrial / Utilitarian  
- Luxury Minimal  
- Retro-future  
- Toy-like Playfulness  
- Pastel & Gentle  
- Art Deco / Geometry

**重要**：方向性は *一つ* に絞る。混ぜない。曖昧にしない。  
方向性を選んだら、タイポグラフィ・色・レイアウト・動き・質感すべてがその世界観に従う。

---

## 3. Differentiation: Make It Unforgettable
AI の仕事は “既視感のある UI” を作ることではない。

- 何がこのインターフェースを唯一無二にするのか？
- どの 1 点が記憶に残るのか？
- どこに世界観が宿るのか？

例：大胆な余白、破格のレイアウト、苛烈なタイポ、温かい質感、異質なモーション。

> *「記号的特徴」がないデザインは AI スロップ化する。*

---

## 4. Typography: 意図をもったフォント選択
AI エージェントはフォントに最も自覚的であるべき。

- 絶対に避ける：Inter, Arial, Roboto, system fonts  
- 良い選択：  
  - 個性ある Display フォント × 上品な Body フォント  
  - 雰囲気を作るセリフ体  
  - Hostile / Brutalist な Grotesk  
  - 優雅な Humanist  
- フォントはデザイン方向性に従うこと。

フォントは UI の世界観を 6 割決める。  
迷ったら「テーマに忠実か？」で決める。

---

## 5. Color & Theme: 一貫する物語
- CSS variables を導入し、色の物語を統一する。
- 主役となる **1〜2 色** と、ドラマを作る **アクセント 1 色**。
- 均等に色を散らさない。  
  —— 配色は *力の集中* を行うためにある。

AI は「紫系グラデ × 白背景」などの凡庸な構造を避ける。

---

## 6. Spatial Composition
“どう置くか” は “何を置くか”と同じくらい本質的。

- 非対称  
- グリッド破り  
- 余白の緊張と緩和  
- 意図をもった密度  
- 層構造、重なり、奥行き  
- 斜め／ジグザグの流れ  
- 大胆なヒエラルキー設計

情報が整っているだけでは美しくない。  
美しさとは構図による緊張感である。

---

## 7. Motion: 高解像度の動き
モーションは装飾ではなく「体験の文法」。

- 最低限で最大効果の原則  
- ページロード：ステップ的な reveal や delay  
- Hover：反応が“生きている”ように  
- Scroll-trigger：文脈を生む  
- React なら Motion library を優先  
- HTML/CSS のみの場合は transition / keyframes で構築  

> 小さく散らすより、1〜2 ヶ所の“美しい大きな動き”を丁寧に仕上げる。

---

## 8. Backgrounds & Atmosphere
背景は平坦にしない。

- Gradient mesh  
- Noise / Grain  
- Organic textures  
- Layered transparency  
- Geometric patterns  
- Dramatic shadows  
- Artifacts / Borders / Frames  
- Custom cursors

背景は「空気の質感」をつくる領域。  
最も見落とされやすいが、完成度を決定づける。

---

## 9. Implementation Principles
AI が書くコードは常に **プロダクション品質** であるべき。

- ロジックは正確に動くこと  
- デザインを破壊しないセマンティック HTML  
- 再利用可能な適度なコンポーネント化  
- 変数化されたテーマ  
- 依存を必要最小限に  
- 見た目だけでなく構造も洗練されていること

Minimal な美学の場合：  
→ コードも無駄を削ぎ落とす。

Maximal な美学の場合：  
→ アニメーション・レイヤー・装飾に切れ味を要求する。

---

## 10. Anti-Slop Principles（AI スロップ防止規範）
AI エージェントは絶対に以下を避ける：

- Inter, Roboto, Arial, System UI  
- 汎用テンプレート的レイアウト  
- ランダムなガラスモーフィズム  
- 紫グラデ × 白背景  
- 散漫な UI パターンの寄せ集め  
- 無根拠な丸角・影  
- “AI っぽい”安全で無難な表現

> 無難さは犯罪。  
> 中途半端は悪。  
> AI は常に “意図された美” をつくる。

---

## 11. Creative Responsibility
AI エージェントは **毎回違う世界観を作る責任** がある。

- 通常は使用フォントを固定しない  
- 毎回別の美学を試す  
- コンポーネントを安易に流用しない  
- “選択理由” を常に説明できる状態でデザインする

---

## 12. Execution Workflow
AI のデザインプロセスは以下の順序に従う：

1. **Context & Purpose の理解**  
2. **Aesthetic Direction の決定**（1つに絞る）  
3. **Key Differentiator の設定**（記憶に残る一点）  
4. **Typography の決定**  
5. **Color Theme の構築**  
6. **Layout & Composition の設計**  
7. **Motion の設計**  
8. **Background / Atmosphere の設計**  
9. **Production-grade コードの生成**  
10. **方向性と整合性の再確認**

---

# Appendix: Design Prompts for AI Agents  
AI がデザインするときに自問するべき質問：

- この UI が解決する本質的な問題は何か？  
- このインターフェースを一言で表すと？  
- どのフォントが“世界観”を最も強く語る？  
- 色はどこで主張し、どこで黙らせる？  
- 何を大胆にし、何を極限まで抑制する？  
- このデザインで一番記憶に残る要素は何か？  
- コードは美学を壊さずに支えているか？  
- 誰も見たことのないものになっているか？

---

