#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT_DIR"

ALLOWED_FILES=(
  "tests/contract/Cargo.toml"
  "kernel-registry/Cargo.toml"
)

mapfile -t CARGO_FILES < <(find . -name Cargo.toml -not -path './target/*' | sort)

violations=()
PACKAGES=("kernel-api" "kernel-core")
for file in "${CARGO_FILES[@]}"; do
  file="${file#./}"
  for pkg in "${PACKAGES[@]}"; do
    if perl -0777 -ne "exit((/${pkg}\\s*=\\s*\\{[^}]*features\\s*=\\s*\\[[^\\]]*test-utils/si) ? 0 : 1)" "$file"; then
      allowed="false"
      for allowed_file in "${ALLOWED_FILES[@]}"; do
        if [[ "$file" == "$allowed_file" ]]; then
          allowed="true"
          break
        fi
      done
      if [[ "$allowed" != "true" ]]; then
        violations+=("$file ($pkg)")
      fi
    fi
  done
done

if (( ${#violations[@]} > 0 )); then
  echo "Error: kernel-api/kernel-core test-utils feature is only allowed in:"
  for allowed_file in "${ALLOWED_FILES[@]}"; do
    echo "  - $allowed_file"
  done
  for v in "${violations[@]}"; do
    echo "  - $v"
  done
  exit 1
fi

echo "OK: kernel-api/kernel-core test-utils usage is limited to approved Cargo.toml files"
cargo check --release -p kernel-api
cargo check --release -p kernel-core
