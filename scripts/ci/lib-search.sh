#!/usr/bin/env bash

search_lines() {
  local pattern="$1"
  shift

  if command -v rg >/dev/null 2>&1; then
    rg -n -- "$pattern" "$@"
  else
    grep -nE -- "$pattern" "$@"
  fi
}

search_tokens() {
  local pattern="$1"
  shift

  if command -v rg >/dev/null 2>&1; then
    rg -o -- "$pattern" "$@"
  else
    grep -oE -- "$pattern" "$@"
  fi
}
