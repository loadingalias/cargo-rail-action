#!/usr/bin/env bash
set -euo pipefail

case "${CHECKSUM_MODE:-required}" in
  required|if-available|off) ;;
  *)
    echo "::error::Invalid checksum mode: ${CHECKSUM_MODE}. Use required, if-available, or off"
    exit 1
    ;;
esac

if [[ "$REQUESTED_VERSION" != "latest" \
  && ! "$REQUESTED_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.-]+)?$ ]]; then
  echo "::error::version must be latest or a semantic version"
  exit 1
fi

# Resolve version (latest -> actual version number)
VERSION="$REQUESTED_VERSION"
if [[ "$VERSION" == "latest" ]]; then
  RESOLVED_VERSION=$(gh api repos/loadingalias/cargo-rail/releases/latest --jq '.tag_name' 2>/dev/null | sed 's/^v//' || true)
  if [[ -n "$RESOLVED_VERSION" ]]; then
    VERSION="$RESOLVED_VERSION"
  else
    echo "::warning::Could not determine latest version from GitHub API"
    VERSION="latest"
  fi
fi

cache_helpers_present () {
  local directory suffix=""
  directory="$(dirname "$(command -v cargo-rail)")"
  [[ "$RUNNER_OS" == "Windows" ]] && suffix=".exe"
  [[ -f "$directory/cargo-rail-native-rustc-wrapper${suffix}" \
    && -f "$directory/cargo-rail-native-rustc-worker${suffix}" ]]
}

# Check if already installed and matches resolved version requirement.
# Transparent setup requires the adjacent launcher and worker too.
if command -v cargo-rail &>/dev/null; then
  INSTALLED_VERSION=$(cargo-rail rail --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || echo "unknown")
  if [[ "$VERSION" != "latest" && "$INSTALLED_VERSION" == "$VERSION" ]] && cache_helpers_present; then
    echo "cargo-rail $INSTALLED_VERSION already installed"
    echo "method=cached" >> "$GITHUB_OUTPUT"
    echo "version=$INSTALLED_VERSION" >> "$GITHUB_OUTPUT"
    exit 0
  fi
fi

# Determine target triple from runner environment
case "$RUNNER_OS-$RUNNER_ARCH" in
  Linux-X64)   TARGET="x86_64-unknown-linux-gnu" ;;
  Linux-ARM64) TARGET="aarch64-unknown-linux-gnu" ;;
  macOS-X64)   TARGET="x86_64-apple-darwin" ;;
  macOS-ARM64) TARGET="aarch64-apple-darwin" ;;
  Windows-X64) TARGET="x86_64-pc-windows-msvc" ;;
  Windows-ARM64) TARGET="aarch64-pc-windows-msvc" ;;
  *)
    echo "::warning::Unknown platform $RUNNER_OS-$RUNNER_ARCH, will try cargo install"
    TARGET=""
    ;;
esac

