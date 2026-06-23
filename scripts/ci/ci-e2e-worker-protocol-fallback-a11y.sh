#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib-search.sh"

search_lines 'test_protocol_fallback_a11y_i18n' tests/contract/worker/README.md >/dev/null

echo "::warning::worker protocol fallback a11y e2e is a scaffold check; no frontend runner is present yet"
echo "worker protocol fallback a11y e2e scaffold check passed"
