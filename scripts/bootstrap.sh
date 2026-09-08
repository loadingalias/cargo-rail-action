#!/usr/bin/env bash
set -euo pipefail

RUNTIME_VERSION="9.0.0"
RUNTIME_RELEASE="v9.0.0"
RUNTIME_MANIFEST="cargo-rail-action-runtime-v1.tsv"
MAX_MANIFEST_BYTES=65536
MAX_RUNTIME_BYTES=33554432

fail() {
  local message="${1//%/%25}"
  message="${message//$'\r'/%0D}"
  message="${message//$'\n'/%0A}"
  printf '::error::%s\n' "$message" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "Cargo-Rail Action v9 requires $1"
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

canonical_file() {
  local directory name
  directory="$(dirname -- "$1")"
  name="$(basename -- "$1")"
  printf '%s/%s\n' "$(cd -- "$directory" && pwd -P)" "$name"
}

contained_by() {
  case "$1" in
    "$2"|"$2"/*) return 0 ;;
    *) return 1 ;;
  esac
}

require_command git
require_command curl
if command -v sha256sum >/dev/null 2>&1; then
  :
elif command -v shasum >/dev/null 2>&1; then
  :
else
  fail "Cargo-Rail Action v9 requires sha256sum or shasum"
fi

case "${RUNNER_OS:-}-${RUNNER_ARCH:-}" in
  Linux-X64)
    TARGET="x86_64-unknown-linux-gnu"
    if command -v getconf >/dev/null 2>&1; then
      GLIBC_VERSION="$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}')"
    else
      GLIBC_VERSION="$(ldd --version 2>&1 | head -n 1 | grep -Eo '[0-9]+\.[0-9]+' | tail -n 1)"
    fi
    [[ "$GLIBC_VERSION" =~ ^[0-9]+\.[0-9]+$ ]] || fail "Cargo-Rail Action v9 requires GNU libc 2.39 or newer"
    GLIBC_MAJOR="${GLIBC_VERSION%%.*}"
    GLIBC_MINOR="${GLIBC_VERSION#*.}"
    if (( GLIBC_MAJOR < 2 || (GLIBC_MAJOR == 2 && GLIBC_MINOR < 39) )); then
      fail "Cargo-Rail Action v9 requires GNU libc 2.39 or newer; found $GLIBC_VERSION"
    fi
    ;;
  macOS-ARM64) TARGET="aarch64-apple-darwin" ;;
  Windows-X64) TARGET="x86_64-pc-windows-msvc" ;;
  *) fail "Cargo-Rail Action v9 does not support ${RUNNER_OS:-unknown}/${RUNNER_ARCH:-unknown}" ;;
esac

LOCAL_RUNTIME=""
LOCAL_DIGEST=""
while (( $# > 0 )); do
  case "$1" in
    --local-runtime)
      (( $# >= 2 )) || fail "--local-runtime requires an absolute path"
      LOCAL_RUNTIME="$2"
      shift 2
      ;;
    --local-runtime-sha256)
      (( $# >= 2 )) || fail "--local-runtime-sha256 requires a digest"
      LOCAL_DIGEST="$2"
      shift 2
      ;;
    *) break ;;
  esac
done
(( $# > 0 )) || fail "the Rust runtime operation is required"

RUNNER_TEMP_PATH="${RUNNER_TEMP:-}"
[[ -n "$RUNNER_TEMP_PATH" && -d "$RUNNER_TEMP_PATH" ]] || fail "RUNNER_TEMP must name an existing directory"
RUNNER_TEMP_PATH="$(cd -- "$RUNNER_TEMP_PATH" && pwd -P)"
BOOTSTRAP_TEMP="$(mktemp -d "$RUNNER_TEMP_PATH/cargo-rail-action-bootstrap.XXXXXX")"
chmod 700 "$BOOTSTRAP_TEMP"
PUBLICATION_LOCK=""
cleanup() {
  rm -rf -- "$BOOTSTRAP_TEMP"
  if [[ -n "$PUBLICATION_LOCK" ]]; then
    rmdir -- "$PUBLICATION_LOCK" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if [[ -n "$LOCAL_RUNTIME" || -n "$LOCAL_DIGEST" ]]; then
  [[ -n "$LOCAL_RUNTIME" && -n "$LOCAL_DIGEST" ]] || fail "local runtime path and SHA-256 must be supplied together"
  [[ "$LOCAL_RUNTIME" == /* && "$LOCAL_DIGEST" =~ ^[0-9a-f]{64}$ ]] || fail "local runtime authority is malformed"
  [[ -f "$LOCAL_RUNTIME" && ! -L "$LOCAL_RUNTIME" ]] || fail "local runtime must be a regular non-symbolic file"
  LOCAL_RUNTIME="$(canonical_file "$LOCAL_RUNTIME")"
  ACTION_ROOT="${GITHUB_ACTION_PATH:-}"
  [[ -n "$ACTION_ROOT" && -d "$ACTION_ROOT" ]] || fail "GITHUB_ACTION_PATH must name the Action repository"
  ACTION_ROOT="$(cd -- "$ACTION_ROOT" && pwd -P)"
  if ! contained_by "$LOCAL_RUNTIME" "$ACTION_ROOT" && ! contained_by "$LOCAL_RUNTIME" "$RUNNER_TEMP_PATH"; then
    fail "local runtime must remain inside GITHUB_ACTION_PATH or RUNNER_TEMP"
  fi
  RUNTIME_BYTES="$(wc -c < "$LOCAL_RUNTIME" | tr -d ' ')"
  (( RUNTIME_BYTES > 0 && RUNTIME_BYTES <= MAX_RUNTIME_BYTES )) || fail "local runtime exceeds the 32 MiB bound"
  [[ "$(sha256_file "$LOCAL_RUNTIME")" == "$LOCAL_DIGEST" ]] || fail "local runtime digest does not match"
  RUNTIME_SUFFIX=""
  [[ "$TARGET" == *windows-msvc ]] && RUNTIME_SUFFIX=".exe"
  RUNTIME_ASSET="cargo-rail-action-$TARGET$RUNTIME_SUFFIX"
  RUNTIME_SOURCE="$LOCAL_RUNTIME"
  RUNTIME_DIGEST="$LOCAL_DIGEST"
else
  MANIFEST_PATH="$BOOTSTRAP_TEMP/$RUNTIME_MANIFEST"
  RELEASE_ROOT="https://github.com/loadingalias/cargo-rail-action/releases/download/$RUNTIME_RELEASE"
  curl --fail --silent --show-error --location --retry 3 --proto '=https' --tlsv1.2 \
    --max-filesize "$MAX_MANIFEST_BYTES" --output "$MANIFEST_PATH" "$RELEASE_ROOT/$RUNTIME_MANIFEST" \
    || fail "cannot download the Cargo-Rail Action runtime manifest for $RUNTIME_RELEASE"
  [[ -f "$MANIFEST_PATH" && ! -L "$MANIFEST_PATH" ]] || fail "runtime manifest is not a regular file"
  MANIFEST_BYTES="$(wc -c < "$MANIFEST_PATH" | tr -d ' ')"
  (( MANIFEST_BYTES > 0 && MANIFEST_BYTES <= MAX_MANIFEST_BYTES )) || fail "runtime manifest exceeds 64 KiB"
  [[ "$(tail -c 1 "$MANIFEST_PATH" | od -An -t x1 | tr -d ' ')" == 0a ]] || fail "runtime manifest is not LF terminated"
  if LC_ALL=C grep -q $'\r' "$MANIFEST_PATH"; then
    fail "runtime manifest contains carriage returns"
  fi
  HEADER_LINE="$(head -n 1 "$MANIFEST_PATH")"
  [[ "$HEADER_LINE" == $'cargo-rail-action-runtime-v1\t'"$RUNTIME_VERSION" ]] \
    || fail "runtime manifest version header is incompatible"

  EXPECTED_TARGETS=$'aarch64-apple-darwin\nx86_64-pc-windows-msvc\nx86_64-unknown-linux-gnu'
  OBSERVED_TARGETS=""
  RUNTIME_ASSET=""
  RUNTIME_BYTES=""
  RUNTIME_DIGEST=""
  while IFS= read -r ROW; do
    [[ -n "$ROW" ]] || fail "runtime manifest contains a blank row"
    ROW_WITHOUT_TABS="${ROW//$'\t'/}"
    (( ${#ROW} - ${#ROW_WITHOUT_TABS} == 3 )) || fail "runtime manifest row must contain exactly four columns"
    IFS=$'\t' read -r ROW_TARGET ROW_ASSET ROW_BYTES ROW_DIGEST <<< "$ROW"
    [[ "$ROW_TARGET" =~ ^[a-z0-9_-]+$ && "$ROW_ASSET" != */* && "$ROW_ASSET" != *\\* \
      && "$ROW_ASSET" != .* && "$ROW_BYTES" =~ ^(0|[1-9][0-9]*)$ && "$ROW_DIGEST" =~ ^[0-9a-f]{64}$ ]] \
      || fail "runtime manifest contains invalid authority"
    case "$ROW_TARGET" in
      aarch64-apple-darwin) EXPECTED_ASSET="cargo-rail-action-aarch64-apple-darwin" ;;
      x86_64-pc-windows-msvc) EXPECTED_ASSET="cargo-rail-action-x86_64-pc-windows-msvc.exe" ;;
      x86_64-unknown-linux-gnu) EXPECTED_ASSET="cargo-rail-action-x86_64-unknown-linux-gnu" ;;
      *) fail "runtime manifest advertises an unsupported target" ;;
    esac
    [[ "$ROW_ASSET" == "$EXPECTED_ASSET" ]] || fail "runtime manifest target and asset disagree"
    (( ROW_BYTES > 0 && ROW_BYTES <= MAX_RUNTIME_BYTES )) || fail "runtime manifest executable exceeds 32 MiB"
    OBSERVED_TARGETS+="${OBSERVED_TARGETS:+$'\n'}$ROW_TARGET"
    if [[ "$ROW_TARGET" == "$TARGET" ]]; then
      [[ -z "$RUNTIME_ASSET" ]] || fail "runtime manifest contains duplicate targets"
      RUNTIME_ASSET="$ROW_ASSET"
      RUNTIME_BYTES="$ROW_BYTES"
      RUNTIME_DIGEST="$ROW_DIGEST"
    fi
  done < <(tail -n +2 "$MANIFEST_PATH")
  [[ "$OBSERVED_TARGETS" == "$EXPECTED_TARGETS" ]] || fail "runtime manifest target rows are incomplete, duplicated, or unsorted"
  [[ -n "$RUNTIME_ASSET" ]] || fail "runtime manifest has no executable for $TARGET"
