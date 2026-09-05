check:
    cargo fmt --all --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-targets
    bash -n scripts/bootstrap.sh
    git diff --check
