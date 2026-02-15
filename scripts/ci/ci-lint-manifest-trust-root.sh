#!/usr/bin/env bash
set -euo pipefail

for marker in manifest_trust_root.json manifest_trust_root.json.sig retired revoked; do
  rg -n "$marker" docs/implementation_plan.md docs/verification_matrix.md >/dev/null || {
    echo "missing trust-root marker: $marker"
    exit 1
  }
done

echo "manifest trust root lint stub passed"
