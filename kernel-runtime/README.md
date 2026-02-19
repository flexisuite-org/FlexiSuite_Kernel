# kernel-runtime

Sandbox execution runtime for FlexiSuite Kernel, supporting Deno (JS/TS) and WebAssembly environments.

## Minimum Supported Rust Version (MSRV)

This crate requires **Rust 1.85** or newer, as specified in `rust-toolchain.toml` and `Cargo.toml`. This is mandated by the use of the `2024` edition and specific modern dependencies.

## Platform Support

### Unix-like (Linux, macOS)
- Full support for both Deno and WASM runtimes.
- CPU time limiting is enforced using POSIX `pthread_getcpuclockid`.

### Windows
- **Limited Support**: Windows is currently not a primary target for production sandbox execution.
- CPU time limiting (via `pthread` APIs) is not available on Windows.
- Builds on Windows may require adjustments or may fail if Unix-specific primitives are explicitly used without guards (ongoing improvements).

## Deno Runtime セキュリティハードニング

- `deno_core` の組み込み実行は、強化された OS/コンテナサンドボックス配下でのみ実行してください。
- リリースビルドでは、`DenoSandbox` の実行に `FLEXISUITE_DENO_HARDENED=1` が必要です。
- この変数は、デプロイ環境で以下を満たす場合にのみ設定してください。
  - seccomp/AppArmor（または同等の syscall/profile 制限）
  - 最小権限ユーザーでの実行
  - シークレット分離およびホスト機密リソースへの非アクセス
