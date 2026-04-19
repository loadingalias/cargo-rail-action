#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

PLAN_FIXTURE="$(cat "$ROOT/tests/fixtures/plan_rust_src.json")"
SCOPE_FIXTURE="$(python3 - <<'PY' "$ROOT/tests/fixtures/plan_rust_src.json"
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
  print(json.dumps(json.load(f)["scope"]))
PY
)"

python3 "$ROOT/scripts/validate_contract.py" \
  --plan-json "$PLAN_FIXTURE" \
  --scope-json "$SCOPE_FIXTURE"

PLAN_FILE="$TMP_DIR/plan.json"
SCOPE_FILE="$TMP_DIR/scope.json"
printf '%s' "$PLAN_FIXTURE" > "$PLAN_FILE"
printf '%s' "$SCOPE_FIXTURE" > "$SCOPE_FILE"

python3 "$ROOT/scripts/validate_contract.py" \
  --plan-json-file "$PLAN_FILE" \
  --scope-json-file "$SCOPE_FILE"

OLD_PLAN="$(python3 - <<'PY' "$ROOT/tests/fixtures/plan_rust_src.json"
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
  plan = json.load(f)
plan["plan_contract_version"] = 2
print(json.dumps(plan))
PY
)"

if python3 "$ROOT/scripts/validate_contract.py" --plan-json "$OLD_PLAN" --scope-json "$SCOPE_FIXTURE" >"$TMP_DIR/out.txt" 2>&1; then
  echo "expected plan contract validation to fail for old contract"
  exit 1
fi
grep -F "plan_contract_version too old: got 2, expected 3" "$TMP_DIR/out.txt"

NEW_SCOPE="$(python3 - <<'PY' "$ROOT/tests/fixtures/plan_rust_src.json"
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
  scope = json.load(f)["scope"]
scope["scope_contract_version"] = 2
print(json.dumps(scope))
PY
)"

if python3 "$ROOT/scripts/validate_contract.py" --plan-json "$PLAN_FIXTURE" --scope-json "$NEW_SCOPE" >"$TMP_DIR/out.txt" 2>&1; then
  echo "expected scope contract validation to fail for new contract"
  exit 1
fi
grep -F "scope_contract_version too new: got 2, expected 1" "$TMP_DIR/out.txt"

echo "contract validation tests passed"
