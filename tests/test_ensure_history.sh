#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT="$ROOT/scripts/ensure_history.sh"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

init_remote_repo() {
  local name="$1"
  INIT_REMOTE="$TMP_DIR/$name-remote.git"
  INIT_WORK="$TMP_DIR/$name-work"

  git init --bare --initial-branch=main "$INIT_REMOTE" >/dev/null
  git clone "$INIT_REMOTE" "$INIT_WORK" >/dev/null
  git -C "$INIT_WORK" config user.email "test@example.com"
  git -C "$INIT_WORK" config user.name "Test"
  git -C "$INIT_WORK" branch -M main
}

run_history_check() {
  local repo="$1"
  local base_ref="$2"
  local output_file="$3"

  (
    cd "$repo"
    BASE_REF="$base_ref" GITHUB_OUTPUT="$output_file" bash "$SCRIPT"
  )
}

test_raw_sha_fetch_stays_shallow() {
  init_remote_repo raw-sha
  local remote="$INIT_REMOTE"
  local work="$INIT_WORK"
  local clone="$TMP_DIR/raw-sha-clone"
  local output_file="$TMP_DIR/raw-sha-output.txt"
  local base_sha

  printf 'one\n' > "$work/file.txt"
  git -C "$work" add file.txt
  git -C "$work" commit -m "one" >/dev/null
  base_sha="$(git -C "$work" rev-parse HEAD)"

  printf 'two\n' > "$work/file.txt"
  git -C "$work" add file.txt
  git -C "$work" commit -m "two" >/dev/null
  git -C "$work" push -u origin main >/dev/null

  git clone --depth 1 --branch main "file://$remote" "$clone" >/dev/null

  if git -C "$clone" rev-parse --verify "$base_sha^{commit}" >/dev/null 2>&1; then
    echo "raw SHA unexpectedly present before targeted fetch"
    exit 1
  fi

  run_history_check "$clone" "$base_sha" "$output_file"

  git -C "$clone" rev-parse --verify "$base_sha^{commit}" >/dev/null
  [[ -f "$(git -C "$clone" rev-parse --git-path shallow)" ]]
  grep -qx 'shallow=true' "$output_file"
}

test_branch_ref_falls_back_to_full_history() {
  init_remote_repo branch-ref
  local remote="$INIT_REMOTE"
  local work="$INIT_WORK"
  local clone="$TMP_DIR/branch-ref-clone"
  local output_file="$TMP_DIR/branch-ref-output.txt"

  printf 'base\n' > "$work/file.txt"
  git -C "$work" add file.txt
  git -C "$work" commit -m "base" >/dev/null
  git -C "$work" push -u origin main >/dev/null

  git -C "$work" checkout -b feature >/dev/null
  printf 'feature\n' > "$work/feature.txt"
  git -C "$work" add feature.txt
  git -C "$work" commit -m "feature" >/dev/null
  git -C "$work" push -u origin feature >/dev/null

  git -C "$work" checkout main >/dev/null
  printf 'main\n' > "$work/main.txt"
  git -C "$work" add main.txt
  git -C "$work" commit -m "main advance" >/dev/null
  git -C "$work" push >/dev/null

  git clone --depth 1 --branch feature "file://$remote" "$clone" >/dev/null

  if git -C "$clone" rev-parse --verify "origin/main^{commit}" >/dev/null 2>&1; then
    echo "origin/main unexpectedly present before branch fetch"
    exit 1
  fi

  run_history_check "$clone" "origin/main" "$output_file"

  git -C "$clone" rev-parse --verify "origin/main^{commit}" >/dev/null
  git -C "$clone" merge-base HEAD origin/main >/dev/null
  [[ ! -f "$(git -C "$clone" rev-parse --git-path shallow)" ]]
  grep -qx 'shallow=true' "$output_file"
}

test_raw_sha_fetch_stays_shallow
test_branch_ref_falls_back_to_full_history

echo "ensure_history tests passed"
