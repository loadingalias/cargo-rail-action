# Automated checks; CI runs this recipe on every native host.
check:
    cargo fmt --all --check
    cargo clippy --all-targets --all-features --locked -- -D warnings
    cargo nextest run --all-targets --all-features --locked
    bash -n scripts/bootstrap.sh scripts/read-cargo-rail-lock.sh
    bash scripts/read-cargo-rail-lock.sh
    git diff --check

# Markdown lint runs on the workstation, where the pinned rumdl is installed.
check-markdown:
    rumdl check .

fix:
    cargo fmt --all
    cargo clippy --all-targets --all-features --locked --fix --allow-dirty --allow-staged
    cargo fmt --all
    rumdl fmt .

check-cargo-rail binary archive version:
    CARGO_RAIL_TEST_BINARY={{quote(binary)}} CARGO_RAIL_TEST_RELEASE_ARCHIVE={{quote(archive)}} CARGO_RAIL_TEST_RELEASE_VERSION={{quote(version)}} cargo test --all-targets --all-features --locked --no-fail-fast -- --ignored

check-locked-cargo-rail binary archive:
    #!/usr/bin/env bash
    set -euo pipefail
    version="$(bash scripts/read-cargo-rail-lock.sh | sed -n 's/^version=//p')"
    just check-cargo-rail {{quote(binary)}} {{quote(archive)}} "$version"
