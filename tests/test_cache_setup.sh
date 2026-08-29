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
  local mode="$1" healthy="${2:-true}" state="${3:-installed}" root_portability="${4:-physical}" provider="${5:-aws-s3}"
  printf '{"status":{"schema_version":13,"installation":{"state":"%s","healthy":%s,"max_bytes":10737418240,"root_portability":"%s"},"remote":{"provider":"%s","authority":"%s","mode":"%s","activation":"direct_transport_selected"}}}' \
    "$state" "$healthy" "$root_portability" "$provider" "$authority" "$mode"
}

run_setup() {
  local mode="$1" url="$2" local_dir="${3:-}" name="${4:-run}" root_portability="${5:-physical}" strict_probe="${6:-false}" provider="${7:-aws-s3}"
  : > "$TEMPORARY/$name.log"
  PATH="$TEMPORARY/bin:$PATH" \
    FAKE_CARGO_LOG="$TEMPORARY/$name.log" \
    FAKE_STATUS_JSON="$(status_json "$mode" true installed "$root_portability" "$provider")" \
    FAKE_PROBE_JSON="${FAKE_PROBE_JSON:-}" \
    python3 "$ROOT/scripts/cache_setup.py" \
      --url "$url" \
      --mode "$mode" \
      --max-size 10GiB \
      --local-dir "$local_dir" \
      --root-portability "$root_portability" \
      --strict-probe "$strict_probe" \
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
  ["rail", "cache", "setup", "--remote", sys.argv[2], "--remote-mode", "read", "--max-size", "10GiB", "--root-portability", "physical", "--local-dir", sys.argv[3]],
  ["rail", "cache", "status", "--scope", "local", "-f", "json"],
], calls
PY
grep -Fxq 'healthy=true' "$TEMPORARY/read.output"
grep -Fxq 'provider=aws-s3' "$TEMPORARY/read.output"
grep -Fxq 'mode=read' "$TEMPORARY/read.output"
grep -Fxq 'max_bytes=10737418240' "$TEMPORARY/read.output"
grep -Fxq 'root_portability=physical' "$TEMPORARY/read.output"
python3 - "$TEMPORARY/read.output" <<'PY'
import json
import pathlib
import sys

line = next(line for line in pathlib.Path(sys.argv[1]).read_text().splitlines() if line.startswith("status_json="))
value = json.loads(line.removeprefix("status_json="))
assert value["cache_action_status_version"] == 1
assert value["remote"]["mode"] == "read"
assert value["installation"]["healthy"] is True
assert value["installation"]["root_portability"] == "physical"
PY
if grep -Fq "$sensitive_url" "$TEMPORARY/read.output" || grep -Fq "$sensitive_url" "$TEMPORARY/read.summary"; then
  echo "cache output leaked its input URL" >&2
  exit 1
fi
if grep -Fq "$sensitive_dir" "$TEMPORARY/read.output" || grep -Fq "$sensitive_dir" "$TEMPORARY/read.summary"; then
  echo "cache output leaked its local path" >&2
  exit 1
fi
grep -Fq 'Status inspection was local; strict remote probing was not requested.' "$TEMPORARY/read.summary"

AWS_ACCESS_KEY_ID=fixture-access \
AWS_SECRET_ACCESS_KEY=fixture-secret \
AWS_SESSION_TOKEN=fixture-session \
FAKE_CARGO_ENV_LOG="$TEMPORARY/write.env" \
  run_setup read-write 'r2://0123456789abcdef0123456789abcdef/bucket/prefix' '' write physical false cloudflare-r2
python3 - "$TEMPORARY/write.log" <<'PY'
import pathlib
import sys

calls = [[arg.decode() for arg in call.split(b"\0") if arg] for call in pathlib.Path(sys.argv[1]).read_bytes().split(b"\0\0") if call]
assert calls[0][-6:] == ["--remote-mode", "read-write", "--max-size", "10GiB", "--root-portability", "physical"]
assert "--local-dir" not in calls[0]
PY
grep -Fxq 'provider=cloudflare-r2' "$TEMPORARY/write.output"
python3 - "$TEMPORARY/write.env" <<'PY'
import pathlib
import sys

values = [value.decode() for value in pathlib.Path(sys.argv[1]).read_bytes().split(b"\0") if value]
assert values == ["fixture-access", "fixture-secret", "fixture-session"] * 2, values
PY
if grep -Eq 'fixture-(access|secret|session)' "$TEMPORARY/write.output" "$TEMPORARY/write.summary"; then
  echo "cache output leaked an R2 credential" >&2
  exit 1
