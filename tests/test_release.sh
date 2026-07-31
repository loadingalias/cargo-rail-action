#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT="$ROOT/scripts/release.sh"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

REMOTE="$TMP_DIR/remote.git"
WORK="$TMP_DIR/work"
BIN="$TMP_DIR/bin"
GH_STATE_DIR="$TMP_DIR/gh-state"
GH_LOG="$TMP_DIR/gh.log"
SUMMARY="$TMP_DIR/summary.md"

git init --bare --initial-branch=main "$REMOTE" >/dev/null
git clone "$REMOTE" "$WORK" >/dev/null
git -C "$WORK" config user.name "Test User"
git -C "$WORK" config user.email "test@example.com"
printf 'release fixture\n' > "$WORK/action.txt"
git -C "$WORK" add action.txt
git -C "$WORK" commit -m "initial" >/dev/null
git -C "$WORK" push -u origin main >/dev/null
RELEASE_SHA="$(git -C "$WORK" rev-parse HEAD)"

mkdir -p "$BIN" "$GH_STATE_DIR"
cat > "$BIN/gh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail

printf '%s\n' "$*" >> "$GH_LOG"

if [[ "$1" == "release" && "$2" == "view" ]]; then
  [[ -f "$GH_STATE_DIR/$3" ]]
  exit
fi

if [[ "$1" == "release" && "$2" == "create" ]]; then
  if [[ "${GH_FAIL_CREATE:-false}" == "true" ]]; then
    exit 1
  fi
  touch "$GH_STATE_DIR/$3"
  exit
fi

echo "unexpected gh invocation: $*" >&2
exit 1
SH
chmod +x "$BIN/gh"

run_release() {
  (
    cd "$WORK"
    env \
      PATH="$BIN:$PATH" \
      GH_FAIL_CREATE="${GH_FAIL_CREATE:-false}" \
      GH_LOG="$GH_LOG" \
      GH_STATE_DIR="$GH_STATE_DIR" \
      GITHUB_REF="refs/heads/main" \
      GITHUB_STEP_SUMMARY="$SUMMARY" \
      VERSION="6.1.2" \
      bash "$SCRIPT"
  )
}

HOOKS_DIR="$(git --git-dir="$REMOTE" rev-parse --git-path hooks)"
cat > "$HOOKS_DIR/update" <<'SH'
#!/usr/bin/env bash
if [[ "$1" == "refs/tags/v6" ]]; then
  echo "rejecting floating tag for atomicity test" >&2
  exit 1
fi
SH
chmod +x "$HOOKS_DIR/update"

if run_release >"$TMP_DIR/atomic.out" 2>&1; then
  echo "expected atomic tag push to fail"
  exit 1
fi
if git --git-dir="$REMOTE" rev-parse --verify "refs/tags/v6.1.2" >/dev/null 2>&1; then
  echo "version tag escaped a rejected atomic push"
  exit 1
fi
if git --git-dir="$REMOTE" rev-parse --verify "refs/tags/v6" >/dev/null 2>&1; then
  echo "floating tag escaped a rejected atomic push"
  exit 1
fi

rm "$HOOKS_DIR/update"
if GH_FAIL_CREATE=true run_release >"$TMP_DIR/partial.out" 2>&1; then
  echo "expected GitHub release creation to fail"
  exit 1
fi
[[ "$(git --git-dir="$REMOTE" rev-parse "refs/tags/v6.1.2^{commit}")" == "$RELEASE_SHA" ]]
[[ "$(git --git-dir="$REMOTE" rev-parse "refs/tags/v6^{commit}")" == "$RELEASE_SHA" ]]
MAJOR_TAG_OBJECT="$(git --git-dir="$REMOTE" rev-parse "refs/tags/v6")"

run_release
[[ -f "$GH_STATE_DIR/v6.1.2" ]]
CREATE_CALLS="$(grep -c '^release create v6.1.2' "$GH_LOG")"
[[ "$CREATE_CALLS" == "2" ]]
[[ "$(git --git-dir="$REMOTE" rev-parse "refs/tags/v6")" == "$MAJOR_TAG_OBJECT" ]]

run_release
[[ "$(grep -c '^release create v6.1.2' "$GH_LOG")" == "$CREATE_CALLS" ]]
[[ "$(git --git-dir="$REMOTE" rev-parse "refs/tags/v6")" == "$MAJOR_TAG_OBJECT" ]]

printf 'new head\n' >> "$WORK/action.txt"
git -C "$WORK" add action.txt
git -C "$WORK" commit -m "advance main" >/dev/null
git -C "$WORK" push origin main >/dev/null
if run_release >"$TMP_DIR/drift.out" 2>&1; then
  echo "expected an existing version tag on another commit to fail"
  exit 1
fi
grep -Fq "expected $(git -C "$WORK" rev-parse HEAD)" "$TMP_DIR/drift.out"
[[ "$(git --git-dir="$REMOTE" rev-parse "refs/tags/v6.1.2^{commit}")" == "$RELEASE_SHA" ]]
[[ "$(git --git-dir="$REMOTE" rev-parse "refs/tags/v6^{commit}")" == "$RELEASE_SHA" ]]

echo "release tests passed"
