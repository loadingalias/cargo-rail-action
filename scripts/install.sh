#!/usr/bin/env bash
set -euo pipefail

case "${COMPONENT_SET:-core}" in
  core|cache|surface|distributed|complete) COMPONENT_SET="${COMPONENT_SET:-core}" ;;
  *) echo "::error::Invalid Cargo-Rail component set: ${COMPONENT_SET:-}"; exit 1 ;;
esac
if [[ ! "$REQUESTED_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.-]+)?$ ]]; then
  echo "::error::version must be an exact semantic version"
  exit 1
fi

VERSION="$REQUESTED_VERSION"

linux_libc() {
  if [[ -n "${ACTION_RUNNER_LIBC:-}" ]]; then
    case "$ACTION_RUNNER_LIBC" in
      gnu|musl) printf '%s\n' "$ACTION_RUNNER_LIBC" ;;
      *) echo "::error::Invalid internal Linux libc override: $ACTION_RUNNER_LIBC" >&2; exit 1 ;;
    esac
  elif grep -qi musl <<< "$(ldd --version 2>&1 || true)"; then
    printf 'musl\n'
  else
    printf 'gnu\n'
  fi
}

case "$RUNNER_OS-$RUNNER_ARCH" in
  Linux-X64) TARGET="x86_64-unknown-linux-$(linux_libc)"; SUFFIX="" ;;
  Linux-ARM64) TARGET="aarch64-unknown-linux-$(linux_libc)"; SUFFIX="" ;;
  macOS-ARM64) TARGET="aarch64-apple-darwin"; SUFFIX="" ;;
  Windows-X64) TARGET="x86_64-pc-windows-msvc"; SUFFIX=".exe" ;;
  Windows-ARM64) TARGET="aarch64-pc-windows-msvc"; SUFFIX=".exe" ;;
  *) TARGET=""; SUFFIX="" ;;
esac

INSTALL_DIR="$HOME/.cargo/bin"
RECEIPT="$INSTALL_DIR/cargo-rail-components-${COMPONENT_SET}-v1.tsv"

capability_counts_for_set () {
  local selected_set="$1"
  printf '%s\n' "core 1"
  case "$selected_set" in
    cache)
      printf '%s\n' "cache 2"
      ;;
    surface)
      printf '%s\n' "analysis 1" "surface 1" "surface-source 1"
      ;;
    distributed)
      printf '%s\n' "distributed 1"
      ;;
    complete)
      printf '%s\n' "analysis 1" "cache 2" "distributed 1" "surface 1" "surface-source 1"
      ;;
  esac
}

capabilities_for_set () {
  capability_counts_for_set "$1" | awk '{print $1}' | sort
}

selected_names () {
  local manifest="$1"
  if [[ "$COMPONENT_SET" == complete ]]; then
    sed '1d' "$manifest" | cut -f1
    return
  fi
  local capabilities
  capabilities="$(capabilities_for_set "$COMPONENT_SET" | paste -sd, -)"
  awk -F '\t' -v capabilities="$capabilities" '
    BEGIN { count = split(capabilities, selected, ","); for (i = 1; i <= count; i++) wanted[selected[i]] = 1 }
    NR > 1 && wanted[$4] { print $1 }
  ' "$manifest"
}

component_set_is_available () {
  local manifest="$1" selected_set="$2" capability expected actual
  while read -r capability expected; do
    actual="$(awk -F '\t' -v capability="$capability" 'NR > 1 && $4 == capability { count++ } END { print count + 0 }' "$manifest")"
    [[ "$actual" == "$expected" ]] || return 1
  done < <(capability_counts_for_set "$selected_set")
}

calc_sha256 () {
  local file="$1"
  if command -v sha256sum &>/dev/null; then
    sha256sum "$file" | awk '{print $1}'
  elif command -v shasum &>/dev/null; then
    shasum -a 256 "$file" | awk '{print $1}'
  elif command -v python3 &>/dev/null; then
    python3 -c 'import hashlib,sys; print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$file"
  elif command -v python &>/dev/null; then
    python -c 'import hashlib,sys; print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$file"
  else
    echo "::error::No sha256 tool found (sha256sum/shasum/python)"
    exit 1
  fi
}

