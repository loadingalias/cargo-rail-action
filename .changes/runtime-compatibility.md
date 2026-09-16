---
"cargo-rail-action" = "major"
---

Add a native Linux ARM64 runtime and validate Cargo-Rail Surface preparation against its ready outcome.
Bind planner, setup, cache, release, packaging, and validation to one published Cargo-Rail version and matching
release commit while retaining a separate reviewed CI tooling commit.
Preserve exact Cargo-Rail selections and publish the authenticated installed version to downstream jobs.

Reuse one GitHub-hosted package workflow for CI and release validation.
Generate SLSA provenance for every executable Action runtime and authorize only the asset-collection job to create
attestations.
Publish a stable `cargo-rail-action` launcher for later workflow steps without changing the authenticated target
runtime's immutable installation.

Allow immutable pre-release integrations to build and authenticate the checked-out Action runtime explicitly.
Install the complete cataloged CI tool set on macOS, including Nextest, and honor the pinned Windows Bash path.
Publish cache record paths in the Windows form accepted by GitHub artifact upload.
Accept both released and current Cargo-Rail cache status contracts while strictly validating readiness, integrity,
byte accounting, remote authority, and recoverable quarantine receipts.
Reuse valid cache reports from earlier attempts of the same GitHub workflow run, prefer the latest report for each
job, and reject reports from another run or a future attempt.

Restrict the Action release workflow to manual `main` dispatches and fixed package, bump, publication, and review
policy.
Derive the Action runtime release from `Cargo.toml` and retain only the inputs required to resume a durable Cargo-Rail
release transaction.
