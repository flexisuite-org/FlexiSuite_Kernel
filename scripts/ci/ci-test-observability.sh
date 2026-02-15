#!/usr/bin/env bash
set -euo pipefail

for metric in worker_canvas_fallback_total manifest_signature_bypass_active_total manifest_signature_bypass_expired_total; do
  rg -n "$metric" docs/implementation_plan.md docs/verification_matrix.md >/dev/null || {
    echo "missing metric reference: $metric"
    exit 1
  }
done

echo "observability checks passed"
