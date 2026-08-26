#!/usr/bin/env bash
set -euo pipefail

case "${CHECKSUM_MODE:-required}" in
  required|if-available|off) ;;
  *) echo "::error::Invalid checksum mode: ${CHECKSUM_MODE}. Use required, if-available, or off"; exit 1 ;;
esac
case "${COMPONENT_SET:-core}" in
  core|cache|surface|distributed|complete) COMPONENT_SET="${COMPONENT_SET:-core}" ;;
  *) echo "::error::Invalid Cargo-Rail component set: ${COMPONENT_SET:-}"; exit 1 ;;
esac
if [[ "$REQUESTED_VERSION" != "latest" \
  && ! "$REQUESTED_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.-]+)?$ ]]; then
  echo "::error::version must be latest or a semantic version"
  exit 1
fi

VERSION="$REQUESTED_VERSION"
if [[ "$VERSION" == "latest" ]]; then
  RESOLVED_VERSION=$(gh api repos/loadingalias/cargo-rail/releases/latest --jq '.tag_name' 2>/dev/null | sed 's/^v//' || true)
  if [[ -n "$RESOLVED_VERSION" ]]; then
    VERSION="$RESOLVED_VERSION"
  else
    echo "::warning::Could not determine latest version from GitHub API"
  fi
fi

case "$RUNNER_OS-$RUNNER_ARCH" in
  Linux-X64) TARGET="x86_64-unknown-linux-gnu"; SUFFIX="" ;;
  Linux-ARM64) TARGET="aarch64-unknown-linux-gnu"; SUFFIX="" ;;
  macOS-ARM64) TARGET="aarch64-apple-darwin"; SUFFIX="" ;;
  Windows-X64) TARGET="x86_64-pc-windows-msvc"; SUFFIX=".exe" ;;
  Windows-ARM64) TARGET="aarch64-pc-windows-msvc"; SUFFIX=".exe" ;;
  *) TARGET=""; SUFFIX="" ;;
esac

INSTALL_DIR="$HOME/.cargo/bin"
RECEIPT="$INSTALL_DIR/cargo-rail-components-${COMPONENT_SET}-v1.tsv"

