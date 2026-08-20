#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

ruby -ryaml - "$ROOT/action.yaml" "$ROOT/cache/action.yaml" <<'RUBY'
planner = YAML.load_file(ARGV.fetch(0))
cache = YAML.load_file(ARGV.fetch(1))
violations = [planner, cache].flat_map do |action|
  action.fetch("runs").fetch("steps").each_with_object([]) do |step, found|
    found << step["name"] if step.fetch("run", "").match?(/\$\{\{\s*inputs\./)
  end
end
abort "action run blocks interpolate inputs directly: #{violations.join(', ')}" unless violations.empty?
abort "planner action still owns execution-job cache setup" if planner.fetch("inputs").key?("cache-url")
inputs = cache.fetch("inputs")
abort "cache URL is not required" unless inputs.fetch("url").fetch("required")
url_description = inputs.fetch("url").fetch("description")
abort "cache URL provider surface drifted" unless url_description.include?("AWS S3") && url_description.include?("Azure Blob Storage") && url_description.include?("Cloudflare R2")
abort "cache action advertises generic S3 compatibility" if url_description.include?("S3-compatible")
abort "cache mode default drifted" unless inputs.fetch("mode").fetch("default") == "read-write"
cache_step = cache.fetch("runs").fetch("steps").find { |step| step["name"] == "Configure compiler cache" }
abort "compiler-cache setup step missing" unless cache_step
abort "compiler-cache setup does not pass the persisted URL" unless cache_step.fetch("run").include?('--remote "$CACHE_URL"')
installer = cache.fetch("runs").fetch("steps").find { |step| step["name"] == "Install cargo-rail" }
abort "cache action does not share the installer" unless installer.fetch("run").include?("../scripts/install.sh")
RUBY

bash -n "$ROOT/scripts/install.sh"
grep -Fq 'cargo-rail-native-rustc-wrapper cargo-rail-native-rustc-worker' "$ROOT/scripts/install.sh"

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
plan["plan_contract_version"] = 5
print(json.dumps(plan))
PY
)"

if python3 "$ROOT/scripts/validate_contract.py" --plan-json "$OLD_PLAN" --scope-json "$SCOPE_FIXTURE" >"$TMP_DIR/out.txt" 2>&1; then
  echo "expected plan contract validation to fail for old contract"
  exit 1
fi
grep -Fq "plan_contract_version too old: got 5, expected 6" "$TMP_DIR/out.txt"

NEW_PLAN="$(python3 - <<'PY' "$ROOT/tests/fixtures/plan_rust_src.json"
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
  plan = json.load(f)
plan["plan_contract_version"] = 7
print(json.dumps(plan))
PY
)"

if python3 "$ROOT/scripts/validate_contract.py" --plan-json "$NEW_PLAN" --scope-json "$SCOPE_FIXTURE" >"$TMP_DIR/out.txt" 2>&1; then
  echo "expected plan contract validation to fail for new contract"
  exit 1
fi
grep -Fq "plan_contract_version too new: got 7, expected 6" "$TMP_DIR/out.txt"

OLD_SCOPE_PLAN="$(python3 - <<'PY' "$ROOT/tests/fixtures/plan_rust_src.json"
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
  plan = json.load(f)
plan["scope"]["scope_contract_version"] = 3
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
grep -Fq "scope_contract_version too old: got 3, expected 4" "$TMP_DIR/out.txt"

NEW_SCOPE_PLAN="$(python3 - <<'PY' "$ROOT/tests/fixtures/plan_rust_src.json"
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
  plan = json.load(f)
plan["scope"]["scope_contract_version"] = 5
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
grep -Fq "scope_contract_version too new: got 5, expected 4" "$TMP_DIR/out.txt"

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

MISSING_UNIVERSE="$(python3 - <<'PY' "$ROOT/tests/fixtures/plan_rust_src.json"
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
  plan = json.load(f)
del plan["resolution_universe"]
print(json.dumps(plan))
PY
)"

if python3 "$ROOT/scripts/validate_contract.py" --plan-json "$MISSING_UNIVERSE" --scope-json "$SCOPE_FIXTURE" >"$TMP_DIR/out.txt" 2>&1; then
  echo "expected plan contract validation to fail without a resolution universe"
  exit 1
fi
grep -Fq "plan.resolution_universe missing or invalid in planner output" "$TMP_DIR/out.txt"

BAD_UNIVERSE="$(python3 - <<'PY' "$ROOT/tests/fixtures/plan_rust_src.json"
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
  plan = json.load(f)
plan["resolution_universe"] = {"mode": "exact", "identity": "not-a-versioned-digest"}
print(json.dumps(plan))
PY
)"

if python3 "$ROOT/scripts/validate_contract.py" --plan-json "$BAD_UNIVERSE" --scope-json "$SCOPE_FIXTURE" >"$TMP_DIR/out.txt" 2>&1; then
  echo "expected plan contract validation to fail for an invalid resolution universe"
  exit 1
fi
grep -Fq "plan.resolution_universe.mode invalid in planner output" "$TMP_DIR/out.txt"
grep -Fq "plan.resolution_universe.identity missing or invalid in planner output" "$TMP_DIR/out.txt"

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
