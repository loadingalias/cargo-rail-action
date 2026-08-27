#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TEMPORARY="$(mktemp -d)"
trap 'rm -rf "$TEMPORARY"' EXIT

if [[ "$#" -eq 0 ]]; then
  CARGO_RAIL_BIN="${CARGO_RAIL_BIN:?CARGO_RAIL_BIN is required for a self-contained drift test}"
  SOURCE="$TEMPORARY/source"
  PLAN="$TEMPORARY/plan.json"
  READER="$ROOT/scripts/plan.py"
  bash "$ROOT/tests/create_workspace.sh" "$SOURCE" >/dev/null
  (
    cd "$SOURCE"
    "$CARGO_RAIL_BIN" rail plan --since HEAD~1 --json > "$PLAN"
  )
elif [[ "$#" -eq 3 ]]; then
  SOURCE="$1"
  PLAN="$2"
  READER="$3"
else
  echo "usage: $0 [workspace plan.json reader.py]" >&2
  exit 64
fi

SOURCE="$(cd "$SOURCE" && pwd -P)"
PLAN="$(cd "$(dirname "$PLAN")" && pwd -P)/$(basename "$PLAN")"
READER="$(cd "$(dirname "$READER")" && pwd -P)/$(basename "$READER")"

(
  cd "$SOURCE"
  python3 "$READER" verify-checkout "$PLAN"
)

for drift in unstaged staged untracked renamed deleted executable-mode head-commit; do
  WORKTREE="$TEMPORARY/plan-drift-$drift"
  LOG="$TEMPORARY/plan-drift-$drift.log"
  cp -R "$SOURCE" "$WORKTREE"
  case "$drift" in
    unstaged)
      printf '\n// unstaged drift\n' >> "$WORKTREE/crates/core/src/lib.rs"
      ;;
    staged)
      printf '\n// staged drift\n' >> "$WORKTREE/crates/core/src/lib.rs"
      git -C "$WORKTREE" add crates/core/src/lib.rs
      ;;
    untracked)
      printf '// untracked drift\n' > "$WORKTREE/crates/core/src/untracked.rs"
      ;;
    renamed)
      git -C "$WORKTREE" mv crates/core/src/lib.rs crates/core/src/renamed.rs
      ;;
    deleted)
      rm "$WORKTREE/crates/core/src/lib.rs"
      ;;
    executable-mode)
      chmod u+x "$WORKTREE/crates/core/src/lib.rs"
      ;;
    head-commit)
      printf '\n// committed drift\n' >> "$WORKTREE/crates/core/src/lib.rs"
      git -C "$WORKTREE" add crates/core/src/lib.rs
      git -C "$WORKTREE" commit --quiet -m "create head drift"
      ;;
  esac
  if (
    cd "$WORKTREE"
    python3 "$READER" verify-checkout "$PLAN"
  ) > "$LOG" 2>&1; then
    echo "saved-plan verification accepted $drift drift" >&2
    exit 1
  fi
  grep -Fq 'cargo-rail rejected current execution authority' "$LOG"
done

echo "saved-plan worktree drift tests passed"
