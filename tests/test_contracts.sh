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
plan["plan_contract_version"] = 4
print(json.dumps(plan))
PY
)"

if python3 "$ROOT/scripts/validate_contract.py" --plan-json "$OLD_PLAN" --scope-json "$SCOPE_FIXTURE" >"$TMP_DIR/out.txt" 2>&1; then
  echo "expected plan contract validation to fail for old contract"
  exit 1
fi
grep -Fq "plan_contract_version too old: got 4, expected 5" "$TMP_DIR/out.txt"

NEW_PLAN="$(python3 - <<'PY' "$ROOT/tests/fixtures/plan_rust_src.json"
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
  plan = json.load(f)
plan["plan_contract_version"] = 6
print(json.dumps(plan))
PY
)"

if python3 "$ROOT/scripts/validate_contract.py" --plan-json "$NEW_PLAN" --scope-json "$SCOPE_FIXTURE" >"$TMP_DIR/out.txt" 2>&1; then
  echo "expected plan contract validation to fail for new contract"
  exit 1
fi
grep -Fq "plan_contract_version too new: got 6, expected 5" "$TMP_DIR/out.txt"

OLD_SCOPE_PLAN="$(python3 - <<'PY' "$ROOT/tests/fixtures/plan_rust_src.json"
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
  plan = json.load(f)
plan["scope"]["scope_contract_version"] = 2
print(json.dumps(plan))
PY
)"
OLD_SCOPE="$(python3 - <<'PY' "$OLD_SCOPE_PLAN"
import json
import sys

print(json.dumps(json.loads(sys.argv[1])["scope"]))
PY
)"

if python3 "$ROOT/scripts/validate_contract.py" --plan-json "$OLD_SCOPE_PLAN" --scope-json "$OLD_SCOPE" >"$TMP_DIR/out.txt" 2>&1; then
  echo "expected scope contract validation to fail for old contract"
  exit 1
fi
grep -Fq "scope_contract_version too old: got 2, expected 3" "$TMP_DIR/out.txt"

NEW_SCOPE_PLAN="$(python3 - <<'PY' "$ROOT/tests/fixtures/plan_rust_src.json"
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
  plan = json.load(f)
plan["scope"]["scope_contract_version"] = 4
print(json.dumps(plan))
PY
)"
NEW_SCOPE="$(python3 - <<'PY' "$NEW_SCOPE_PLAN"
import json
import sys

print(json.dumps(json.loads(sys.argv[1])["scope"]))
PY
)"

if python3 "$ROOT/scripts/validate_contract.py" --plan-json "$NEW_SCOPE_PLAN" --scope-json "$NEW_SCOPE" >"$TMP_DIR/out.txt" 2>&1; then
  echo "expected scope contract validation to fail for new contract"
  exit 1
fi
grep -Fq "scope_contract_version too new: got 4, expected 3" "$TMP_DIR/out.txt"

BAD_CARGO_ARGS="$(python3 - <<'PY' "$ROOT/tests/fixtures/plan_rust_src.json"
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
  scope = json.load(f)["scope"]
scope["cargo_args"] = []
print(json.dumps(scope))
PY
)"

if python3 "$ROOT/scripts/validate_contract.py" --plan-json "$PLAN_FIXTURE" --scope-json "$BAD_CARGO_ARGS" >"$TMP_DIR/out.txt" 2>&1; then
  echo "expected scope contract validation to fail for mismatched cargo_args"
  exit 1
fi
grep -Fq "scope.cargo_args does not match scope mode/crates" "$TMP_DIR/out.txt"

MISSING_SNAPSHOT="$(python3 - <<'PY' "$ROOT/tests/fixtures/plan_rust_src.json"
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
  plan = json.load(f)
del plan["inputs"]["snapshot_id"]
print(json.dumps(plan))
PY
)"

if python3 "$ROOT/scripts/validate_contract.py" --plan-json "$MISSING_SNAPSHOT" --scope-json "$SCOPE_FIXTURE" >"$TMP_DIR/out.txt" 2>&1; then
  echo "expected plan contract validation to fail without snapshot identity"
  exit 1
fi
grep -Fq "inputs.snapshot_id missing in planner output" "$TMP_DIR/out.txt"

BAD_SURFACE_SCOPE="$(python3 - <<'PY' "$ROOT/tests/fixtures/plan_rust_src.json"
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
  plan = json.load(f)
plan["surfaces"]["build"]["scope"]["cargo_args"] = []
print(json.dumps(plan))
PY
)"

if python3 "$ROOT/scripts/validate_contract.py" --plan-json "$BAD_SURFACE_SCOPE" --scope-json "$SCOPE_FIXTURE" >"$TMP_DIR/out.txt" 2>&1; then
  echo "expected plan contract validation to fail for a mismatched surface scope"
  exit 1
fi
grep -Fq "plan.surfaces.build.scope.cargo_args does not match scope mode/crates" "$TMP_DIR/out.txt"

BAD_CUSTOM_SURFACE="$(python3 - <<'PY' "$ROOT/tests/fixtures/plan_rust_src.json"
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
  plan = json.load(f)
plan["surfaces"]["custom:coverage"] = True
print(json.dumps(plan))
PY
)"

if python3 "$ROOT/scripts/validate_contract.py" --plan-json "$BAD_CUSTOM_SURFACE" --scope-json "$SCOPE_FIXTURE" >"$TMP_DIR/out.txt" 2>&1; then
  echo "expected plan contract validation to fail for a malformed custom surface"
  exit 1
fi
grep -Fq "plan.surfaces.custom:coverage missing or invalid in planner output" "$TMP_DIR/out.txt"

echo "contract validation tests passed"
