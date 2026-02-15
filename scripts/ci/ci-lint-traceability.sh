#!/usr/bin/env bash
set -euo pipefail

[[ -f docs/implementation_plan.md ]]
[[ -f docs/verification_matrix.md ]]

impl_reqs=$(rg -o 'REQ-[A-Z0-9-]+' docs/implementation_plan.md | sort -u)
matrix_reqs=$(rg -o 'REQ-[A-Z0-9-]+' docs/verification_matrix.md | sort -u)

if [[ "$impl_reqs" != "$matrix_reqs" ]]; then
  echo "REQ ID mismatch between implementation_plan and verification_matrix"
  diff <(echo "$impl_reqs") <(echo "$matrix_reqs") || true
  exit 1
fi

echo "traceability lint passed"
