#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib-search.sh"

search_lines 'SECURITY DEFINER|search_path = flexi, pg_catalog, pg_temp|REVOKE ALL ON FUNCTION' docs/implementation_plan.md >/dev/null || {
  echo "SQL security contract markers are missing from implementation_plan"
  exit 1
}

echo "sql security lint stub passed"
