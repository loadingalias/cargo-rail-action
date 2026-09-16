# Changelog

## [10.0.0] - 2026-09-16
- Add a native Linux ARM64 runtime and validate Cargo-Rail Surface preparation against its ready outcome.
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
## [9.0.1] - 2026-09-13

- Default to Cargo-Rail v0.27.1 with corrected prebuilt installation and GitHub draft recovery.
  Accept stable v0.27.x releases while preserving support for stable v0.26.x releases.

## [9.0.0] - 2026-09-13

- Use the native Action runtime and compatible Cargo-Rail 0.26 components
  for independently validated planning, compiler-cache setup, and release execution.
  Install authenticated component archives, including their source license,
  and reject unknown or inconsistent contracts before exposing outputs.
  Accept LF and CRLF rows in release checksum files collected from native runners,
  while rejecting embedded carriage returns and ambiguous checksums.

  Add the release Action for caller-owned GitHub publication jobs.
  Validate the original release record, repository, workflow, dispatch,
  and reviewed merge before invoking Cargo-Rail.
  Expose the transaction ID, exact release commit, state, and executor URL.
  Recover the same request after runner loss without an Action-specific publication engine.

  Keep runtime manifest generation as product packaging.
  Delegate immutable releases and explicit major-tag promotion to Cargo-Rail,
  including conflict detection and recovery after uncertain effects.
