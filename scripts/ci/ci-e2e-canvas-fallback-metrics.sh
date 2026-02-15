#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib-search.sh"

search_lines 'test_canvas_fallback_accessibility_floor' tests/contract/worker/README.md >/dev/null
search_lines 'test_canvas_fallback_metric_emission' tests/contract/worker/README.md >/dev/null
search_lines 'worker_canvas_fallback_total' docs/implementation_plan.md docs/verification_matrix.md >/dev/null

echo "canvas fallback metrics e2e stub passed"
