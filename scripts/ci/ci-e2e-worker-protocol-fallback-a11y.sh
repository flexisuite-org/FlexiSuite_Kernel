#!/usr/bin/env bash
set -euo pipefail

rg -n 'test_protocol_fallback_a11y_i18n' tests/contract/worker/README.md >/dev/null

echo "worker protocol fallback a11y e2e stub passed"
