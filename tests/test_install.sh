#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TEMPORARY="$(mktemp -d)"
trap 'rm -rf "$TEMPORARY"' EXIT
VERSION=9.8.7
TARGET=x86_64-unknown-linux-gnu
RELEASE="$TEMPORARY/release"
PAYLOAD_ROOT="$TEMPORARY/payload"
PAYLOAD="$PAYLOAD_ROOT/cargo-rail-$TARGET"
mkdir -p "$RELEASE" "$PAYLOAD" "$TEMPORARY/bin" "$TEMPORARY/home/.cargo/bin"

names=(
  cargo-rail
  cargo-rail-compiler-observation
  cargo-rail-distributed-worker
  cargo-rail-fact-driver
  cargo-rail-fact-driver-source-v1.json
  cargo-rail-native-rustc-worker
  cargo-rail-native-rustc-wrapper
)
for name in "${names[@]}"; do
  if [[ "$name" == *.json ]]; then
    printf '{"version":1,"files":[]}' > "$PAYLOAD/$name"
  else
    cp "$ROOT/tests/fixtures/fake-cargo-rail.sh" "$PAYLOAD/$name"
    chmod +x "$PAYLOAD/$name"
  fi
done

capability () {
  case "$1" in
    cargo-rail) echo core ;;
    cargo-rail-compiler-observation) echo analysis ;;
    cargo-rail-distributed-worker) echo distributed ;;
    cargo-rail-fact-driver) echo surface ;;
    cargo-rail-fact-driver-source-v1.json) echo surface-source ;;
    *) echo cache ;;
  esac
}
{
  printf 'cargo-rail-components-v1\t%s\t%s\n' "$VERSION" "$TARGET"
  for name in "${names[@]}"; do
    printf '%s\t%s\t%s\t%s\n' "$name" "$(shasum -a 256 "$PAYLOAD/$name" | awk '{print $1}')" \
      "$(wc -c < "$PAYLOAD/$name" | tr -d ' ')" "$(capability "$name")"
  done
} > "$PAYLOAD/cargo-rail-components-v1.tsv"

ARCHIVE="cargo-rail-$TARGET.tar.gz"
tar -czf "$RELEASE/$ARCHIVE" -C "$PAYLOAD_ROOT" "cargo-rail-$TARGET"
printf '%s  %s\n' "$(shasum -a 256 "$RELEASE/$ARCHIVE" | awk '{print $1}')" "$ARCHIVE" > "$RELEASE/SHA256SUMS"
cp "$ROOT/tests/fixtures/fake-curl.sh" "$TEMPORARY/bin/curl"
chmod +x "$TEMPORARY/bin/curl"

run_installer () {
  local requested_set="$1" requested_home
  requested_home="${2:-$TEMPORARY/home-$requested_set}"
  mkdir -p "$requested_home/.cargo/bin"
  RUNNER_OS=Linux RUNNER_ARCH=X64 REQUESTED_VERSION="$VERSION" \
    COMPONENT_SET="$requested_set" ACTION_INSTALL_RELEASE="$RELEASE" HOME="$requested_home" \
    GITHUB_OUTPUT="$TEMPORARY/output" GITHUB_PATH="$TEMPORARY/path" \
    PATH="$requested_home/.cargo/bin:$TEMPORARY/bin:$PATH" \
    bash "$ROOT/scripts/install.sh"
}

expected_names () {
  printf '%s\n' cargo-rail
  case "$1" in
    cache) printf '%s\n' cargo-rail-native-rustc-wrapper cargo-rail-native-rustc-worker ;;
    surface) printf '%s\n' cargo-rail-compiler-observation cargo-rail-fact-driver cargo-rail-fact-driver-source-v1.json ;;
    distributed) printf '%s\n' cargo-rail-distributed-worker ;;
    complete) printf '%s\n' cargo-rail-compiler-observation cargo-rail-distributed-worker cargo-rail-fact-driver \
      cargo-rail-fact-driver-source-v1.json cargo-rail-native-rustc-wrapper cargo-rail-native-rustc-worker ;;
  esac
}

for set in core cache surface distributed complete; do
  run_installer "$set"
  home="$TEMPORARY/home-$set"
  receipt="$home/.cargo/bin/cargo-rail-components-$set-v1.tsv"
  test -f "$receipt"
  diff -u <(expected_names "$set" | sort) <(sed '1d' "$receipt" | cut -f1 | sort)
  while IFS= read -r name; do
    test -f "$home/.cargo/bin/$name"
  done < <(expected_names "$set")
done

: > "$TEMPORARY/output"
mv "$RELEASE" "$TEMPORARY/release-unavailable"
run_installer cache "$TEMPORARY/home-complete"
grep -Fxq 'method=cached' "$TEMPORARY/output"
mv "$TEMPORARY/release-unavailable" "$RELEASE"

: > "$TEMPORARY/output"
mv "$RELEASE" "$TEMPORARY/release-unavailable"
run_installer core "$TEMPORARY/home-cache"
grep -Fxq 'method=cached' "$TEMPORARY/output"
mv "$TEMPORARY/release-unavailable" "$RELEASE"

printf damaged > "$TEMPORARY/home-surface/.cargo/bin/cargo-rail-fact-driver"
run_installer surface
cmp "$ROOT/tests/fixtures/fake-cargo-rail.sh" "$TEMPORARY/home-surface/.cargo/bin/cargo-rail-fact-driver"

mv "$PAYLOAD/cargo-rail-fact-driver" "$PAYLOAD/cargo-rail-fact-driver.missing"
tar -czf "$RELEASE/$ARCHIVE" -C "$PAYLOAD_ROOT" "cargo-rail-$TARGET"
printf '%s  %s\n' "$(shasum -a 256 "$RELEASE/$ARCHIVE" | awk '{print $1}')" "$ARCHIVE" > "$RELEASE/SHA256SUMS"
if run_installer surface "$TEMPORARY/home-missing-surface" > "$TEMPORARY/missing.out" 2>&1; then
  echo "action installer accepted a missing Surface component" >&2
  exit 1
fi
grep -Fq "cargo-rail-fact-driver is missing from" "$TEMPORARY/missing.out"
mv "$PAYLOAD/cargo-rail-fact-driver.missing" "$PAYLOAD/cargo-rail-fact-driver"

printf tampered > "$PAYLOAD/cargo-rail-native-rustc-worker"
tar -czf "$RELEASE/$ARCHIVE" -C "$PAYLOAD_ROOT" "cargo-rail-$TARGET"
printf '%s  %s\n' "$(shasum -a 256 "$RELEASE/$ARCHIVE" | awk '{print $1}')" "$ARCHIVE" > "$RELEASE/SHA256SUMS"
if run_installer cache "$TEMPORARY/home-tampered-cache" > "$TEMPORARY/tamper.out" 2>&1; then
  echo "action installer accepted a component that disagreed with the authenticated inventory" >&2
  exit 1
fi
grep -Fq "Component authority does not match cargo-rail-native-rustc-worker" "$TEMPORARY/tamper.out"

echo "installer component-set tests passed"
