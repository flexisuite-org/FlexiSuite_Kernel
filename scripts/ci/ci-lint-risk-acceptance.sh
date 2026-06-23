#!/usr/bin/env bash
set -euo pipefail

ledger="docs/security-risk-acceptance.md"
if [[ "${GITHUB_ACTIONS:-}" == "true" || "${CI:-}" == "true" || -n "${GITHUB_RUN_ID:-}" ]]; then
  today="$(date -u +%F)"
else
  today="${RISK_ACCEPTANCE_TODAY:-$(date -u +%F)}"
fi

if [[ ! -s "${ledger}" ]]; then
  echo "risk acceptance lint error: ${ledger} is missing or empty" >&2
  exit 1
fi

trim_cell() {
  local value="$1"
  value="${value//\`/}"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "${value}"
}

collect_configured_risks() {
  python3 - .cargo/audit.toml deny.toml <<'PY'
import pathlib
import re
import sys

try:
    import tomllib
except ModuleNotFoundError:
    try:
        import tomli as tomllib
    except ModuleNotFoundError:
        print(
            "risk acceptance lint error: Python 3.11+ tomllib or third-party tomli is required",
            file=sys.stderr,
        )
        sys.exit(1)

rustsec_id = re.compile(r"^RUSTSEC-[0-9]{4}-[0-9]+$")

for path_arg in sys.argv[1:]:
    path = pathlib.Path(path_arg)
    if not path.exists():
        print(f"risk acceptance lint error: required policy file is missing: {path}", file=sys.stderr)
        sys.exit(1)

    data = tomllib.loads(path.read_text(encoding="utf-8"))
    advisories = data.get("advisories", {})
    ignored_advisories = advisories.get("ignore", [])
    if isinstance(ignored_advisories, str):
        ignored_advisories = [ignored_advisories]

    for advisory in ignored_advisories:
        advisory_id = None
        if isinstance(advisory, str):
            advisory_id = advisory
        elif isinstance(advisory, dict):
            advisory_id = advisory.get("id")

        if isinstance(advisory_id, str) and rustsec_id.fullmatch(advisory_id):
            print(advisory_id)

    bans = data.get("bans", {})
    skipped_bans = bans.get("skip", [])
    if isinstance(skipped_bans, dict):
        skipped_bans = [skipped_bans]

    for skipped in skipped_bans:
        if not isinstance(skipped, dict):
            continue
        name = skipped.get("name")
        version = skipped.get("version", "")
        if isinstance(name, str) and name:
            print(f"cargo-deny:bans.skip:{name}@{version}")
PY
}

ids="$(collect_configured_risks | sort -u)"

if [[ -z "${ids}" ]]; then
  echo "risk acceptance lint passed: no RustSec ignore or cargo-deny bans.skip entries found"
  exit 0
fi

violations=0

while IFS= read -r id; do
  [[ -n "${id}" ]] || continue

  row="$(grep -F "| \`${id}\` |" "${ledger}" || true)"
  if [[ -z "${row}" ]]; then
    echo "risk acceptance lint error: ${id} is ignored but has no ledger row" >&2
    violations=1
    continue
  fi
  row_count="$(printf '%s\n' "${row}" | grep -c . || true)"
  if [[ "${row_count}" -gt 1 ]]; then
    echo "risk acceptance lint error: ${id} has duplicate ledger rows" >&2
    violations=1
    continue
  fi

  IFS='|' read -r _ advisory package_path reachability owner approver accepted_until tracking control status _ <<< "${row}"
  advisory="$(trim_cell "${advisory}")"
  package_path="$(trim_cell "${package_path}")"
  reachability="$(trim_cell "${reachability}")"
  owner="$(trim_cell "${owner}")"
  approver="$(trim_cell "${approver}")"
  accepted_until="$(trim_cell "${accepted_until}")"
  tracking="$(trim_cell "${tracking}")"
  control="$(trim_cell "${control}")"
  status="$(trim_cell "${status}")"

  missing_required=0
  for required in "${advisory}" "${package_path}" "${reachability}" "${owner}" "${approver}" "${accepted_until}" "${tracking}" "${control}" "${status}"; do
    if [[ -z "${required}" || "${required}" == "N/A" || "${required}" == "不明" ]]; then
      echo "risk acceptance lint error: ${id} ledger row has an empty/N/A/unknown required field" >&2
      violations=1
      missing_required=1
      break
    fi
  done
  [[ "${missing_required}" -eq 0 ]] || continue

  if [[ ! "${accepted_until}" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]]; then
    echo "risk acceptance lint error: ${id} accepted-until is not YYYY-MM-DD: ${accepted_until}" >&2
    violations=1
  elif [[ "${accepted_until}" < "${today}" ]]; then
    echo "risk acceptance lint error: ${id} acceptance expired on ${accepted_until}" >&2
    violations=1
  fi

  if [[ ! "${tracking}" =~ ^https://github\.com/flexisuite-org/FlexiSuite_Kernel/issues/[0-9]+$ ]]; then
    echo "risk acceptance lint error: ${id} tracking issue must be a FlexiSuite_Kernel GitHub issue URL" >&2
    violations=1
  fi

  if [[ "${status}" != "accepted" ]]; then
    echo "risk acceptance lint error: ${id} status must be accepted while the advisory is ignored" >&2
    violations=1
  fi
done <<< "${ids}"

if [[ "${violations}" -ne 0 ]]; then
  exit 1
fi

echo "risk acceptance lint passed"
