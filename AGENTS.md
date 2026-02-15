# AI-DLC and Spec-Driven Development

Kiro-style Spec Driven Development implementation on AI-DLC (AI Development Life Cycle)

## Project Memory
Project memory keeps persistent guidance (steering, specs notes, component docs) so Codex honors your standards each run. Treat it as the long-lived source of truth for patterns, conventions, and decisions.

- Use `.kiro/steering/` for project-wide policies: architecture principles, naming schemes, security constraints, tech stack decisions, api standards, etc.
- Use local `AGENTS.md` files for feature or library context (e.g. `src/lib/payments/AGENTS.md`): describe domain assumptions, API contracts, or testing conventions specific to that folder. Codex auto-loads these when working in the matching path.
- Specs notes stay with each spec (under `.kiro/specs/`) to guide specification-level workflows.

## Project Context

### Paths
- Steering: `.kiro/steering/`
- Specs: `.kiro/specs/`

### Steering vs Specification

**Steering** (`.kiro/steering/`) - Guide AI with project-wide rules and context
**Specs** (`.kiro/specs/`) - Formalize development process for individual features

### Active Specifications
- Check `.kiro/specs/` for active specifications
- Use `/prompts:kiro-spec-status [feature-name]` to check progress

## Development Guidelines
- Think in English, generate responses in English. All Markdown content written to project files (e.g., requirements.md, design.md, tasks.md, research.md, validation reports) MUST be written in the target language configured for this specification (see spec.json.language).

## Minimal Workflow
- Phase 0 (optional): `/prompts:kiro-steering`, `/prompts:kiro-steering-custom`
- Phase 1 (Specification):
  - `/prompts:kiro-spec-init "description"`
  - `/prompts:kiro-spec-requirements {feature}`
  - `/prompts:kiro-validate-gap {feature}` (optional: for existing codebase)
  - `/prompts:kiro-spec-design {feature} [-y]`
  - `/prompts:kiro-validate-design {feature}` (optional: design review)
  - `/prompts:kiro-spec-tasks {feature} [-y]`
- Phase 2 (Implementation): `/prompts:kiro-spec-impl {feature} [tasks]`
  - `/prompts:kiro-validate-impl {feature}` (optional: after implementation)
- Progress check: `/prompts:kiro-spec-status {feature}` (use anytime)

## Development Rules
- 3-phase approval workflow: Requirements → Design → Tasks → Implementation
- Human review required each phase; use `-y` only for intentional fast-track
- Keep steering current and verify alignment with `/prompts:kiro-spec-status`
- Follow the user's instructions precisely, and within that scope act autonomously: gather the necessary context and complete the requested work end-to-end in this run, asking questions only when essential information is missing or the instructions are critically ambiguous.

## Steering Configuration
- Load entire `.kiro/steering/` as project memory
- Default files: `product.md`, `tech.md`, `structure.md`
- Custom files are supported (managed via `/prompts:kiro-steering-custom`)

---

## FlexiSuite Project Overview

FlexiSuite is a **"Flexible OS for the SaaS era"** — an operating system-level platform that democratizes AI-driven application development (Vibe Coding). This is NOT an MVP or a prototype. We are building a **production-grade, full-featured OS kernel** in Rust.

### Background
- The concept of "Custom UX" (AI-driven UI/UX self-modification) was validated through a separate product called **FlexiStudy** (a study management app with embedded Gemini CLI).
- This repository is the **generalized infrastructure** that makes Custom UX available to anyone, as a platform.

### Core Philosophy
- **Kernel/Userland Separation**: The Kernel (Rust) provides primitives (Identity, Storage, Events, Compute). Business logic runs in Userland (sandboxed JS/TS or Wasm).
- **App is Data**: Applications are JSON definitions rendered by a single "Universal Player" (Next.js). No per-user containers.
- **3-Tier Trust Model**: Kernel Provided (direct) → Store Verified (reviewed, initially iframe) → User Imported (iframe sandbox).
- **AI Native**: The system is designed to be equally usable by humans and AI agents.

### Tech Stack
- **Backend**: Rust (Axum + SeaORM + Tokio)
- **Frontend**: Next.js (Universal Player, single instance, multi-tenant)
- **Sandbox**: Deno Core (JS/TS) + Wasmtime (Wasm) hybrid
- **Database**: PostgreSQL with Row-Level Security (RLS)
- **Cache/Events**: Redis (Streams with abstraction layer)
- **Auth**: PASETO v4
- **Component Compiler**: SWC (Rust) + esm.sh CDN resolution

## Development Principles

> **These are absolute rules. No exceptions.**

1. **Production-grade always.** Every line of code must be written with production-quality robustness. No throwaway code, no "we'll fix it later", no shortcuts. If you are unsure about the correct approach, research and verify before implementing.
2. **No guesswork.** Never implement based on assumptions or speculation. If you don't know the answer, investigate the codebase, read documentation, or ask. Incorrect implementations are worse than no implementation.
3. **This is an OS, not an app.** Design decisions must account for multi-tenancy, security isolation, backward compatibility, and performance at scale. Think in terms of system contracts, not feature checklists.
4. **Tenant isolation is sacred.** Every database access MUST go through `TenantContext`. Raw SQL without tenant scoping MUST NOT exist in any public API. This is enforced at the type system level.
5. **Security by default.** User-generated code runs in sandboxes. External dependencies are isolated. Trust must be earned through verification, not assumed.
