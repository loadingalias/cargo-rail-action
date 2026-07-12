#!/usr/bin/env bash
set -euo pipefail

workspace="${1:?workspace path required}"
mkdir -p "$workspace/crates/core/src" "$workspace/crates/api/src" "$workspace/crates/cli/src"

cat > "$workspace/Cargo.toml" <<'EOF'
[workspace]
members = ["crates/*"]
resolver = "2"
EOF

cat > "$workspace/crates/core/Cargo.toml" <<'EOF'
[package]
name = "test-core"
version = "0.1.0"
edition = "2021"
EOF
echo 'pub fn core() {}' > "$workspace/crates/core/src/lib.rs"

cat > "$workspace/crates/api/Cargo.toml" <<'EOF'
[package]
name = "test-api"
version = "0.1.0"
edition = "2021"

[dependencies]
test-core = { path = "../core" }
EOF
echo 'pub fn api() { test_core::core(); }' > "$workspace/crates/api/src/lib.rs"

cat > "$workspace/crates/cli/Cargo.toml" <<'EOF'
[package]
name = "test-cli"
version = "0.1.0"
edition = "2021"

[dependencies]
test-api = { path = "../api" }
EOF
echo 'fn main() { test_api::api(); }' > "$workspace/crates/cli/src/main.rs"

git -C "$workspace" init --initial-branch=main
git -C "$workspace" config user.email "test@test.com"
git -C "$workspace" config user.name "Test"
git -C "$workspace" add -A
git -C "$workspace" commit -m "initial"
echo '// change' >> "$workspace/crates/core/src/lib.rs"
git -C "$workspace" add -A
git -C "$workspace" commit -m "modify core"
