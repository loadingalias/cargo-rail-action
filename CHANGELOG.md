# Changelog

## Unreleased

- Keep planner, cache, setup, and release defaults on the exact Cargo-Rail release in `.github/cargo-rail.lock`
  until a published Action runtime supports authenticated moving selection.
  Preserve exact version pins.
  Publish the authenticated installed version to downstream jobs.
- Generate SLSA provenance for every executable Action runtime in the release asset workflow.
- Keep the Cargo-Rail release version and matching source commit in one validated lock file.
  Reuse one package workflow for CI and release validation,
  and build release assets only on GitHub-hosted runners.
- Publish a stable `cargo-rail-action` launcher for later workflow steps
  without changing the authenticated target runtime's immutable installation.
- Restrict this repository's publication workflow to manual `main` dispatches and fixed release policy.
  Derive the Action runtime release from `Cargo.toml`
  and retain only the inputs required to resume a durable transaction.

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
