#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

run_summary() {
  local fixture="$1"
  local out="$2"
  python3 "$ROOT/scripts/render_summary.py" \
    --plan-json-file "$fixture" \
    --install-method binary \
    --install-version 0.12.0 \
    --base-ref origin/main > "$out"
}

# Determinism: same fixture => byte-identical summary output.
run_summary "$ROOT/tests/fixtures/plan_rust_src.json" "$TMP_DIR/summary_1.md"
run_summary "$ROOT/tests/fixtures/plan_rust_src.json" "$TMP_DIR/summary_2.md"
diff -u "$TMP_DIR/summary_1.md" "$TMP_DIR/summary_2.md"

# Golden regression checks.
run_summary "$ROOT/tests/fixtures/plan_rust_src.json" "$TMP_DIR/summary_rust_src.md"
diff -u "$ROOT/tests/golden/summary_rust_src.md" "$TMP_DIR/summary_rust_src.md"

run_summary "$ROOT/tests/fixtures/plan_docs_only.json" "$TMP_DIR/summary_docs_only.md"
diff -u "$ROOT/tests/golden/summary_docs_only.md" "$TMP_DIR/summary_docs_only.md"

python3 - <<'PY' > "$TMP_DIR/plan_large.json"
import json

trace = []
trace.append({"id": 1, "code": "FILE_KIND_RUST_SRC", "file": "crates/lib-01/src/lib.rs", "surface": "build"})
for idx in range(2, 24):
  trace.append(
    {
      "id": idx,
      "code": "TRANSITIVE_DEPENDS_ON_DIRECT",
      "crate": f"lib-{idx:02d}",
      "depends_on": "lib-01",
      "surface": "build",
    }
  )

plan = {
  "files": [{"path": "crates/lib-01/src/lib.rs"}],
  "impact": {
    "direct_crates": [f"lib-{idx:02d}" for idx in range(1, 15)],
    "transitive_crates": [f"dep-{idx:02d}" for idx in range(1, 5)],
  },
  "surfaces": {
    "build": {"enabled": True, "reasons": list(range(1, 24))},
    "test": {"enabled": True, "reasons": list(range(1, 24))},
  },
  "scope": {
    "mode": "crates",
    "crates": [f"pkg-{idx:02d}" for idx in range(1, 17)],
    "surfaces": {"build": True, "test": True},
  },
  "trace": trace,
}

print(json.dumps(plan))
PY

run_summary "$TMP_DIR/plan_large.json" "$TMP_DIR/summary_large.md"
grep -F "**Changed direct crates (14):** \`lib-01, lib-02, lib-03, lib-04, lib-05, lib-06, lib-07, lib-08, lib-09, lib-10, lib-11, lib-12, ... +2 more\`" "$TMP_DIR/summary_large.md"
grep -F "**Execution crates (16):** \`pkg-01, pkg-02, pkg-03, pkg-04, pkg-05, pkg-06, pkg-07, pkg-08, pkg-09, pkg-10, pkg-11, pkg-12, ... +4 more\`" "$TMP_DIR/summary_large.md"
grep -F "**Sample trace entries (20 of 23)**" "$TMP_DIR/summary_large.md"
grep -F -- "- ... +3 more trace entries" "$TMP_DIR/summary_large.md"

echo "summary tests passed"
