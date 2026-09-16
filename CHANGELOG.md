# Changelog

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
