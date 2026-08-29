#!/usr/bin/env bash
set -euo pipefail

printf '%s\0' "$@" >> "$FAKE_CARGO_LOG"
printf '\0' >> "$FAKE_CARGO_LOG"
if [[ -n "${FAKE_CARGO_ENV_LOG:-}" ]]; then
  printf '%s\0%s\0%s\0' \
    "${AWS_ACCESS_KEY_ID-unset}" \
    "${AWS_SECRET_ACCESS_KEY-unset}" \
    "${AWS_SESSION_TOKEN-unset}" >> "$FAKE_CARGO_ENV_LOG"
fi

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
