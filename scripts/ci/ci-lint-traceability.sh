#!/usr/bin/env bash
set -euo pipefail

# Run the Traceability Linter
echo "Running Traceability Linter..."
cargo run --release -q -p ops-linters --bin traceability-linter -- --path .

echo "traceability lint passed"
