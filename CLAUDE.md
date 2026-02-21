# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

FlexiSuite Kernel is a **Rust-based OS kernel for the SaaS era** — an operating system-level platform that democratizes AI-driven application development (Vibe Coding).

**This is NOT an MVP. This is an MDP (Minimal Desirable Product).**

We are building something that will be used reliably for the next **20 years**. Absolute stability and production-grade quality is non-negotiable. There is no "temporary" code, no "we'll fix it later", no shortcuts. If you're unsure about the correct approach, research and verify before implementing.

**Core Philosophy:**
- **Kernel/Userland Separation**: Rust Kernel provides primitives (Identity, Storage, Events, Compute). Business logic runs in sandboxed JS/TS or Wasm.
- **App is Data**: Applications are JSON definitions rendered by a single "Universal Player" (Next.js). No per-user containers.
- **3-Tier Trust Model**: Kernel Provided → Store Verified → User Imported (all in Web Workers)
- **Tenant Isolation is Sacred**: Every DB access MUST go through `TenantContext`. Raw SQL without tenant scoping is forbidden.

## Key Commands

### Building
```bash
# Build entire workspace
cargo build

# Build specific crate
cargo build -p kernel-api

# Build with release profile
cargo build --release
```

### Testing
```bash
# Run all tests
cargo test

# Run tests for specific crate
cargo test -p kernel-api

# Run contract tests
cargo test -p contract-tests

# Run single test by name
cargo test test_name_here
```

### Linting
```bash
# Run all lints (via deny.toml)
cargo deny check

# Run SQL security linter
./scripts/ci/ci-lint-sql-security.sh

# Run contract test suite
./scripts/ci/ci-test-contract-suite.sh
```

### Development
```bash
# Check Rust toolchain (MSRV: 1.85+)
rustc --version

# Format code
cargo fmt

# Check clippy
cargo clippy -- -D warnings
```

## Architecture

### Crate Structure

```
FlexiSuite Kernel (Rust Workspace)
├── kernel-core/       # Core types, traits, TenantContext, auth, events
├── kernel-api/        # Axum HTTP server, middleware, handlers
├── kernel-data/       # SeaORM entities, repositories, migrations
├── kernel-runtime/    # Deno Core + Wasmtime sandbox
├── kernel-registry/   # Component store, packages
├── kernel-archiver/   # Audit log archival
├── ops/linters/      # CI lint tools
└── tests/contract/   # Contract verification tests
```

### Key Design Patterns

1. **Tenant Isolation**: Enforced via `TenantContext` and `TenantScoped<T>` wrapper types at compile time
2. **Sealed Traits**: Repository patterns use sealed traits to prevent external implementation
3. **SECURITY DEFINER**: PostgreSQL functions use standard template (`search_path`, `pg_catalog` qualification, `REVOKE PUBLIC`)
4. **Contract Tests**: Core requirements verified via `tests/contract/` — these are the SSOT (Single Source of Truth)

### Important Documentation

- `docs/implementation_plan.md` — RFC 2119 contract specifications (MUST/SHOULD/MAY)
- `docs/verification_matrix.md` — REQ-* validation gates (PR-Blocking/Nightly/Drill)
- `docs/flexisuite-concept.md` — High-level architecture and philosophy
- `docs/negative-space-spec.md` — Things NOT to do

### Database

- PostgreSQL with Row-Level Security (RLS) — DEFAULT DENY policy
- SeaORM for ORM
- Redis for caching and event streaming (Redis Streams abstraction)

### Authentication

- PASETO v4 (public tokens) — not JWT
- Token format includes `kid` (key ID) for key rotation
- Tenant authorization via HMAC-signed tokens + `authorize_tenant()` function

## AI/Agent Development Workflow

This project uses **Kiro-style Spec-Driven Development**:

1. **Specification Phase**:
   - Use `/kiro:spec-init "description"` to start
   - `/kiro:spec-requirements {feature}` — write requirements
   - `/kiro:spec-design {feature}` — design approval
   - `/kiro:spec-tasks {feature}` — task breakdown

2. **Implementation Phase**:
   - `/kiro:spec-impl {feature}` — implement tasks

3. **Validation**: All high-risk REQ-* requirements must have PR-Blocking or Nightly tests

### Development Rules (Absolute - No Exceptions)

1. **Production-grade always.** This is a 20-year system. No throwaway code, no shortcuts, no "we'll fix it later". If unsure, research first.
2. **No guesswork.** Never implement based on assumptions. Investigate the codebase, read documentation, or ask. Incorrect implementations are worse than no implementation.
3. **This is an OS, not an app.** Think in system contracts, not feature checklists. Multi-tenancy, security isolation, backward compatibility, and performance at scale matter.
4. **Tenant isolation is sacred.** Every DB access MUST go through `TenantContext`. Raw SQL without tenant scoping MUST NOT exist in any public API. Enforced at the type system level.
5. **MDP over MVP.** Do not build "temporary" features. If included, it must be implemented with the quality of a finished OS. Fewer, perfectly working features is better than many half-baked ones.
