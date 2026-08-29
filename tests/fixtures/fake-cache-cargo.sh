#!/usr/bin/env bash
set -euo pipefail

printf '%s\0' "$@" >> "$FAKE_CARGO_LOG"
printf '\0' >> "$FAKE_CARGO_LOG"

if [[ "${1:-}" == rail && "${2:-}" == cache && "${3:-}" == setup ]]; then
  exit "${FAKE_SETUP_EXIT:-0}"
fi
if [[ "${1:-}" == rail && "${2:-}" == cache && "${3:-}" == status ]]; then
  printf '%s\n' "$FAKE_STATUS_JSON"
  exit "${FAKE_STATUS_EXIT:-0}"
fi
if [[ "${1:-}" == rail && "${2:-}" == cache && "${3:-}" == probe ]]; then
  printf '%s\n' "$FAKE_PROBE_JSON"
  exit "${FAKE_PROBE_EXIT:-0}"
fi

echo "unexpected fake Cargo invocation" >&2
exit 97
