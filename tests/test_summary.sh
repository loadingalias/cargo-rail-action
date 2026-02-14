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
    --install-version 0.10.0 \
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

echo "summary tests passed"
