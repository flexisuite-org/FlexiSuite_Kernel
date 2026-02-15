#!/usr/bin/env bash
set -euo pipefail

rg -n 'test_canvas_fallback_accessibility_floor' tests/contract/worker/README.md >/dev/null
rg -n 'test_canvas_fallback_metric_emission' tests/contract/worker/README.md >/dev/null
rg -n 'worker_canvas_fallback_total' docs/implementation_plan.md docs/verification_matrix.md >/dev/null

echo "canvas fallback metrics e2e stub passed"
