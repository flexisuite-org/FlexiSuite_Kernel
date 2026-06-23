#!/usr/bin/env bash
set -euo pipefail

[[ -f tests/contract/worker/README.md ]]
[[ -f tests/contract/supplychain/README.md ]]

echo "::warning::frontend security e2e is a scaffold check; no frontend runner is present yet"
echo "frontend security e2e scaffold check passed"
