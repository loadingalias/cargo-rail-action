---
cargo-rail-action = "major"
---

Run the stateless `cargo rail release` from Cargo-Rail 0.32.
The release Action installs Cargo-Rail and runs `cargo rail release --yes`, or `--dry-run`;
running it again finishes an interrupted release.

- Inputs are `crates`, `bump`, `pull-request`, `dry-run`, `token`, and `version`.
  The `packages`, `publish`, and `review` inputs are removed.
- Outputs are `state`, `commit`, `tags` (a JSON array), and `pull-request`.
  The `transaction-id`, `release-sha`, and `run-url` outputs are removed.
  A pending check on the source commit fails the step.
- The `token` input reaches GitHub and the release push alone, through process-scoped Git configuration,
  so the checkout keeps no credentials.
  The release commit uses the `github-actions[bot]` identity unless the checkout configures one.
- The new `release/asset` Action attests files and attaches them to the draft release
  that `cargo rail release` waits on.
- The release-record validator, the `release-record-v11` schema, and the reviewed-merge path through `pull_request_target` are removed.
  Release workflows no longer pass `transaction`, `intent`, or `source` dispatch inputs.
- The `v{major}` alias follows each major release without a configuration edit.