# Fast path: Pre-built binary download
if [[ -n "$TARGET" && -n "$VERSION" ]]; then
  echo "Attempting binary download: cargo-rail v$VERSION for $TARGET"

  if [[ "$RUNNER_OS" == "Windows" ]]; then
    ARCHIVE="cargo-rail-${TARGET}.zip"
  else
    ARCHIVE="cargo-rail-${TARGET}.tar.gz"
  fi

  URL="https://github.com/loadingalias/cargo-rail/releases/download/v${VERSION}/${ARCHIVE}"
  INSTALL_DIR="$HOME/.cargo/bin"
  mkdir -p "$INSTALL_DIR"
  calc_sha256 () {
    local file="$1"
    if command -v sha256sum &>/dev/null; then
      sha256sum "$file" | awk '{print $1}'
    elif command -v shasum &>/dev/null; then
      shasum -a 256 "$file" | awk '{print $1}'

    else

      if command -v python3 &>/dev/null; then
        python3 -c 'import hashlib,sys; p=sys.argv[1]; h=hashlib.sha256(); f=open(p,"rb"); [h.update(b) for b in iter(lambda: f.read(1024*1024), b"")]; f.close(); print(h.hexdigest())' "$file"
      elif command -v python &>/dev/null; then
        python -c 'import hashlib,sys; p=sys.argv[1]; h=hashlib.sha256(); f=open(p,"rb"); [h.update(b) for b in iter(lambda: f.read(1024*1024), b"")]; f.close(); print(h.hexdigest())' "$file"
      else
        echo "::error::No sha256 tool found (sha256sum/shasum/python)"
        exit 1
      fi
    fi
  }

  verify_checksum () {
    local archive_path="$1"
    local archive_name="$2"
    local mode="${CHECKSUM_MODE:-required}"

    case "$mode" in
      off)
        echo "Checksum verification disabled (checksum=off)"
        return 0
        ;;
      required|if-available)
        ;;
      *)
        echo "::error::Invalid checksum mode: $mode. Use required, if-available, or off"
        exit 1
        ;;
    esac

    local sums_url="https://github.com/loadingalias/cargo-rail/releases/download/v${VERSION}/SHA256SUMS"
    local sums_path="$DOWNLOAD_DIR/SHA256SUMS"

    if ! curl -fsSL --retry 3 "$sums_url" -o "$sums_path" 2>/dev/null; then
      if [[ "$mode" == "required" ]]; then
        echo "::error::SHA256SUMS not found for v$VERSION; set checksum: if-available or off to skip verification"
        exit 1
      fi
      echo "::warning::SHA256SUMS not found for v$VERSION; skipping checksum verification"
      return 0
    fi

    local expected
    expected="$(awk -v f="$archive_name" '$2==f || $2=="*"f {print $1; exit}' "$sums_path" 2>/dev/null || true)"
    if [[ ! "$expected" =~ ^[0-9A-Fa-f]{64}$ ]]; then
      echo "::error::SHA256SUMS is present for v$VERSION but has no entry for $archive_name"
      exit 1
    fi
    expected="$(printf '%s' "$expected" | tr '[:upper:]' '[:lower:]')"

    local actual
    actual="$(calc_sha256 "$archive_path")"
    if [[ "$actual" != "$expected" ]]; then
      echo "::error::Checksum mismatch for $archive_name (expected $expected, got $actual)"
      exit 1
    fi

    echo "Checksum verified for $archive_name"
  }

  DOWNLOAD_DIR="$(mktemp -d)"
  ARCHIVE_PATH="$DOWNLOAD_DIR/$ARCHIVE"
  trap 'rm -rf "$DOWNLOAD_DIR"' EXIT
  if curl -fsSL --retry 3 "$URL" -o "$ARCHIVE_PATH"; then
    echo "Downloaded $ARCHIVE"
    verify_checksum "$ARCHIVE_PATH" "$ARCHIVE"
    echo "Extracting to $INSTALL_DIR..."

    EXTRACT_DIR="$DOWNLOAD_DIR/extract"
    mkdir "$EXTRACT_DIR"
    if [[ "$RUNNER_OS" == "Windows" ]]; then
      unzip -q "$ARCHIVE_PATH" -d "$EXTRACT_DIR"
      SUFFIX=".exe"
    else
      tar -xf "$ARCHIVE_PATH" -C "$EXTRACT_DIR"
      SUFFIX=""
    fi
    for name in cargo-rail cargo-rail-native-rustc-wrapper cargo-rail-native-rustc-worker; do
      SOURCE="$(find "$EXTRACT_DIR" -type f -name "${name}${SUFFIX}" -print -quit)"
      if [[ -z "$SOURCE" ]]; then
        echo "::error::${name}${SUFFIX} is missing from $ARCHIVE"
        exit 1
      fi
      mv "$SOURCE" "$INSTALL_DIR/${name}${SUFFIX}"
      [[ "$RUNNER_OS" == "Windows" ]] || chmod +x "$INSTALL_DIR/${name}${SUFFIX}"
    done
    echo "Cargo-Rail binaries installed to $INSTALL_DIR"
    rm -rf "$DOWNLOAD_DIR"
    trap - EXIT

    # Add cargo bin to PATH for this and subsequent steps
    echo "$INSTALL_DIR" >> "$GITHUB_PATH"
    export PATH="$INSTALL_DIR:$PATH"

    # Verify binary works (cargo-rail is a cargo subcommand, use "rail --version")
    if ! "$INSTALL_DIR/cargo-rail${SUFFIX}" rail --version || ! cache_helpers_present; then
      echo "::error::cargo-rail binary failed to execute"
      file "$INSTALL_DIR/cargo-rail" || true
      ldd "$INSTALL_DIR/cargo-rail" 2>/dev/null || true
      exit 1
    fi

    INSTALLED_VERSION=$("$INSTALL_DIR/cargo-rail${SUFFIX}" rail --version | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)
    echo "Installed cargo-rail v$INSTALLED_VERSION from binary"
    echo "method=binary" >> "$GITHUB_OUTPUT"
    echo "version=$INSTALLED_VERSION" >> "$GITHUB_OUTPUT"
    exit 0
  else
    echo "::warning::Binary download failed for $URL"
  fi
fi

# Medium path: cargo-binstall (if available)
if command -v cargo-binstall &>/dev/null; then
  echo "Trying cargo-binstall..."
  BINSTALL_ARGS=(--no-confirm --force)
  [[ -n "$VERSION" && "$VERSION" != "latest" ]] && BINSTALL_ARGS+=(--version "$VERSION")

  if cargo binstall cargo-rail "${BINSTALL_ARGS[@]}" 2>/dev/null; then
    if cache_helpers_present; then
      INSTALLED_VERSION=$(cargo-rail rail --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)
      echo "Installed cargo-rail v$INSTALLED_VERSION via cargo-binstall"
      echo "method=binstall" >> "$GITHUB_OUTPUT"
      echo "version=$INSTALLED_VERSION" >> "$GITHUB_OUTPUT"
      exit 0
    fi
    echo "::warning::cargo-binstall omitted compiler-cache binaries; falling back to cargo install"
  fi
fi

# Slow path: cargo install (requires Rust toolchain)
echo "::warning::Falling back to cargo install (this may take 2-5 minutes)"
INSTALL_ARGS=(--locked --force)
[[ -n "$VERSION" && "$VERSION" != "latest" ]] && INSTALL_ARGS+=(--version "$VERSION")

cargo install cargo-rail "${INSTALL_ARGS[@]}"

cache_helpers_present || {
  echo "::error::cargo-rail installation omitted compiler-cache launcher or worker"
  exit 1
}

INSTALLED_VERSION=$(cargo-rail rail --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)
echo "Installed cargo-rail v$INSTALLED_VERSION via cargo install"
echo "method=cargo-install" >> "$GITHUB_OUTPUT"
echo "version=$INSTALLED_VERSION" >> "$GITHUB_OUTPUT"
