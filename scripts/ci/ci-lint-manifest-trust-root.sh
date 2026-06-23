#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib-search.sh"

for marker in manifest_trust_root.json manifest_trust_root.json.sig retired revoked; do
  search_lines "$marker" docs/implementation_plan.md docs/verification_matrix.md >/dev/null || {
    echo "missing trust-root marker: $marker"
    exit 1
  }
done

echo "::warning::manifest trust root lint is a scaffold check; signature verification is still required"
echo "manifest trust root lint scaffold check passed"
