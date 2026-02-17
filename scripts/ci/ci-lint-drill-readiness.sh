#!/bin/bash
set -e

METADATA_FILE="docs/dr_readiness.yaml"

if [ ! -f "$METADATA_FILE" ]; then
  echo "Error: $METADATA_FILE not found."
  exit 1
fi

REQUIRED_KEYS=("runbook_updated_at" "owner" "next_drill_at" "last_drill_report")

for key in "${REQUIRED_KEYS[@]}"; do
  if ! grep -q "^$key:" "$METADATA_FILE"; then
    echo "Error: Missing required key '$key' in $METADATA_FILE"
    exit 1
  fi
done

echo "DR Readiness check passed."
