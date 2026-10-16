---
cargo-rail-action = "major"
---

Run the stateless `cargo rail release` from Cargo-Rail 0.32.
The release Action installs Cargo-Rail and runs `cargo rail release --yes`.
When the `dry-run` input is `true`, it passes `--dry-run` instead of `--yes`.
When you run the Action again, it finishes an interrupted release.

- Before you upgrade, migrate a `.config/rail.toml` that has a `[release]` table.
  Follow [Upgrade from Cargo-Rail 0.31](https://github.com/loadingalias/cargo-rail/blob/main/docs/releases.md#upgrade-from-cargo-rail-031).
- Inputs are `crates`, `bump`, `pull-request`, `dry-run`, `token`, and `version`.
  This release removes the `packages`, `publish`, and `review` inputs.
- Outputs are `state`, `commit`, `tags` (a JSON array), and `pull-request`.
  This release removes the `transaction-id`, `release-sha`, and `run-url` outputs.
  A pending check on the commit being released fails the step.
- The values of `state` change.
  In v11, `state` was `active`, `complete`, or `aborted`.
  In v12, `state` is `nothing_to_release`, `planned`, `pull_request`, `awaiting_checks`, `awaiting_assets`, or `released`.
  Cargo-Rail also has `cancelled`, but it does not report it with the `--yes` or `--dry-run` that the Action passes.
  Update each workflow condition that compares `state`.
- The Action passes the `token` input to Cargo-Rail as `GH_TOKEN` and as process-scoped Git configuration.
  Cargo-Rail uses the token for the GitHub API and for Git access to `origin`, including the release push.
  Build scripts that run during packaging or API checks do not receive it.
  This requires Cargo-Rail 0.32.0, which v12 installs.
  The checkout keeps no credentials.
  The release commit uses the `github-actions[bot]` identity unless the checkout configures one.
- The new `release/asset` Action attests files and attaches them to the draft release that `cargo rail release` waits for.
- This release removes the release-record validator, the `release-record-v11` schema,
  and the reviewed-merge path through `pull_request_target`.
  Release workflows no longer pass `transaction`, `intent`, or `source` dispatch inputs.
- The `v{major}` alias follows each major release without a change to the configuration.