installed_set_is_valid () {
  local receipt="$1" required_set="$2"
  [[ -n "$TARGET" && -f "$receipt" && ! -L "$receipt" ]] || return 1
  local header kind receipt_version receipt_target declared_set extra name digest bytes capability file
  local declared_capabilities expected_capabilities required_capability
  header="$(sed -n '1p' "$receipt")"
  IFS=$'\t' read -r kind receipt_version receipt_target declared_set extra <<< "$header"
  [[ "$kind" == cargo-rail-installed-components-v1 && "$receipt_version" == "$VERSION" \
    && "$receipt_target" == "$TARGET" && -z "${extra:-}" ]] || return 1
  case "$declared_set" in core|cache|surface|distributed|complete) ;; *) return 1 ;; esac
  while IFS=$'\t' read -r name digest bytes capability extra; do
    [[ -n "$name" ]] || continue
    [[ -z "${extra:-}" && "$name" != */* && "$name" != *\\* && "$name" != .* \
      && "$digest" =~ ^[0-9a-f]{64}$ && "$bytes" =~ ^[0-9]+$ && -n "$capability" ]] || return 1
    file="$INSTALL_DIR/$name"
    [[ -f "$file" && ! -L "$file" ]] || return 1
    [[ "$(wc -c < "$file" | tr -d ' ')" == "$bytes" ]] || return 1
    [[ "$(calc_sha256 "$file")" == "$digest" ]] || return 1
  done < <(sed '1d' "$receipt")
  [[ -n "$(sed '1d' "$receipt")" ]] || return 1
  [[ -z "$(sed '1d' "$receipt" | cut -f1 | sort | uniq -d)" ]] || return 1
  component_set_is_available "$receipt" "$declared_set" || return 1
  declared_capabilities="$(sed '1d' "$receipt" | cut -f4 | sort -u)"
  expected_capabilities="$(capabilities_for_set "$declared_set")"
  [[ "$declared_capabilities" == "$expected_capabilities" ]] || return 1
  while IFS= read -r required_capability; do
    grep -qx "$required_capability" <<< "$declared_capabilities" || return 1
  done < <(capabilities_for_set "$required_set")
}

compatible_installed_sets () {
  case "$COMPONENT_SET" in
    core) printf '%s\n' core cache surface distributed complete ;;
    cache) printf '%s\n' cache complete ;;
    surface) printf '%s\n' surface complete ;;
    distributed) printf '%s\n' distributed complete ;;
    complete) printf '%s\n' complete ;;
  esac
}

while IFS= read -r installed_set; do
  candidate="$INSTALL_DIR/cargo-rail-components-${installed_set}-v1.tsv"
  if installed_set_is_valid "$candidate" "$COMPONENT_SET"; then
    INSTALLED_VERSION=$("$INSTALL_DIR/cargo-rail${SUFFIX}" rail --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || echo unknown)
    if [[ "$INSTALLED_VERSION" == "$VERSION" ]]; then
      echo "cargo-rail $INSTALLED_VERSION $installed_set component set satisfies requested $COMPONENT_SET capabilities"
      echo "$INSTALL_DIR" >> "$GITHUB_PATH"
      echo "method=cached" >> "$GITHUB_OUTPUT"
      echo "version=$INSTALLED_VERSION" >> "$GITHUB_OUTPUT"
      exit 0
    fi
  fi
done < <(compatible_installed_sets)

verify_archive_checksum () {
  local archive_path="$1" archive_name="$2"
  local sums_url="https://github.com/loadingalias/cargo-rail/releases/download/v${VERSION}/SHA256SUMS"
  local sums_path="$DOWNLOAD_DIR/SHA256SUMS"
  if ! curl -fsSL --retry 3 "$sums_url" -o "$sums_path" 2>/dev/null; then
    echo "::error::SHA256SUMS not found for v$VERSION"
    exit 1
  fi
  local expected actual
  expected="$(awk -v f="$archive_name" '$2==f || $2=="*"f {print $1; exit}' "$sums_path" 2>/dev/null || true)"
  [[ "$expected" =~ ^[0-9A-Fa-f]{64}$ ]] || { echo "::error::SHA256SUMS has no entry for $archive_name"; exit 1; }
  expected="$(printf '%s' "$expected" | tr '[:upper:]' '[:lower:]')"
  actual="$(calc_sha256 "$archive_path")"
  [[ "$actual" == "$expected" ]] || { echo "::error::Checksum mismatch for $archive_name"; exit 1; }
}

if [[ -n "$TARGET" ]]; then
  ARCHIVE="cargo-rail-${TARGET}$([[ "$RUNNER_OS" == Windows ]] && printf '.zip' || printf '.tar.gz')"
  URL="https://github.com/loadingalias/cargo-rail/releases/download/v${VERSION}/${ARCHIVE}"
  DOWNLOAD_DIR="$(mktemp -d)"
  chmod 700 "$DOWNLOAD_DIR"
  trap 'rm -rf "$DOWNLOAD_DIR"' EXIT
  ARCHIVE_PATH="$DOWNLOAD_DIR/$ARCHIVE"
  if curl -fsSL --retry 3 "$URL" -o "$ARCHIVE_PATH"; then
    verify_archive_checksum "$ARCHIVE_PATH" "$ARCHIVE"
    EXTRACT_DIR="$DOWNLOAD_DIR/extract"
    mkdir "$EXTRACT_DIR"
    if [[ "$RUNNER_OS" == Windows ]]; then
      unzip -q "$ARCHIVE_PATH" -d "$EXTRACT_DIR"
    else
      tar -xzf "$ARCHIVE_PATH" -C "$EXTRACT_DIR"
    fi
    COMPONENT_DIR="$EXTRACT_DIR/cargo-rail-$TARGET"
    MANIFEST="$COMPONENT_DIR/cargo-rail-components-v1.tsv"
    [[ -f "$MANIFEST" && ! -L "$MANIFEST" ]] || {
      echo "::error::$ARCHIVE does not use the required cargo-rail-$TARGET component layout"; exit 1
    }
    HEADER="$(sed -n '1p' "$MANIFEST")"
    EXPECTED_HEADER="$(printf 'cargo-rail-components-v1\t%s\t%s' "$VERSION" "$TARGET")"
    [[ "$HEADER" == "$EXPECTED_HEADER" ]] || { echo "::error::Release component authority is incompatible"; exit 1; }
    while IFS=$'\t' read -r name digest bytes capability extra; do
      [[ -n "$name" ]] || continue
      [[ -z "${extra:-}" && "$name" != */* && "$name" != *\\* && "$name" != .* && "$digest" =~ ^[0-9a-f]{64}$ \
        && "$bytes" =~ ^[0-9]+$ && -n "$capability" ]] || { echo "::error::Invalid component authority"; exit 1; }
      FILE="$COMPONENT_DIR/$name"
      [[ -f "$FILE" && ! -L "$FILE" ]] || { echo "::error::$name is missing from $ARCHIVE"; exit 1; }
      [[ "$(wc -c < "$FILE" | tr -d ' ')" == "$bytes" && "$(calc_sha256 "$FILE")" == "$digest" ]] || {
        echo "::error::Component authority does not match $name"; exit 1
      }
    done < <(sed '1d' "$MANIFEST")
    [[ -z "$(sed '1d' "$MANIFEST" | cut -f1 | sort | uniq -d)" ]] || {
      echo "::error::Release component authority contains duplicate names"; exit 1
    }
    component_set_is_available "$MANIFEST" "$COMPONENT_SET" || {
      echo "::error::Release archive does not provide the complete $COMPONENT_SET component set"; exit 1
    }

    mkdir -p "$INSTALL_DIR"
    STAGE="$(mktemp -d "$INSTALL_DIR/.cargo-rail-action-install.XXXXXX")"
    {
      printf 'cargo-rail-installed-components-v1\t%s\t%s\t%s\n' "$VERSION" "$TARGET" "$COMPONENT_SET"
      while IFS= read -r name; do
        cp "$COMPONENT_DIR/$name" "$STAGE/$name"
        capability="$(awk -F '\t' -v name="$name" '$1 == name { print $4; found = 1 } END { exit !found }' "$MANIFEST")"
        if [[ "$capability" == surface-source ]]; then
          chmod 600 "$STAGE/$name"
        else
          chmod +x "$STAGE/$name"
        fi
        awk -F '\t' -v name="$name" '$1 == name { print; found = 1 } END { exit !found }' "$MANIFEST"
      done < <(selected_names "$MANIFEST")
    } > "$STAGE/receipt.tsv"
    while IFS= read -r name; do
      mv -f "$STAGE/$name" "$INSTALL_DIR/$name"
    done < <(selected_names "$MANIFEST")
    mv -f "$STAGE/receipt.tsv" "$RECEIPT"
    rmdir "$STAGE"
    rm -rf "$DOWNLOAD_DIR"
    trap - EXIT
    echo "$INSTALL_DIR" >> "$GITHUB_PATH"
    export PATH="$INSTALL_DIR:$PATH"
    installed_set_is_valid "$RECEIPT" "$COMPONENT_SET" || { echo "::error::Installed Cargo-Rail component set failed authentication"; exit 1; }
    INSTALLED_VERSION=$("$INSTALL_DIR/cargo-rail${SUFFIX}" rail --version | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)
    [[ "$INSTALLED_VERSION" == "$VERSION" ]] || { echo "::error::Installed Cargo-Rail version mismatch"; exit 1; }
    echo "Installed cargo-rail v$INSTALLED_VERSION ($COMPONENT_SET component set)"
    echo "method=binary" >> "$GITHUB_OUTPUT"
    echo "version=$INSTALLED_VERSION" >> "$GITHUB_OUTPUT"
    exit 0
  fi