fi

probe_json="$(printf '{"schema_version":1,"command":"cache","mode":"probe","result":"ready","exit_code":0,"ready":true,"remote":{"provider":"aws-s3","authority":"%s","mode":"read","activation":"direct_transport_selected"},"protocol_marker":"existing"}' "$authority")"
FAKE_PROBE_JSON="$probe_json" run_setup read 's3://cache-bucket/team?owner=123456789012' '' strict remap true
python3 - "$TEMPORARY/strict.log" <<'PY'
import pathlib
import sys

calls = [[arg.decode() for arg in call.split(b"\0") if arg] for call in pathlib.Path(sys.argv[1]).read_bytes().split(b"\0\0") if call]
assert calls == [
  ["rail", "cache", "probe", "--help"],
  ["rail", "cache", "setup", "--remote", "s3://cache-bucket/team?owner=123456789012", "--remote-mode", "read", "--max-size", "10GiB", "--root-portability", "remap"],
  ["rail", "cache", "status", "--scope", "local", "-f", "json"],
  ["rail", "cache", "probe", "-f", "json"],
], calls
PY
grep -Fxq 'root_portability=remap' "$TEMPORARY/strict.output"
grep -Fxq 'remote_ready=true' "$TEMPORARY/strict.output"
grep -Fxq 'protocol_marker=existing' "$TEMPORARY/strict.output"
grep -Fq 'cache_action_probe_version' "$TEMPORARY/strict.output"
grep -Fq 'The strict probe authenticated to the selected provider' "$TEMPORARY/strict.summary"
python3 - "$TEMPORARY/strict.output" <<'PY'
import json
import pathlib
import sys

line = next(line for line in pathlib.Path(sys.argv[1]).read_text().splitlines() if line.startswith("status_json="))
value = json.loads(line.removeprefix("status_json="))
assert value["installation"]["root_portability"] == "remap"
PY

: > "$TEMPORARY/unsupported-probe.log"
if PATH="$TEMPORARY/bin:$PATH" FAKE_CARGO_LOG="$TEMPORARY/unsupported-probe.log" \
  FAKE_PROBE_HELP_EXIT=2 FAKE_STATUS_JSON='{}' FAKE_PROBE_JSON='{}' \
  python3 "$ROOT/scripts/cache_setup.py" --url s3://bucket --mode read --max-size 1GiB \
    --root-portability physical --strict-probe true \
    --install-method cached --install-version 0.24.0 \
    --github-output "$TEMPORARY/unsupported-probe.output" --summary "$TEMPORARY/unsupported-probe.summary" \
    > "$TEMPORARY/unsupported-probe.stdout" 2> "$TEMPORARY/unsupported-probe.stderr"; then
  echo "cache setup accepted strict probing from an incompatible Cargo-Rail" >&2
  exit 1
fi
python3 - "$TEMPORARY/unsupported-probe.log" <<'PY'
import pathlib
import sys

calls = [[arg.decode() for arg in call.split(b"\0") if arg] for call in pathlib.Path(sys.argv[1]).read_bytes().split(b"\0\0") if call]
assert calls == [["rail", "cache", "probe", "--help"]], calls
PY
grep -Fq 'installed Cargo-Rail 0.24.0 does not support strict cache probing' "$TEMPORARY/unsupported-probe.stderr"
[[ ! -s "$TEMPORARY/unsupported-probe.output" ]]

for root_portability in '' portable REMAP; do
  : > "$TEMPORARY/invalid-root.log"
  if PATH="$TEMPORARY/bin:$PATH" FAKE_CARGO_LOG="$TEMPORARY/invalid-root.log" FAKE_STATUS_JSON='{}' \
    python3 "$ROOT/scripts/cache_setup.py" --url s3://bucket --mode read --max-size 1GiB \
      --root-portability "$root_portability" --strict-probe false \
      --install-method cached --install-version 0.24.0 \
      --github-output "$TEMPORARY/invalid-root.output" --summary "$TEMPORARY/invalid-root.summary" \
      > "$TEMPORARY/invalid-root.stdout" 2> "$TEMPORARY/invalid-root.stderr"; then
    echo "cache setup accepted invalid root portability '$root_portability'" >&2
    exit 1
  fi
  [[ ! -s "$TEMPORARY/invalid-root.log" ]]
  grep -Fq 'root-portability must be explicitly set to physical or remap' "$TEMPORARY/invalid-root.stderr"
