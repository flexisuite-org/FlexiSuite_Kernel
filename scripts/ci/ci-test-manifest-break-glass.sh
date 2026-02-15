#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib-search.sh"

for marker in break-glass MANIFEST_SIGNATURE_BYPASS_ENABLED MANIFEST_SIGNATURE_BYPASS_DISABLED MANIFEST_SIGNATURE_BYPASS_USED '60分' '(tenant_id, manifest_digest)'; do
  search_lines "$marker" docs/implementation_plan.md docs/verification_matrix.md >/dev/null || {
    echo "missing break-glass contract marker: $marker"
    exit 1
  }
done

search_lines 'test_manifest_break_glass_scope_and_ttl' tests/contract/supplychain/README.md >/dev/null || {
  echo "missing break-glass test case scaffold"
  exit 1
}

echo "manifest break-glass contract stub passed"
