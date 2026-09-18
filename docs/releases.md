# Release Cargo-Rail Action

Release Cargo-Rail and Cargo-Rail Action independently.
Release Cargo-Rail first, then update one lock before releasing a changed Action.
A Cargo-Rail release does not require an Action release when the Action contract remains compatible.

## Version authority

| Fact                               | Owner                               | Consumer |
| ---------------------------------- | ----------------------------------- | -------- |
| Action package and runtime version | `Cargo.toml`                        | Bootstrap, runtime manifest, exact Action release |
| Tested Cargo-Rail release          | `.github/cargo-rail.lock` `version` | Package and release workflows |
| Matching release source            | `.github/cargo-rail.lock` `commit`  | Push and pull-request contract tests |
| CI tooling source                  | `.github/cargo-rail.lock` `tooling` | Package workflow tool installation |
| Public Cargo-Rail selection        | Action `version` input              | Calling workflow |
| Compatible Action alias            | `.config/rail.toml`                 | Cargo-Rail release publication |

The public Action defaults to the exact Cargo-Rail release in `.github/cargo-rail.lock`.
Publication uses the same locked release.
ASF consumers pin both the Action commit and Cargo-Rail version.

## Qualify a Cargo-Rail release

Complete these steps after Cargo-Rail publishes its immutable release.
They change local or repository source only; the verification commands do not publish anything.

1. Verify the exact Cargo-Rail release and a downloaded archive:

   ```bash
   gh release verify "v${CARGO_RAIL_VERSION}" --repo loadingalias/cargo-rail
   gh attestation verify "${CARGO_RAIL_ARCHIVE}" --repo loadingalias/cargo-rail
   ```

1. Set `version` and `commit` in `.github/cargo-rail.lock` to the exact Cargo-Rail release and its dereferenced tag
   commit. Set `tooling` independently to the reviewed Cargo-Rail commit that owns the Action's CI tool installation.

1. Validate the lock:

   ```bash
   bash scripts/read-cargo-rail-lock.sh
   ```

   The command prints one exact stable version, its full lowercase release commit SHA, and one full lowercase tooling
   commit SHA.

1. Run the complete local Action lane:

   ```bash
   just check
   just check-locked-cargo-rail \
     /absolute/path/to/cargo-rail \
     /absolute/path/to/cargo-rail-TARGET.zip
   ```

1. Push the reviewed lock update through normal CI.
   CI builds and checks the locked release source with tooling from the separately pinned tooling commit. It does not
   upload release assets.

## Release the Action

These steps publish external state.
Do not start them until the intended commit is on `main`, CI passed,
and the repository enforces immutable releases.

1. Open the `Release` workflow on `main` and leave all recovery inputs empty.

1. Approve the protected `release` environment after confirming the selected commit and lock.

1. Wait for Cargo-Rail to dispatch `Package` at the prepared release commit.
   The workflow downloads the exact locked Cargo-Rail release, runs the independent contracts,
   builds all runtimes on GitHub-hosted runners, and attests each executable.

1. Wait for Cargo-Rail to publish the complete immutable Action release and advance `v10`.
   Alias promotion occurs after the exact release succeeds.

1. Verify the release and every release asset:

   ```bash
   gh release verify "v${ACTION_VERSION}" --repo loadingalias/cargo-rail-action
   gh attestation verify cargo-rail-action-aarch64-apple-darwin \
     --repo loadingalias/cargo-rail-action
   gh attestation verify cargo-rail-action-aarch64-unknown-linux-gnu \
     --repo loadingalias/cargo-rail-action
   gh attestation verify cargo-rail-action-x86_64-pc-windows-msvc.exe \
     --repo loadingalias/cargo-rail-action
   gh attestation verify cargo-rail-action-x86_64-unknown-linux-gnu \
     --repo loadingalias/cargo-rail-action
   gh attestation verify cargo-rail-action-runtime-v1.tsv \
     --repo loadingalias/cargo-rail-action
   gh attestation verify LICENSE \
     --repo loadingalias/cargo-rail-action
   ```

1. Query the Cargo-Rail and Action release records.
   Continue to ASF preparation only when both report `immutable: true`.

## Resume a retained transaction

Do not start a second release after a partial publication.
Cargo-Rail retains the original transaction and its external-effect evidence.

1. Read the retained transaction ID, intent identity, and source commit from Cargo-Rail's failure
   output or release record.

1. Dispatch `Release` on `main` with all three recovery inputs.
   The Action rejects an incomplete or mismatched set before resuming publication.

1. Verify the exact release and runtimes after the resumed transaction completes.

Cargo-Rail refuses conflicting remote objects and does not repeat an uncertain publication blindly.
