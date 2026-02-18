#!/usr/bin/env bash
set -euo pipefail

# Run the Traceability Linter
echo "Running Traceability Linter..."
cargo run -q -p ops-linters --bin traceability-linter -- --path .
