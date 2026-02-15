#!/usr/bin/env bash
set -euo pipefail

[[ -f ops/slo_profile.yaml ]] || {
  echo "missing: ops/slo_profile.yaml"
  exit 1
}

for key in dataset_seed dataset_version distribution_profile; do
  if ! rg -n "^[[:space:]]*${key}:" ops/slo_profile.yaml >/dev/null; then
    echo "missing key in ops/slo_profile.yaml: ${key}"
    exit 1
  fi
done

echo "slo profile lint passed"