done

for strict_probe in '' TRUE 1; do
  : > "$TEMPORARY/invalid-probe.log"
  if PATH="$TEMPORARY/bin:$PATH" FAKE_CARGO_LOG="$TEMPORARY/invalid-probe.log" FAKE_STATUS_JSON='{}' \
    python3 "$ROOT/scripts/cache_setup.py" --url s3://bucket --mode read --max-size 1GiB \
      --root-portability physical --strict-probe "$strict_probe" \
      --install-method cached --install-version 0.24.0 \
      --github-output "$TEMPORARY/invalid-probe.output" --summary "$TEMPORARY/invalid-probe.summary" \
      > "$TEMPORARY/invalid-probe.stdout" 2> "$TEMPORARY/invalid-probe.stderr"; then
    echo "cache setup accepted invalid strict probe '$strict_probe'" >&2
    exit 1
  fi
  [[ ! -s "$TEMPORARY/invalid-probe.log" ]]
  grep -Fq 'strict-probe must be true or false' "$TEMPORARY/invalid-probe.stderr"
done

: > "$TEMPORARY/probe-failure.log"
if PATH="$TEMPORARY/bin:$PATH" FAKE_CARGO_LOG="$TEMPORARY/probe-failure.log" \
  FAKE_STATUS_JSON="$(status_json read)" FAKE_PROBE_JSON='{}' FAKE_PROBE_EXIT=7 \
  python3 "$ROOT/scripts/cache_setup.py" --url s3://bucket --mode read --max-size 1GiB \
    --root-portability physical --strict-probe true \
    --install-method cached --install-version 0.24.0 \
    --github-output "$TEMPORARY/probe-failure.output" --summary "$TEMPORARY/probe-failure.summary" \
    > "$TEMPORARY/probe-failure.stdout" 2> "$TEMPORARY/probe-failure.stderr"; then
  echo "cache setup hid a strict probe failure" >&2
  exit 1
fi
grep -Fq 'cache probe failed with exit code 7' "$TEMPORARY/probe-failure.stderr"
[[ ! -s "$TEMPORARY/probe-failure.output" ]]

: > "$TEMPORARY/probe-mismatch.log"
mismatched_probe_json="${probe_json/$authority/remote-authority-v1-sha256-$(printf 'b%.0s' {1..64})}"
if PATH="$TEMPORARY/bin:$PATH" FAKE_CARGO_LOG="$TEMPORARY/probe-mismatch.log" \
  FAKE_STATUS_JSON="$(status_json read)" FAKE_PROBE_JSON="$mismatched_probe_json" \
  python3 "$ROOT/scripts/cache_setup.py" --url s3://bucket --mode read --max-size 1GiB \
    --root-portability physical --strict-probe true \
    --install-method cached --install-version 0.24.0 \
    --github-output "$TEMPORARY/probe-mismatch.output" --summary "$TEMPORARY/probe-mismatch.summary" \
    > "$TEMPORARY/probe-mismatch.stdout" 2> "$TEMPORARY/probe-mismatch.stderr"; then
  echo "cache setup accepted a probe for another authority" >&2
  exit 1
fi
grep -Fq 'cache probe remote.authority disagrees with configured status' "$TEMPORARY/probe-mismatch.stderr"
[[ ! -s "$TEMPORARY/probe-mismatch.output" ]]

for invalid in '' write READ; do
  : > "$TEMPORARY/invalid.log"
  if PATH="$TEMPORARY/bin:$PATH" FAKE_CARGO_LOG="$TEMPORARY/invalid.log" FAKE_STATUS_JSON='{}' \
    python3 "$ROOT/scripts/cache_setup.py" --url s3://bucket --mode "$invalid" --max-size 1GiB \
      --root-portability physical --strict-probe false \
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
    --root-portability physical --strict-probe false \
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
    --root-portability physical --strict-probe false \
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
    --root-portability physical --strict-probe false \
    --install-method cached --install-version 0.24.0 \
    --github-output "$TEMPORARY/provider.output" --summary "$TEMPORARY/provider.summary" \
    > "$TEMPORARY/provider.stdout" 2> "$TEMPORARY/provider.stderr"; then
  echo "cache setup accepted an unknown provider" >&2
  exit 1
fi
grep -Fq 'cache status.remote.provider is unsupported' "$TEMPORARY/provider.stderr"
[[ ! -s "$TEMPORARY/provider.output" ]]

echo "cache setup tests passed"
