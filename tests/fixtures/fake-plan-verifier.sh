#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 4 || "$1" != rail || "$2" != plan || "$3" != --verify ]]; then
  echo "unexpected cargo-rail verification arguments: $*" >&2
  exit 64
fi

if [[ -n "${FAKE_CARGO_RAIL_LOG:-}" ]]; then
  printf '%s\0' "$@" > "$FAKE_CARGO_RAIL_LOG"
fi
if [[ -n "${FAKE_CARGO_RAIL_STDOUT:-}" ]]; then
  printf '%s' "$FAKE_CARGO_RAIL_STDOUT"
fi
if [[ -n "${FAKE_CARGO_RAIL_STDERR:-}" ]]; then
  printf '%s\n' "$FAKE_CARGO_RAIL_STDERR" >&2
fi
exit "${FAKE_CARGO_RAIL_STATUS:-0}"
