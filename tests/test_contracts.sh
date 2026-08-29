#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TEMPORARY="$(mktemp -d)"
trap 'rm -rf "$TEMPORARY"' EXIT
PLAN="$TEMPORARY/plan.json"

python3 "$ROOT/tests/make_plan.py" rust "$PLAN"
python3 "$ROOT/scripts/plan.py" validate "$PLAN"
python3 "$ROOT/scripts/plan.py" required "$PLAN" > "$TEMPORARY/actual-required"
printf '%s\n' '["cargo.build","cargo.test","miri"]' > "$TEMPORARY/expected-required"
cmp "$TEMPORARY/expected-required" "$TEMPORARY/actual-required"
[[ "$(python3 "$ROOT/scripts/plan.py" required "$PLAN")" == '["cargo.build","cargo.test","miri"]' ]]
[[ "$(python3 "$ROOT/scripts/plan.py" is-required "$PLAN" miri)" == true ]]
[[ "$(python3 "$ROOT/scripts/plan.py" is-required "$PLAN" docs)" == false ]]
[[ "$(python3 "$ROOT/scripts/plan.py" identity "$PLAN")" =~ ^plan-v8:sha256:[0-9a-f]{64}$ ]]

python3 "$ROOT/scripts/plan.py" cargo-args "$PLAN" miri > "$TEMPORARY/actual-args"
printf '%s\0%s\0' -p 'demo;echo-not-a-shell' > "$TEMPORARY/expected-args"
cmp "$TEMPORARY/expected-args" "$TEMPORARY/actual-args"
[[ "$(python3 "$ROOT/scripts/plan.py" cargo-scope "$PLAN" miri)" == packages ]]
python3 "$ROOT/scripts/plan.py" package-names "$PLAN" miri > "$TEMPORARY/actual-package-names"
printf '%s\0' demo > "$TEMPORARY/expected-package-names"
cmp "$TEMPORARY/expected-package-names" "$TEMPORARY/actual-package-names"
python3 "$ROOT/scripts/plan.py" target-args "$PLAN" miri > "$TEMPORARY/actual-targets"
printf '%s\0%s\0' --test contract > "$TEMPORARY/expected-targets"
cmp "$TEMPORARY/expected-targets" "$TEMPORARY/actual-targets"

if python3 "$ROOT/scripts/plan.py" is-required "$PLAN" unknown > "$TEMPORARY/unknown.out" 2>&1; then
  echo "reader accepted an unregistered work ID" >&2
  exit 1
fi
grep -Fq 'plan does not register work unknown' "$TEMPORARY/unknown.out"

mutate_and_reject() {
  local expression="$1"
  python3 - "$PLAN" "$TEMPORARY/invalid.json" "$expression" <<'PY'
import json
import sys

source, destination, expression = sys.argv[1:]
plan = json.load(open(source))
exec(expression, {"plan": plan})
json.dump(plan, open(destination, "w"))
PY
  if python3 "$ROOT/scripts/plan.py" validate "$TEMPORARY/invalid.json" > "$TEMPORARY/invalid.out" 2>&1; then
    echo "reader accepted invalid plan mutation: $expression" >&2
    exit 1
  fi
}

mutate_and_reject 'plan["plan_contract_version"] = 7'
mutate_and_reject 'plan["required"] = []'
mutate_and_reject 'plan["inputs"]["unknown"] = True'
mutate_and_reject 'plan["work"]["miri"]["scope"]["selection"]["cargo_args"] = ["--workspace"]'
mutate_and_reject 'plan["evidence"][next(iter(plan["evidence"]))]["complete"] = False'
mutate_and_reject 'plan["identity"] = "plan-v8:sha256:" + "0" * 64'

python3 "$ROOT/tests/make_plan.py" docs "$TEMPORARY/docs.json"
python3 "$ROOT/scripts/plan.py" validate "$TEMPORARY/docs.json"
[[ "$(python3 "$ROOT/scripts/plan.py" required "$TEMPORARY/docs.json")" == '["docs"]' ]]
[[ "$(python3 "$ROOT/scripts/plan.py" cargo-scope "$TEMPORARY/docs.json" miri)" == skipped ]]
python3 "$ROOT/scripts/plan.py" package-names "$TEMPORARY/docs.json" miri > "$TEMPORARY/docs-package-names"
[[ ! -s "$TEMPORARY/docs-package-names" ]]

python3 "$ROOT/tests/make_plan.py" workspace "$TEMPORARY/workspace.json"
[[ "$(python3 "$ROOT/scripts/plan.py" cargo-scope "$TEMPORARY/workspace.json" miri)" == workspace ]]
python3 "$ROOT/scripts/plan.py" package-names "$TEMPORARY/workspace.json" miri > "$TEMPORARY/workspace-package-names"
[[ ! -s "$TEMPORARY/workspace-package-names" ]]

python3 "$ROOT/tests/make_plan.py" punctuation "$TEMPORARY/punctuation.json"
python3 "$ROOT/scripts/plan.py" package-names "$TEMPORARY/punctuation.json" miri > "$TEMPORARY/punctuation-package-names"
printf '%s\0' demo-name_123 > "$TEMPORARY/expected-punctuation-package-names"
cmp "$TEMPORARY/expected-punctuation-package-names" "$TEMPORARY/punctuation-package-names"

python3 "$ROOT/tests/make_plan.py" duplicate "$TEMPORARY/duplicate.json"
if python3 "$ROOT/scripts/plan.py" package-names "$TEMPORARY/duplicate.json" miri > "$TEMPORARY/duplicate.out" 2>&1; then
  echo "reader accepted ambiguous package-name projection" >&2
  exit 1
fi
grep -Fq 'package names are ambiguous' "$TEMPORARY/duplicate.out"

echo "planner contract tests passed"
