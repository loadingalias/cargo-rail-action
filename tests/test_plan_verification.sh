#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TEMPORARY="$(mktemp -d)"
trap 'rm -rf "$TEMPORARY"' EXIT
PLAN="$TEMPORARY/plan.json"
PLAN_RESOLVED="$(python3 -c 'import pathlib, sys; print(pathlib.Path(sys.argv[1]).resolve())' "$PLAN")"
WORKSPACE="$TEMPORARY/workspace"
FAKE_BIN="$TEMPORARY/bin"
LOG="$TEMPORARY/cargo-rail.log"

mkdir -p "$WORKSPACE" "$FAKE_BIN"
EXECUTABLE_SUFFIX=""
if [[ "${OS:-}" == Windows_NT ]]; then
  EXECUTABLE_SUFFIX=.exe
fi
rustc --edition=2021 "$ROOT/tests/fixtures/fake-plan-verifier.rs" \
  -o "$FAKE_BIN/cargo-rail$EXECUTABLE_SUFFIX"
python3 "$ROOT/tests/make_plan.py" rust "$PLAN"

if (
  cd "$WORKSPACE"
  PATH="$FAKE_BIN:$PATH" FAKE_CARGO_RAIL_LOG="$LOG" \
    python3 "$ROOT/scripts/plan.py" verify-checkout "$PLAN"
) > "$TEMPORARY/accepted.out" 2> "$TEMPORARY/accepted.err"; then
  :
else
  status=$?
  echo "native cargo-rail verifier fixture failed with exit code $status" >&2
  cat "$TEMPORARY/accepted.out" >&2
  cat "$TEMPORARY/accepted.err" >&2
  exit "$status"
fi
test ! -s "$TEMPORARY/accepted.out"
test ! -s "$TEMPORARY/accepted.err"
printf 'rail\0plan\0--verify\0%s\0' "$PLAN_RESOLVED" > "$TEMPORARY/expected.log"
cmp "$TEMPORARY/expected.log" "$LOG"

if (
  cd "$WORKSPACE"
  PATH="$FAKE_BIN:$PATH" FAKE_CARGO_RAIL_STATUS=2 \
    FAKE_CARGO_RAIL_STDERR='saved worktree capture does not match current authority' \
    python3 "$ROOT/scripts/plan.py" verify-checkout "$PLAN"
) > "$TEMPORARY/rejected.out" 2> "$TEMPORARY/rejected.err"; then
  echo "plan reader accepted Cargo-Rail's authority rejection" >&2
  exit 1
fi
test ! -s "$TEMPORARY/rejected.out"
grep -Fq 'cargo-rail rejected current execution authority with exit code 2' "$TEMPORARY/rejected.err"
grep -Fq 'saved worktree capture does not match current authority' "$TEMPORARY/rejected.err"

if (
  cd "$WORKSPACE"
  PATH="$FAKE_BIN:$PATH" FAKE_CARGO_RAIL_STDOUT='unexpected' \
    python3 "$ROOT/scripts/plan.py" verify-checkout "$PLAN"
) > "$TEMPORARY/stdout.out" 2> "$TEMPORARY/stdout.err"; then
  echo "plan reader accepted verifier stdout" >&2
  exit 1
fi
test ! -s "$TEMPORARY/stdout.out"
grep -Fq 'saved-plan verification emitted unexpected stdout' "$TEMPORARY/stdout.err"

if CARGO_RAIL_BIN="$TEMPORARY/missing-cargo-rail" \
  python3 "$ROOT/scripts/plan.py" verify-checkout "$PLAN" \
  > "$TEMPORARY/missing.out" 2> "$TEMPORARY/missing.err"; then
  echo "plan reader accepted an unavailable configured Cargo-Rail binary" >&2
  exit 1
fi
test ! -s "$TEMPORARY/missing.out"
grep -Fq 'cannot execute cargo-rail saved-plan verification' "$TEMPORARY/missing.err"

echo "saved-plan verification tests passed"