fi

if [[ "$COMPONENT_SET" != core ]]; then
  echo "::error::A verified native Cargo-Rail archive is required for the $COMPONENT_SET component set"
  exit 1
fi

if command -v cargo-binstall &>/dev/null; then
  BINSTALL_ARGS=(--no-confirm --force --version "$VERSION")
  if cargo binstall cargo-rail "${BINSTALL_ARGS[@]}"; then
    INSTALLED_VERSION=$(cargo-rail rail --version | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)
    [[ "$INSTALLED_VERSION" == "$VERSION" ]] || { echo "::error::Installed Cargo-Rail version mismatch"; exit 1; }
    echo "method=binstall" >> "$GITHUB_OUTPUT"
    echo "version=$INSTALLED_VERSION" >> "$GITHUB_OUTPUT"
    exit 0
  fi
fi

INSTALL_ARGS=(--locked --force --version "$VERSION")
cargo install cargo-rail "${INSTALL_ARGS[@]}"
INSTALLED_VERSION=$(cargo-rail rail --version | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)
[[ "$INSTALLED_VERSION" == "$VERSION" ]] || { echo "::error::Installed Cargo-Rail version mismatch"; exit 1; }
echo "method=cargo-install" >> "$GITHUB_OUTPUT"
echo "version=$INSTALLED_VERSION" >> "$GITHUB_OUTPUT"
