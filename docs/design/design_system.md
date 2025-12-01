# Design System: FlexiSuite "Lumina"

![FlexiSuite Lumina UI Mockup]

## 1. Core Philosophy
**"Clarity, Freshness, and Human Warmth"**
The design aims for a pristine, distraction-free environment that feels modern and efficient but retains a touch of human warmth through the vibrant accent color. It should feel like a well-organized, sunlit workspace.

## 2. Technology Stack & Styling
- **Framework**: Tailwind CSS
- **Rationale**: Ensures consistency, rapid development, and a modern, scalable CSS architecture.
- **Config**: Extend the default Tailwind theme with the specific color palette defined below.

## 3. Color Palette

### Primary Accent
- **Raspberry Pink**: `#c2255d`
  - *Tailwind Token*: `colors.primary` (DEFAULT: `#c2255d`, foreground: `#ffffff`)
  - *Usage*: Primary buttons, active states, key highlights, brand elements.
  - *Vibe*: Energetic, confident, distinct.

### Neutral / Backgrounds (Light Theme)
- **Surface Base**: `#ffffff` (Pure White)
  - *Tailwind Class*: `bg-white`
- **Surface Alt**: `#f8f9fa` (Off-white/Light Gray)
  - *Tailwind Class*: `bg-slate-50` or custom `bg-surface-alt`
  - *Usage*: Sidebar, cards, secondary areas.
- **Border**: `#e2e8f0` (Cool Gray)
  - *Tailwind Class*: `border-slate-200`
  - *Usage*: Subtle dividers.

### Text
- **Primary Text**: `#1e293b` (Slate 800)
  - *Tailwind Class*: `text-slate-800`
  - *Usage*: High contrast for readability, softer than pure black.
- **Secondary Text**: `#64748b` (Slate 500)
  - *Tailwind Class*: `text-slate-500`
  - *Usage*: Metadata, hints, descriptions.

## 4. Typography
**Font Family**: `Inter` (Google Fonts)
- *Tailwind Config*: `fontFamily: { sans: ['var(--font-inter)', 'sans-serif'] }`

- **Headings**: Bold, tight tracking (`tracking-tight`, `font-bold`).
- **Body**: Regular weight, comfortable line height (`leading-relaxed`).
- **Labels**: Medium weight, slightly smaller (`text-sm`, `font-medium`).

## 5. UI Components

### Buttons
- **Primary**:
  - `bg-primary text-white hover:bg-primary/90 shadow-sm rounded-lg px-4 py-2 transition-all`
- **Secondary**:
  - `bg-transparent border border-primary text-primary hover:bg-primary/5 rounded-lg px-4 py-2 transition-all`
- **Ghost**:
  - `bg-transparent text-slate-600 hover:bg-slate-100 rounded-lg px-4 py-2 transition-all`

### Cards & Containers
- **Style**:
  - `bg-white border border-slate-200 shadow-sm rounded-xl`
  - *Shadow*: `shadow-sm` or `shadow` (soft drop shadow).
- **Radius**: Generous rounding (`rounded-xl` or `rounded-2xl`) to feel friendly.

### Layout & Spacing
- **Whitespace**: Generous. Use `p-6`, `p-8`, `gap-6` to group related elements.
- **Grid**: 8pt grid system (Tailwind's default spacing scale is based on 4px, so `2`=8px, `4`=16px).

## 6. Visual Style
- **Iconography**: Lucide React (Thin stroke, rounded edges).
- **Imagery**: Clean, high-key photography or abstract geometric shapes with soft gradients.

## 7. Component Gallery

### Detailed Dashboard
![Detailed Dashboard]

### Settings Page
![Settings Page]

### Data Table
![Data Table]

### Login Screen
![Login Screen]
