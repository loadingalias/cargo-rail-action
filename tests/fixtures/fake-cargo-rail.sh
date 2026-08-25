#!/usr/bin/env bash
if [[ "${1:-}" == rail && "${2:-}" == --version ]]; then
  echo "cargo-rail 9.8.7"
  exit 0
fi
exit 0
