#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HISTORY="$ROOT/scripts/ensure_history.sh"
SELECT="$ROOT/scripts/select_base.py"
TEMPORARY="$(mktemp -d)"
trap 'rm -rf "$TEMPORARY"' EXIT

init_remote_repo() {
  local name="$1"
  INIT_REMOTE="$TEMPORARY/$name-remote.git"
  INIT_WORK="$TEMPORARY/$name-work"
  git init --bare --initial-branch=main "$INIT_REMOTE" >/dev/null
  git clone "$INIT_REMOTE" "$INIT_WORK" >/dev/null
  git -C "$INIT_WORK" config user.email test@example.com
  git -C "$INIT_WORK" config user.name Test
}

run_history() {
  local repo="$1" base_ref="$2" output="$3" merge_base="${4:-false}"
  (
    cd "$repo"
    BASE_REF="$base_ref" USE_MERGE_BASE="$merge_base" GITHUB_OUTPUT="$output" bash "$HISTORY"
  )
}

make_diverged_remote() {
  init_remote_repo "$1"
  printf 'base\n' > "$INIT_WORK/file.txt"
  git -C "$INIT_WORK" add file.txt
  git -C "$INIT_WORK" commit -m base >/dev/null
  BASE_SHA="$(git -C "$INIT_WORK" rev-parse HEAD)"
  git -C "$INIT_WORK" tag -a base-v1 -m base-v1
  git -C "$INIT_WORK" push -u origin main --tags >/dev/null
  git -C "$INIT_WORK" checkout -b feature >/dev/null
  printf 'feature\n' > "$INIT_WORK/feature.txt"
  git -C "$INIT_WORK" add feature.txt
  git -C "$INIT_WORK" commit -m feature >/dev/null
  git -C "$INIT_WORK" push -u origin feature >/dev/null
  git -C "$INIT_WORK" checkout main >/dev/null
  printf 'main\n' > "$INIT_WORK/main.txt"
  git -C "$INIT_WORK" add main.txt
  git -C "$INIT_WORK" commit -m 'main advance' >/dev/null
  git -C "$INIT_WORK" push >/dev/null
  MAIN_SHA="$(git -C "$INIT_WORK" rev-parse HEAD)"
}

make_diverged_remote history
CLONE="$TEMPORARY/clone"
git clone --depth 1 --branch feature "file://$INIT_REMOTE" "$CLONE" >/dev/null

run_history "$CLONE" "$BASE_SHA" "$TEMPORARY/raw.output"
grep -Fxq "ref=$BASE_SHA" "$TEMPORARY/raw.output"
[[ "$(git -C "$CLONE" rev-parse --is-shallow-repository)" == true ]]

run_history "$CLONE" origin/main "$TEMPORARY/origin.output"
grep -Fxq "ref=$MAIN_SHA" "$TEMPORARY/origin.output"
git -C "$CLONE" merge-base HEAD "$MAIN_SHA" >/dev/null

run_history "$CLONE" origin/main "$TEMPORARY/merge-base.output" true
grep -Fxq "ref=$BASE_SHA" "$TEMPORARY/merge-base.output"

rm -rf "$CLONE"
git clone --depth 1 --branch feature "file://$INIT_REMOTE" "$CLONE" >/dev/null
run_history "$CLONE" main "$TEMPORARY/branch.output"
grep -Fxq "ref=$MAIN_SHA" "$TEMPORARY/branch.output"

rm -rf "$CLONE"
git clone --depth 1 --branch feature "file://$INIT_REMOTE" "$CLONE" >/dev/null
run_history "$CLONE" base-v1 "$TEMPORARY/tag.output"
grep -Fxq "ref=$BASE_SHA" "$TEMPORARY/tag.output"

if run_history "$CLONE" absent-ref "$TEMPORARY/missing.output" > "$TEMPORARY/missing.log" 2>&1; then
  echo "history recovery accepted a missing ref" >&2
  exit 1
fi
grep -Fq 'Cannot resolve absent-ref' "$TEMPORARY/missing.log"

select_base() {
  local output="$1"
  shift
  : > "$output"
  (cd "$CLONE" && python3 "$SELECT" "$@" --github-output "$output")
}

select_base "$TEMPORARY/explicit.output" --since "$BASE_SHA" --all false
grep -Fxq "ref=$BASE_SHA" "$TEMPORARY/explicit.output"
grep -Fxq 'all=false' "$TEMPORARY/explicit.output"
grep -Fxq 'merge_base=false' "$TEMPORARY/explicit.output"

printf '{"before":"%s"}\n' "$BASE_SHA" > "$TEMPORARY/push.json"
(cd "$CLONE" && GITHUB_EVENT_NAME=push GITHUB_EVENT_PATH="$TEMPORARY/push.json" \
  python3 "$SELECT" --all false --github-output "$TEMPORARY/push.output")
grep -Fxq "ref=$BASE_SHA" "$TEMPORARY/push.output"
grep -Fxq 'merge_base=false' "$TEMPORARY/push.output"

(cd "$CLONE" && GITHUB_EVENT_NAME=pull_request GITHUB_EVENT_PATH='' GITHUB_BASE_REF=main \
  python3 "$SELECT" --all false --github-output "$TEMPORARY/pull-request.output")
grep -Fxq 'ref=origin/main' "$TEMPORARY/pull-request.output"
grep -Fxq 'merge_base=true' "$TEMPORARY/pull-request.output"

printf '{"before":"%040d"}\n' 0 > "$TEMPORARY/zero.json"
(cd "$CLONE" && GITHUB_EVENT_NAME=push GITHUB_EVENT_PATH="$TEMPORARY/zero.json" \
  python3 "$SELECT" --all false --github-output "$TEMPORARY/zero.output")
grep -Fxq 'ref=' "$TEMPORARY/zero.output"
grep -Fxq 'all=true' "$TEMPORARY/zero.output"

select_base "$TEMPORARY/all.output" --since 'ignored' --all true
grep -Fxq 'all=true' "$TEMPORARY/all.output"

select_base "$TEMPORARY/single-zero.output" --since '0' --all false
grep -Fxq 'ref=0' "$TEMPORARY/single-zero.output"
grep -Fxq 'all=false' "$TEMPORARY/single-zero.output"

if select_base "$TEMPORARY/invalid.output" --since $'bad\nref' --all false > "$TEMPORARY/invalid.log" 2>&1; then
  echo "base selection accepted a multiline ref" >&2
  exit 1
fi
grep -Fq 'since must be one non-empty Git ref' "$TEMPORARY/invalid.log"

echo "history and base-selection tests passed"