selected_capability_counts () {
  printf '%s\n' "core 1"
  case "$COMPONENT_SET" in
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

selected_capabilities () {
  selected_capability_counts | awk '{print $1}' | sort
}

selected_names () {
  local manifest="$1"
  if [[ "$COMPONENT_SET" == complete ]]; then
    sed '1d' "$manifest" | cut -f1
    return
  fi
  local capabilities
  capabilities="$(selected_capabilities | paste -sd, -)"
  awk -F '\t' -v capabilities="$capabilities" '
    BEGIN { count = split(capabilities, selected, ","); for (i = 1; i <= count; i++) wanted[selected[i]] = 1 }
    NR > 1 && wanted[$4] { print $1 }
  ' "$manifest"
}

component_set_is_available () {
  local manifest="$1" capability expected actual
  while read -r capability expected; do
    actual="$(awk -F '\t' -v capability="$capability" 'NR > 1 && $4 == capability { count++ } END { print count + 0 }' "$manifest")"
    [[ "$actual" == "$expected" ]] || return 1
  done < <(selected_capability_counts)
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
  [[ -n "$TARGET" && -f "$RECEIPT" ]] || return 1
  local header expected_header name digest bytes capability extra file declared_capabilities required_capabilities
  header="$(sed -n '1p' "$RECEIPT")"
  expected_header="$(printf 'cargo-rail-installed-components-v1\t%s\t%s\t%s' "$VERSION" "$TARGET" "$COMPONENT_SET")"
  [[ "$header" == "$expected_header" ]] || return 1
  while IFS=$'\t' read -r name digest bytes capability extra; do
    [[ -n "$name" ]] || continue
    [[ -z "${extra:-}" && "$name" != */* && "$name" != *\\* && "$name" != .* \
      && "$digest" =~ ^[0-9a-f]{64}$ && "$bytes" =~ ^[0-9]+$ && -n "$capability" ]] || return 1
    file="$INSTALL_DIR/$name"
    [[ -f "$file" && ! -L "$file" ]] || return 1
    [[ "$(wc -c < "$file" | tr -d ' ')" == "$bytes" ]] || return 1
    [[ "$(calc_sha256 "$file")" == "$digest" ]] || return 1
  done < <(sed '1d' "$RECEIPT")
  [[ -z "$(sed '1d' "$RECEIPT" | cut -f1 | sort | uniq -d)" ]] || return 1
  component_set_is_available "$RECEIPT" || return 1
  if [[ "$COMPONENT_SET" != complete ]]; then
    declared_capabilities="$(sed '1d' "$RECEIPT" | cut -f4 | sort -u)"
    required_capabilities="$(selected_capabilities)"
    [[ "$declared_capabilities" == "$required_capabilities" ]] || return 1
  fi
}

if [[ "$VERSION" != "latest" ]] && command -v cargo-rail &>/dev/null && installed_set_is_valid; then
  INSTALLED_VERSION=$(cargo-rail rail --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || echo unknown)
  if [[ "$INSTALLED_VERSION" == "$VERSION" ]]; then
    echo "cargo-rail $INSTALLED_VERSION $COMPONENT_SET component set already installed"
    echo "method=cached" >> "$GITHUB_OUTPUT"
    echo "version=$INSTALLED_VERSION" >> "$GITHUB_OUTPUT"
    exit 0
  fi
fi

verify_archive_checksum () {
  local archive_path="$1" archive_name="$2" mode="${CHECKSUM_MODE:-required}"
  if [[ "$mode" == off ]]; then
    echo "Checksum verification disabled (checksum=off)"
    return 0
  fi
  local sums_url="https://github.com/loadingalias/cargo-rail/releases/download/v${VERSION}/SHA256SUMS"
  local sums_path="$DOWNLOAD_DIR/SHA256SUMS"
  if ! curl -fsSL --retry 3 "$sums_url" -o "$sums_path" 2>/dev/null; then
    if [[ "$mode" == required ]]; then
      echo "::error::SHA256SUMS not found for v$VERSION"
      exit 1
    fi
    echo "::warning::SHA256SUMS not found for v$VERSION; skipping checksum verification"
    return 0
  fi
  local expected actual
  expected="$(awk -v f="$archive_name" '$2==f || $2=="*"f {print $1; exit}' "$sums_path" 2>/dev/null || true)"
  [[ "$expected" =~ ^[0-9A-Fa-f]{64}$ ]] || { echo "::error::SHA256SUMS has no entry for $archive_name"; exit 1; }
  expected="$(printf '%s' "$expected" | tr '[:upper:]' '[:lower:]')"
  actual="$(calc_sha256 "$archive_path")"
  [[ "$actual" == "$expected" ]] || { echo "::error::Checksum mismatch for $archive_name"; exit 1; }
}

if [[ -n "$TARGET" && "$VERSION" != "latest" ]]; then
  ARCHIVE="cargo-rail-${TARGET}$([[ "$RUNNER_OS" == Windows ]] && printf '.zip' || printf '.tar.gz')"
  URL="https://github.com/loadingalias/cargo-rail/releases/download/v${VERSION}/${ARCHIVE}"
  DOWNLOAD_DIR="$(mktemp -d)"
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
    MANIFESTS="$(find "$EXTRACT_DIR" -type f -name cargo-rail-components-v1.tsv -print)"
    [[ "$(printf '%s\n' "$MANIFESTS" | sed '/^$/d' | wc -l | tr -d ' ')" == 1 ]] || {
      echo "::error::$ARCHIVE must contain exactly one component manifest"; exit 1
    }
    MANIFEST="$MANIFESTS"
    COMPONENT_DIR="$(dirname "$MANIFEST")"
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
    component_set_is_available "$MANIFEST" || {
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
    installed_set_is_valid || { echo "::error::Installed Cargo-Rail component set failed authentication"; exit 1; }
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
  BINSTALL_ARGS=(--no-confirm --force)
  [[ "$VERSION" != latest ]] && BINSTALL_ARGS+=(--version "$VERSION")
  if cargo binstall cargo-rail "${BINSTALL_ARGS[@]}"; then
    INSTALLED_VERSION=$(cargo-rail rail --version | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)
    echo "method=binstall" >> "$GITHUB_OUTPUT"
    echo "version=$INSTALLED_VERSION" >> "$GITHUB_OUTPUT"
    exit 0
  fi
fi

INSTALL_ARGS=(--locked --force)
[[ "$VERSION" != latest ]] && INSTALL_ARGS+=(--version "$VERSION")
cargo install cargo-rail "${INSTALL_ARGS[@]}"
INSTALLED_VERSION=$(cargo-rail rail --version | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)
echo "method=cargo-install" >> "$GITHUB_OUTPUT"
echo "version=$INSTALLED_VERSION" >> "$GITHUB_OUTPUT"
