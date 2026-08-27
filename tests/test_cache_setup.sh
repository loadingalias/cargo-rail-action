#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TEMPORARY="$(mktemp -d)"
trap 'rm -rf "$TEMPORARY"' EXIT
mkdir -p "$TEMPORARY/bin"
cp "$ROOT/tests/fixtures/fake-cache-cargo.sh" "$TEMPORARY/bin/cargo"
chmod +x "$TEMPORARY/bin/cargo"

authority="remote-authority-v1-sha256-$(printf 'a%.0s' {1..64})"
status_json() {
  local mode="$1" healthy="${2:-true}" state="${3:-installed}"
  printf '{"status":{"schema_version":13,"installation":{"state":"%s","healthy":%s,"max_bytes":10737418240},"remote":{"provider":"aws-s3","authority":"%s","mode":"%s","activation":"direct_transport_selected"}}}' \
    "$state" "$healthy" "$authority" "$mode"
}

run_setup() {
  local mode="$1" url="$2" local_dir="${3:-}" name="${4:-run}"
  : > "$TEMPORARY/$name.log"
  PATH="$TEMPORARY/bin:$PATH" \
    FAKE_CARGO_LOG="$TEMPORARY/$name.log" \
    FAKE_STATUS_JSON="$(status_json "$mode")" \
    python3 "$ROOT/scripts/cache_setup.py" \
      --url "$url" \
      --mode "$mode" \
      --max-size 10GiB \
      --local-dir "$local_dir" \
      --install-method cached \
      --install-version 0.24.0 \
      --github-output "$TEMPORARY/$name.output" \
      --summary "$TEMPORARY/$name.summary"
}

sensitive_url='s3://cache-bucket/team path?owner=123456789012&literal=;touch injected'
sensitive_dir="$TEMPORARY/local cache;touch injected"
run_setup read "$sensitive_url" "$sensitive_dir" read

python3 - "$TEMPORARY/read.log" "$sensitive_url" "$sensitive_dir" <<'PY'
import pathlib
import sys

raw = pathlib.Path(sys.argv[1]).read_bytes()
calls = [[arg.decode() for arg in call.split(b"\0") if arg] for call in raw.split(b"\0\0") if call]
assert calls == [
  ["rail", "cache", "setup", "--remote", sys.argv[2], "--remote-mode", "read", "--max-size", "10GiB", "--local-dir", sys.argv[3]],
  ["rail", "cache", "status", "--scope", "local", "-f", "json"],
], calls
PY
grep -Fxq 'healthy=true' "$TEMPORARY/read.output"
grep -Fxq 'provider=aws-s3' "$TEMPORARY/read.output"
grep -Fxq 'mode=read' "$TEMPORARY/read.output"
grep -Fxq 'max_bytes=10737418240' "$TEMPORARY/read.output"
python3 - "$TEMPORARY/read.output" <<'PY'
import json
import pathlib
import sys

line = next(line for line in pathlib.Path(sys.argv[1]).read_text().splitlines() if line.startswith("status_json="))
value = json.loads(line.removeprefix("status_json="))
assert value["cache_action_status_version"] == 1
assert value["remote"]["mode"] == "read"
assert value["installation"]["healthy"] is True
PY
if grep -Fq "$sensitive_url" "$TEMPORARY/read.output" || grep -Fq "$sensitive_url" "$TEMPORARY/read.summary"; then
  echo "cache output leaked its input URL" >&2
  exit 1
fi
if grep -Fq "$sensitive_dir" "$TEMPORARY/read.output" || grep -Fq "$sensitive_dir" "$TEMPORARY/read.summary"; then
  echo "cache output leaked its local path" >&2
  exit 1
fi
grep -Fq 'Status inspection was local and did not contact the remote provider.' "$TEMPORARY/read.summary"

run_setup read-write 'r2://account/bucket/prefix' '' write
python3 - "$TEMPORARY/write.log" <<'PY'
import pathlib
import sys

