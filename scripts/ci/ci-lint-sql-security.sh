#!/usr/bin/env bash
set -euo pipefail

# Run the SQL Security Linter
echo "Running SQL Security Linter..."
cargo run -q -p ops-linters --bin sql-linter -- --path .

echo "sql security lint passed"
