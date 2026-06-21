#!/usr/bin/env bash
set -euo pipefail

base_ref="${PRELAUNCH_COMPAT_BASE_REF:-}"

if [[ -z "${base_ref}" ]]; then
  if [[ -n "${GITHUB_BASE_REF:-}" ]]; then
    git fetch --no-tags --depth=1 origin "${GITHUB_BASE_REF}" >/dev/null 2>&1 || true
    base_ref="origin/${GITHUB_BASE_REF}"
  elif git rev-parse --verify origin/main >/dev/null 2>&1; then
    base_ref="origin/main"
  fi
fi

if [[ -z "${base_ref}" ]] || ! git rev-parse --verify "${base_ref}" >/dev/null 2>&1; then
  echo "prelaunch compat lint skipped: no base ref available"
  exit 0
fi

diff_range="${base_ref}...HEAD"
if ! git merge-base "${base_ref}" HEAD >/dev/null 2>&1; then
  echo "prelaunch compat lint warning: no merge base for ${base_ref}; using two-dot diff" >&2
  diff_range="${base_ref}..HEAD"
fi

changed_files="$(
  git diff --name-only --diff-filter=ACMR "${diff_range}" -- \
    ':(exclude)AGENTS.md' \
    ':(exclude)docs/**' \
    ':(exclude)**/docs/**' \
    ':(exclude)tests/**' \
    ':(exclude)**/tests/**' \
    ':(exclude)**/*_test.*' \
    ':(exclude)**/*_tests.*' \
    ':(exclude)**/*.test.*' \
    ':(exclude)**/*.md' \
    ':(exclude)**/README*' \
    ':(exclude).github/**' \
    ':(exclude)scripts/ci/ci-lint-prelaunch-compat.sh'
)"

[[ -n "${changed_files}" ]] || {
  echo "prelaunch compat lint passed: no implementation files changed"
  exit 0
}