calls = [[arg.decode() for arg in call.split(b"\0") if arg] for call in pathlib.Path(sys.argv[1]).read_bytes().split(b"\0\0") if call]
assert calls[0][-4:] == ["--remote-mode", "read-write", "--max-size", "10GiB"]
assert "--local-dir" not in calls[0]
PY

for invalid in '' write READ; do
  : > "$TEMPORARY/invalid.log"
  if PATH="$TEMPORARY/bin:$PATH" FAKE_CARGO_LOG="$TEMPORARY/invalid.log" FAKE_STATUS_JSON='{}' \
    python3 "$ROOT/scripts/cache_setup.py" --url s3://bucket --mode "$invalid" --max-size 1GiB \
      --install-method cached --install-version 0.24.0 \
      --github-output "$TEMPORARY/invalid.output" --summary "$TEMPORARY/invalid.summary" \
      > "$TEMPORARY/invalid.stdout" 2> "$TEMPORARY/invalid.stderr"; then
    echo "cache setup accepted invalid mode '$invalid'" >&2
    exit 1
  fi
  [[ ! -s "$TEMPORARY/invalid.log" ]]
  grep -Fq 'mode must be explicitly set to read or read-write' "$TEMPORARY/invalid.stderr"
done

: > "$TEMPORARY/failure.log"
if PATH="$TEMPORARY/bin:$PATH" FAKE_CARGO_LOG="$TEMPORARY/failure.log" FAKE_SETUP_EXIT=9 FAKE_STATUS_JSON='{}' \
  python3 "$ROOT/scripts/cache_setup.py" --url s3://bucket --mode read --max-size 1GiB \
    --install-method cached --install-version 0.24.0 \
    --github-output "$TEMPORARY/failure.output" --summary "$TEMPORARY/failure.summary" \
    > "$TEMPORARY/failure.stdout" 2> "$TEMPORARY/failure.stderr"; then
  echo "cache setup hid a setup failure" >&2
  exit 1
fi
grep -Fq 'cache setup failed with exit code 9' "$TEMPORARY/failure.stderr"
python3 - "$TEMPORARY/failure.log" <<'PY'
import pathlib
import sys
assert pathlib.Path(sys.argv[1]).read_bytes().count(b"\0\0") == 1
PY

: > "$TEMPORARY/unhealthy.log"
if PATH="$TEMPORARY/bin:$PATH" FAKE_CARGO_LOG="$TEMPORARY/unhealthy.log" FAKE_STATUS_JSON="$(status_json read false drifted)" \
  python3 "$ROOT/scripts/cache_setup.py" --url s3://bucket --mode read --max-size 1GiB \
    --install-method cached --install-version 0.24.0 \
    --github-output "$TEMPORARY/unhealthy.output" --summary "$TEMPORARY/unhealthy.summary" \
    > "$TEMPORARY/unhealthy.stdout" 2> "$TEMPORARY/unhealthy.stderr"; then
  echo "cache setup accepted unhealthy post-setup status" >&2
  exit 1
fi
grep -Fq 'cache installation is unhealthy after setup' "$TEMPORARY/unhealthy.stderr"
[[ ! -s "$TEMPORARY/unhealthy.output" ]]

: > "$TEMPORARY/provider.log"
invalid_provider="$(status_json read | sed 's/\"provider\":\"aws-s3\"/\"provider\":\"unknown\"/')"
if PATH="$TEMPORARY/bin:$PATH" FAKE_CARGO_LOG="$TEMPORARY/provider.log" FAKE_STATUS_JSON="$invalid_provider" \
  python3 "$ROOT/scripts/cache_setup.py" --url s3://bucket --mode read --max-size 1GiB \
    --install-method cached --install-version 0.24.0 \
    --github-output "$TEMPORARY/provider.output" --summary "$TEMPORARY/provider.summary" \
    > "$TEMPORARY/provider.stdout" 2> "$TEMPORARY/provider.stderr"; then
  echo "cache setup accepted an unknown provider" >&2
  exit 1
fi
grep -Fq 'cache status.remote.provider is unsupported' "$TEMPORARY/provider.stderr"
[[ ! -s "$TEMPORARY/provider.output" ]]

echo "cache setup tests passed"
