# Changelog

## [10.1.0] - 2026-09-28
- The cache action accepts the optional `recent_evictions` record that Cargo-Rail's cache status reports
  when collection evicts results within a day of their last use,
  and still rejects any malformed record.

- Every public action now installs Cargo-Rail 0.30.0 by default,
  the only version this release supports.
  Plans are still contract v9 and cache status is still schema 18; planning evidence is `planning-evidence-v2`,
  which 0.30.0 records.
  Compiler caches start cold once after the upgrade,
  because Cargo-Rail 0.30.0 changed its action keys.
  CI tool installation on Linux and Windows now authenticates `cargo-binstall`'s GitHub API lookups,
  as macOS already did.

- The cache action now accepts only Cargo-Rail cache status schema 18.
  It rejects the retired schema 16, which no Cargo-Rail release has emitted since v0.28.0.
  The README states that each Action release supports exactly the Cargo-Rail version in its lock.

- Selectors and the planner now reject a plan created on another platform
  before checkout verification.
  The error names both platforms and tells the operator to create a plan on this platform.
  The README adds a grouped-job example that guards each selector with its own work ID,
  and a macOS job that routes on a Linux plan but plans again before reading selectors.
  The planner summary now lists the platform, source, Cargo, toolchain,
  and target bindings that every selector verifies.
  The summary also states whether portable evidence was supplied,
  and each widened item names the changed files that lack negative evidence.

- Authenticate shallow pull-request history fetches with exactly one repository credential,
  including checkouts that persisted their own credentials.
  Report a failed history fetch with a safe cause and one recovery action instead of an exit code.
  `target-args` now narrows Cargo work only when every selected target is an integration test.

- The planner's `evidence` input accepts a directory and passes each `*.json` file in it to Cargo-Rail,
  so a workflow can restore one evidence file per recorded work item and tolerate a cache miss.
  The plan reader accepts `planning-evidence-v2` identities, which the matching Cargo-Rail release records,
  and the summary reports how many evidence manifests the plan used.
  The README shows how to record evidence in default-branch jobs, save it by commit,
  and restore the base commit's evidence before planning.

- The README no longer recommends `mode: read` for pull requests and other untrusted jobs.
  A job that runs untrusted code gets no remote cache credentials and does not run the cache action,
  because read access exposes every cached result to that job.
  `read` is for trusted jobs that must not publish, and `read-write` for trusted jobs that seed the cache.
  The README also shows how to verify a release asset's attestation with `gh attestation verify`.

- The planner now audits every tracked or unignored YAML file for Cargo-Rail Action references
  before planning, and `cargo-rail-action audit` runs the same audit locally.
  An earlier major version, an input or output the current release does not provide,
  a `needs` consumer of an output the planning job does not export, or an unparseable file stops the planner.
  Exporting `plan-file` as a job output and reading a plan on another runner label produce warnings.
## [10.0.1] - 2026-09-19
- Attest the complete Action release asset set, including the runtime manifest and license, so every downloaded asset
  can be verified against the exact GitHub-hosted package workflow and source commit.
  
  Accept Cargo-Rail release-record v10 while retaining independent fail-closed schema, identity, checkout, invocation,
  artifact, and effect-order validation.
  
  Synchronize every public Action entry point with the authenticated Cargo-Rail release that writes release-record v10.
  
  Preserve the Action's historical `v{version}` tag namespace under Cargo-Rail's crate-qualified default.
  
  Document that cache setup must precede plan capture when both actions run in one job,
  so later selectors validate the same Cargo configuration that the planner captured.
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