# Place added exception evidence in the same diff hunk as the compatibility code:
# +// LAUNCH_BOUNDARY_COMPAT_EXCEPTION: basis=launched-boundary boundary=... deadline=... removal=... metric=... issue=...
# This lint verifies marker locality and required evidence keys; reviewers verify evidence quality.
compat_pattern='backwards?[ _-]?compat(ibility)?|legacy[ _-]?fallback|legacyFallback|allow[ _-]?legacy|allowLegacy|accept[ _-]?legacy|acceptLegacy|deprecated[ _-]?api|deprecatedApi|migration[ _-]?window|migrationWindow|old[ _-]?format|oldFormat|v1[ _-]?(transition|compat|accept|legacy)|compat(ibility)?[ _-]?shim|compatShim|legacy[ _-]?shim|legacyShim|grace[ _-]?window|graceWindow|後方互換|旧形式|移行窓'
v1_token_pattern='TokenVersion::V1|(^|[^[:alnum:]_])(validate|parse|accept|allow|verify|decode|authorize)[[:alnum:]_]*[_-]?v1([_-]?token)?([^[:alnum:]_]|$)|(^|[^[:alnum:]_])v1[_-]?(token|tenant|tenant_token|auth|credential)([^[:alnum:]_]|$)|(^|[^[:alnum:]_])(tenant|auth|token)[[:alnum:]_]*[_-]?v1([^[:alnum:]_]|$)'
auth_token_path_pattern='(^|/)(auth|tenant|token|identity|security)(/|[^/]*$)|(^|/)[^/]*(auth|tenant|token|identity|security)[^/]*$'
auth_token_v1_pattern='(^|[^[:alnum:]_])version[[:space:]]*(==|=)[[:space:]]*["'\''`]?v1["'\''`]?([^[:alnum:]_]|$)|(^|[^[:alnum:]_])(token|tenant_token|auth_token)?_?version[[:space:]]*(==|=)[[:space:]]*["'\''`]?v1["'\''`]?([^[:alnum:]_]|$)'
rust_deprecated_attr_pattern='^[[:space:]]*#\[[^]]*deprecated'
exception_pattern='(REQ-PRELAUNCH-COMPAT|LAUNCH_BOUNDARY_COMPAT_EXCEPTION)'

violations=0

shopt -s nocasematch

is_comment_only_addition() {
  local file="$1"
  local line="${2#+}"

  case "${file}" in
    *.rs)
      [[ "${line}" =~ ^[[:space:]]*(//|/\*|\*) ]]
      ;;
    *)
      [[ "${line}" =~ ^[[:space:]]*(//|#|--|/\*|\*|<!--) ]]
      ;;
  esac
}

line_matches_compat() {
  local file="$1"
  local line="${2#+}"

  [[ "${line}" =~ ${compat_pattern} ]] \
    || [[ "${line}" =~ ${v1_token_pattern} ]] \
    || {
      [[ "${file}" == *.rs ]] \
        && [[ "${line}" =~ ${rust_deprecated_attr_pattern} ]]
    } \
    || {
      [[ "${file}" =~ ${auth_token_path_pattern} ]] \
        && [[ "${line}" =~ ${auth_token_v1_pattern} ]]
    }
}

has_evidence_field() {
  local field="$1"
  local field_pattern="(^|[[:space:],;])${field}[[:space:]]*[:=][^[:space:],;]+"

  [[ "${hunk_added_text}" =~ ${field_pattern} ]]
}

has_complete_exception_evidence() {
  [[ "${hunk_has_exception}" -ne 0 ]] \
    && has_evidence_field "basis" \
    && has_evidence_field "boundary" \
    && has_evidence_field "deadline" \
    && has_evidence_field "removal" \
    && has_evidence_field "metric" \
    && has_evidence_field "issue" \
    && [[ "${hunk_added_text}" =~ (^|[[:space:],;])basis[[:space:]]*[:=](launched-boundary|real-data|external-integration|spec-exception)([[:space:],;]|$) ]]
}

flush_hunk() {
  local file="$1"

  if [[ "${hunk_has_added_compat}" -eq 0 ]]; then
    return
  fi

  if has_complete_exception_evidence; then
    return
  fi

  echo "Error: possible pre-launch compatibility code without complete nearby exception evidence in ${file}" >&2
  printf '%s\n' "${hunk_lines[@]}" >&2
  violations=1
}

while IFS= read -r file; do
  [[ -f "${file}" ]] || continue

  hunk_lines=()
  hunk_has_added_compat=0
  hunk_has_exception=0
  hunk_added_text=""
  in_hunk=0

  while IFS= read -r line || [[ -n "${line}" ]]; do
    if [[ "${line}" =~ ^@@ ]]; then
      flush_hunk "${file}"
      hunk_lines=("${line}")
      hunk_has_added_compat=0
      hunk_has_exception=0
      hunk_added_text=""
      in_hunk=1
      continue
    fi

    [[ "${in_hunk}" -eq 1 ]] || continue

    hunk_lines+=("${line}")

    if [[ "${line}" == +* && "${line}" != +++* ]]; then
      hunk_added_text+=" ${line#+}"

      if ! is_comment_only_addition "${file}" "${line}" && line_matches_compat "${file}" "${line}"; then
        hunk_has_added_compat=1
      fi

      if [[ "${line}" =~ ${exception_pattern} ]]; then
        hunk_has_exception=1
      fi
    fi
  done < <(git diff --unified=3 "${diff_range}" -- "${file}")

  flush_hunk "${file}"
done <<< "${changed_files}"

if [[ "${violations}" -ne 0 ]]; then
  cat >&2 <<'EOF'

Add REQ-PRELAUNCH-COMPAT or LAUNCH_BOUNDARY_COMPAT_EXCEPTION near the exception,
with basis=launched-boundary|real-data|external-integration|spec-exception,
boundary=..., deadline=..., removal=..., metric=..., and issue=... evidence fields.
EOF
  exit 1
fi

echo "prelaunch compat lint passed"