fi

if [[ -n "${RUNNER_TOOL_CACHE:-}" && -d "$RUNNER_TOOL_CACHE" ]]; then
  INSTALL_BASE="$(cd -- "$RUNNER_TOOL_CACHE" && pwd -P)/cargo-rail-action/runtime"
else
  INSTALL_BASE="$RUNNER_TEMP_PATH/cargo-rail-action-runtime"
fi
mkdir -p -- "$INSTALL_BASE/$RUNTIME_VERSION"
DESTINATION="$INSTALL_BASE/$RUNTIME_VERSION/$TARGET-$RUNTIME_DIGEST"
RUNTIME_PATH="$DESTINATION/$RUNTIME_ASSET"
runtime_destination_is_exact() (
  [[ -d "$DESTINATION" && ! -L "$DESTINATION" && -f "$RUNTIME_PATH" && ! -L "$RUNTIME_PATH" ]] || return 1
  shopt -s dotglob nullglob
  local entries=("$DESTINATION"/*)
  (( ${#entries[@]} == 1 )) && [[ "${entries[0]}" == "$RUNTIME_PATH" ]] \
    && [[ "$(wc -c < "$RUNTIME_PATH" | tr -d ' ')" == "$RUNTIME_BYTES" ]] \
    && [[ "$(sha256_file "$RUNTIME_PATH")" == "$RUNTIME_DIGEST" ]]
)
if [[ ! -e "$DESTINATION" && ! -L "$DESTINATION" ]]; then
  if [[ -z "$LOCAL_RUNTIME" ]]; then
    RUNTIME_SOURCE="$BOOTSTRAP_TEMP/$RUNTIME_ASSET"
    curl --fail --silent --show-error --location --retry 3 --proto '=https' --tlsv1.2 \
      --max-filesize "$MAX_RUNTIME_BYTES" --output "$RUNTIME_SOURCE" "$RELEASE_ROOT/$RUNTIME_ASSET" \
      || fail "cannot download the Cargo-Rail Action runtime for $TARGET"
    [[ -f "$RUNTIME_SOURCE" && ! -L "$RUNTIME_SOURCE" ]] || fail "downloaded runtime is not a regular file"
    [[ "$(wc -c < "$RUNTIME_SOURCE" | tr -d ' ')" == "$RUNTIME_BYTES" ]] || fail "runtime byte length does not match the manifest"
    [[ "$(sha256_file "$RUNTIME_SOURCE")" == "$RUNTIME_DIGEST" ]] || fail "runtime digest does not match the manifest"
  fi
  STAGE="$(mktemp -d "$INSTALL_BASE/$RUNTIME_VERSION/.runtime-stage.XXXXXX")"
  chmod 700 "$STAGE"
  cp -- "$RUNTIME_SOURCE" "$STAGE/$RUNTIME_ASSET"
  chmod 700 "$STAGE/$RUNTIME_ASSET"
  [[ "$(wc -c < "$STAGE/$RUNTIME_ASSET" | tr -d ' ')" == "$RUNTIME_BYTES" \
    && "$(sha256_file "$STAGE/$RUNTIME_ASSET")" == "$RUNTIME_DIGEST" ]] \
    || fail "staged runtime failed authentication"
  LOCK_CANDIDATE="$INSTALL_BASE/$RUNTIME_VERSION/.$TARGET-$RUNTIME_DIGEST.publication-lock"
  if mkdir -- "$LOCK_CANDIDATE" 2>/dev/null; then
    PUBLICATION_LOCK="$LOCK_CANDIDATE"
    if [[ -e "$DESTINATION" ]]; then
      rm -rf -- "$STAGE"
    else
      mv -- "$STAGE" "$DESTINATION"
    fi
    rmdir -- "$PUBLICATION_LOCK"
    PUBLICATION_LOCK=""
  else
    for (( attempt = 0; attempt < 100; attempt++ )); do
      [[ -e "$DESTINATION" || ! -e "$LOCK_CANDIDATE" ]] && break
      sleep 0.05
    done
    rm -rf -- "$STAGE"
  fi
fi

runtime_destination_is_exact \
  || fail "immutable runtime destination is corrupt; remove $DESTINATION and rerun"
"$RUNTIME_PATH" self-check --expect-version "$RUNTIME_VERSION" --expect-target "$TARGET" \
  || fail "the installed Action runtime rejected its version or target identity"
rm -rf -- "$BOOTSTRAP_TEMP"
trap - EXIT
exec "$RUNTIME_PATH" "$@"
