# FlexiSuite Kernel

The core engine for FlexiSuite, a "Flexible OS for the SaaS era".

## Project Overview

FlexiSuite Kernel provides the foundational primitives for identity, storage, events, and sandboxed computation. It is designed to be AI-native and production-grade from Day 1.

## Minimum Supported Rust Version (MSRV)

The entire workspace requires **Rust 1.85** or newer.

- The `rust-toolchain.toml` file at the root specifies the exact toolchain version.
- `Cargo.toml` in individual crates (e.g., `kernel-runtime`) specifies `rust-version = "1.85"`.
- Many crates use `edition = "2024"`, which necessitates the 1.85+ toolchain.

## Getting Started

Refer to the documentation in `docs/` for architecture details and setup instructions.

## Contributing and Security

- Contribution workflow: [`CONTRIBUTING.md`](./CONTRIBUTING.md)
- Security reporting: [`SECURITY.md`](./SECURITY.md)
