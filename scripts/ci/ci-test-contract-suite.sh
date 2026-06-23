#!/usr/bin/env bash
set -euo pipefail

suite="${1:-}"
[[ -n "$suite" ]] || {
  echo "usage: $0 <suite>"
  exit 1
}

suite_dir="tests/contract/${suite}"
[[ -d "$suite_dir" ]] || {
  echo "missing suite directory: ${suite_dir}"
  exit 1
}

[[ -f "${suite_dir}/README.md" ]] || {
  echo "missing suite README: ${suite_dir}/README.md"
  exit 1
}

case "$suite" in
  auth | quota | idempotency | diagnostics | supplychain)
    cargo test -p contract-tests "${suite}::"
    ;;
  worker)
    echo "::warning::worker contract suite is a scaffold check; frontend/worker e2e implementation is still required"
    echo "contract suite scaffold check passed: ${suite}"
    ;;
  *)
    echo "unknown contract suite: ${suite}"
    exit 1
    ;;
esac
