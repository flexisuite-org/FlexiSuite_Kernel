#!/usr/bin/env bash
set -euo pipefail

for nightly in \
  nightly:test-key-revocation-chaos \
  nightly:test-token-compat \
  nightly:test-token-version-usage \
  nightly:test-slo-smoke \
  nightly:test-slo-reproducibility \
  nightly:test-claim-pending-failover \
  nightly:test-hot-shard-detection \
  nightly:test-diagnostics-revocation-lag \
  nightly:test-lockfile-integrity \
  nightly:test-manifest-revocation-propagation \
  nightly:test-manifest-retired-window \
  nightly:test-sandbox-cpu-vs-wallclock; do
  rg -n "$nightly" docs/verification_matrix.md >/dev/null || {
    echo "missing nightly suite in verification_matrix: $nightly"
    exit 1
  }
done

echo "nightly reliability stub passed"
