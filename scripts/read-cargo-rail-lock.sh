#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
LOCK="$ROOT/.github/cargo-rail.lock"

[[ -f "$LOCK" && ! -L "$LOCK" ]] || {
  printf 'Cargo-Rail lock must be a regular non-symbolic file: %s\n' "$LOCK" >&2
  exit 1
}
[[ "$(wc -l < "$LOCK" | tr -d ' ')" == 2 ]] || {
  printf 'Cargo-Rail lock must contain exactly two lines\n' >&2
  exit 1
}
[[ "$(grep -c '^version=' "$LOCK")" == 1 && "$(grep -c '^commit=' "$LOCK")" == 1 ]] || {
  printf 'Cargo-Rail lock must contain one version and one commit\n' >&2
  exit 1
}

VERSION="$(sed -n 's/^version=//p' "$LOCK")"
COMMIT="$(sed -n 's/^commit=//p' "$LOCK")"
[[ "$VERSION" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || {
  printf 'Cargo-Rail lock version must be an exact stable release\n' >&2
  exit 1
}
[[ "$COMMIT" =~ ^[0-9a-f]{40}$ ]] || {
  printf 'Cargo-Rail lock commit must be a full lowercase GitHub commit SHA\n' >&2
  exit 1
}

printf 'version=%s\ncommit=%s\n' "$VERSION" "$COMMIT"
