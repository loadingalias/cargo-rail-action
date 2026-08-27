#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TEMPORARY="$(mktemp -d)"
trap 'rm -rf "$TEMPORARY"' EXIT
mkdir -p "$TEMPORARY/bin"

cat > "$TEMPORARY/bin/gh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail

[[ "$*" == 'api repos/loadingalias/cargo-rail/releases/tags/v0.24.0 --jq .assets[].name' ]]
case "$FAKE_RELEASE" in
  available)
    printf '%s\n' \
      SHA256SUMS \
      cargo-rail-x86_64-pc-windows-msvc.zip \
      cargo-rail-x86_64-unknown-linux-gnu.tar.gz
    ;;
  absent)
    echo 'HTTP 404: Not Found' >&2
    exit 1
    ;;
  incomplete)
    echo SHA256SUMS
    ;;
  error)
    echo 'HTTP 503: Service Unavailable' >&2
    exit 1
    ;;
esac
SH
chmod +x "$TEMPORARY/bin/gh"

run_check() {
  local state="$1" required="$2" output="$3"
  PATH="$TEMPORARY/bin:$PATH" FAKE_RELEASE="$state" \
    python3 "$ROOT/scripts/check_release.py" \
      --release-train "$ROOT/release-train.json" \
      --require "$required" \
      --github-output "$output"
}

run_check available false "$TEMPORARY/available.output"
grep -Fxq 'available=true' "$TEMPORARY/available.output"

run_check absent false "$TEMPORARY/absent.output" 2> "$TEMPORARY/absent.error"
grep -Fxq 'available=false' "$TEMPORARY/absent.output"
grep -Fq 'release is absent' "$TEMPORARY/absent.error"

run_check incomplete false "$TEMPORARY/incomplete.output" 2> "$TEMPORARY/incomplete.error"
grep -Fxq 'available=false' "$TEMPORARY/incomplete.output"
grep -Fq 'cargo-rail-x86_64-pc-windows-msvc.zip' "$TEMPORARY/incomplete.error"

if run_check absent true "$TEMPORARY/required.output" > "$TEMPORARY/required.log" 2>&1; then
  echo "release check accepted an absent required release" >&2
  exit 1
fi
grep -Fq 'must exist before Action release' "$TEMPORARY/required.log"

if run_check error false "$TEMPORARY/error.output" > "$TEMPORARY/error.log" 2>&1; then
  echo "release check hid a GitHub API failure" >&2
  exit 1
fi
grep -Fq 'HTTP 503' "$TEMPORARY/error.log"
test ! -e "$TEMPORARY/error.output"

echo "release-availability tests passed"
