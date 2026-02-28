#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT_DIR"

ALLOWED_FILE="tests/contract/Cargo.toml"

mapfile -t CARGO_FILES < <(find . -name Cargo.toml -not -path './target/*' | sort)

violations=()
for file in "${CARGO_FILES[@]}"; do
  file="${file#./}"
  if perl -0777 -ne 'exit((/kernel-api\s*=\s*\{[^}]*features\s*=\s*\[[^\]]*test-utils/si) ? 0 : 1)' "$file"; then
    if [[ "$file" != "$ALLOWED_FILE" ]]; then
      violations+=("$file")
    fi
  fi
done

if (( ${#violations[@]} > 0 )); then
  echo "Error: kernel-api test-utils feature is only allowed in $ALLOWED_FILE"
  for v in "${violations[@]}"; do
    echo "  - $v"
  done
  exit 1
fi

echo "OK: kernel-api test-utils usage is limited to $ALLOWED_FILE"
cargo check --release -p kernel-api
cargo check --release -p kernel-core
