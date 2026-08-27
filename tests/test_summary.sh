#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TEMPORARY="$(mktemp -d)"
trap 'rm -rf "$TEMPORARY"' EXIT
PLAN="$TEMPORARY/plan.json"
OUTPUT="$TEMPORARY/output"
SUMMARY="$TEMPORARY/summary.md"
HEAD_COMMIT="$(git -C "$ROOT" rev-parse HEAD)"
EXECUTABLE_SUFFIX=""
if [[ "${OS:-}" == Windows_NT ]]; then
  EXECUTABLE_SUFFIX=.exe
fi
VERIFIER="$TEMPORARY/cargo-rail$EXECUTABLE_SUFFIX"
rustc --edition=2021 "$ROOT/tests/fixtures/fake-plan-verifier.rs" -o "$VERIFIER"

python3 "$ROOT/tests/make_plan.py" rust "$PLAN" --head "$HEAD_COMMIT"
(
  cd "$ROOT"
  CARGO_RAIL_BIN="$VERIFIER" python3 scripts/plan.py publish "$PLAN" \
    --github-output "$OUTPUT" \
    --summary "$SUMMARY" \
    --reader "$ROOT/scripts/plan.py" \
    --install-method binary \
    --install-version 0.24.0
)

grep -Eq '^plan_file=/.+/plan\.json$' "$OUTPUT"
grep -Eq '^plan_reader=/.+/scripts/plan\.py$' "$OUTPUT"
grep -Eq '^plan_identity=plan-v8:sha256:[0-9a-f]{64}$' "$OUTPUT"
grep -Fxq 'required_work=["cargo.build","cargo.test","miri"]' "$OUTPUT"
grep -Fxq "head_commit=$HEAD_COMMIT" "$OUTPUT"
grep -Fq '## Cargo-Rail plan' "$SUMMARY"
grep -Fq '| Work | 3 required, 1 skipped |' "$SUMMARY"
grep -Fq "widened because complete evidence was unavailable: \`cargo.test\`" "$SUMMARY"
grep -Fq "| \`miri\` | \`changed_input\` | 1 Cargo package, 1 exact target |" "$SUMMARY"
if grep -Fq 'demo/src/lib.rs' "$SUMMARY"; then
  echo "summary leaked changed path details" >&2
  exit 1
fi

python3 "$ROOT/tests/make_plan.py" docs "$TEMPORARY/mismatch.json"
if (
  cd "$ROOT"
  CARGO_RAIL_BIN="$VERIFIER" FAKE_CARGO_RAIL_STATUS=2 \
    FAKE_CARGO_RAIL_STDERR='saved head commit does not match current authority' \
    python3 scripts/plan.py publish "$TEMPORARY/mismatch.json" \
    --github-output "$TEMPORARY/mismatch-output" \
    --summary "$TEMPORARY/mismatch-summary" \
    --reader "$ROOT/scripts/plan.py"
) > "$TEMPORARY/mismatch.log" 2>&1; then
  echo "publisher accepted a plan bound to another checkout" >&2
  exit 1
fi
grep -Fq 'saved head commit does not match current authority' "$TEMPORARY/mismatch.log"
test ! -e "$TEMPORARY/mismatch-output"
test ! -e "$TEMPORARY/mismatch-summary"

echo "summary tests passed"
