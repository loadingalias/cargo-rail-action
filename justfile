check:
    cargo fmt --all --check
    cargo clippy --all-targets --all-features --locked -- -D warnings
    cargo test --all-targets --all-features --locked --no-fail-fast
    bash -n scripts/bootstrap.sh scripts/read-cargo-rail-lock.sh
    bash scripts/read-cargo-rail-lock.sh
    git diff --check

check-cargo-rail binary archive version:
    CARGO_RAIL_TEST_BINARY={{quote(binary)}} CARGO_RAIL_TEST_RELEASE_ARCHIVE={{quote(archive)}} CARGO_RAIL_TEST_RELEASE_VERSION={{quote(version)}} cargo test --all-targets --all-features --locked --no-fail-fast -- --ignored

check-locked-cargo-rail binary archive:
    #!/usr/bin/env bash
    set -euo pipefail
    version="$(bash scripts/read-cargo-rail-lock.sh | sed -n 's/^version=//p')"
    just check-cargo-rail {{quote(binary)}} {{quote(archive)}} "$version"
