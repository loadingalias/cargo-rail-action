# Cargo-Rail for GitHub Actions

[![Test](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml/badge.svg)](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

`cargo-rail-action` runs the Cargo-Rail planner once, validates its versioned contracts, and publishes affected
surfaces and Cargo package scope to later jobs. Package ownership and reverse-dependency impact come from Cargo's
resolved graph instead of path filters.

The planner action does not build, test, release, publish, or configure later jobs. Existing jobs keep their
toolchains, runners, matrices, and commands. The optional cache action installs verified compiler reuse in each
execution job that needs it.

## Quick start

This workflow handles pull requests and pushes. Pull requests use the PR base; pushes use the event's previous SHA.

```yaml
name: CI

on:
  push:
  pull_request:

permissions:
  contents: read

jobs:
  plan:
    name: Plan affected work
    runs-on: ubuntu-latest
    outputs:
      test: ${{ steps.rail.outputs.test }}
      cargo_args: ${{ steps.rail.outputs.cargo-args }}
    steps:
      - uses: actions/checkout@v7

      - uses: loadingalias/cargo-rail-action@v7
        id: rail
        with:
          version: 0.23.0
          # Push: compare with the previous SHA from the event.
          # Pull request: pass empty and let the action use the PR base.
          since: ${{ github.event_name == 'push' && github.event.before || '' }}

  test:
    name: Test affected packages
    needs: plan
    if: needs.plan.outputs.test == 'true'
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7

      - name: Run tests
        env:
          CARGO_ARGS: ${{ needs.plan.outputs.cargo_args }}
        run: |
          # Intentional word splitting: cargo-args contains Cargo-Rail-generated Cargo arguments.
          # shellcheck disable=SC2086
          cargo test $CARGO_ARGS
```

A shared-library change selects affected dependents. A docs-only change can skip package tests. Incomplete resolution
evidence widens scope. The action fetches missing comparison history when a shallow checkout lacks the selected base.

## Outputs

GitHub Actions outputs are strings. Compare convenience booleans with `'true'` when crossing job boundaries.

| Output | Meaning |
|---|---|
| `build`, `test`, `bench`, `docs`, `infra` | `'true'` or `'false'` for each built-in planner surface |
| `surfaces-json` | Boolean map containing every built-in and configured custom surface |
| `scope-json` | Versioned union execution scope across active package-scoped surfaces |
| `cargo-args` | Shell projection of that union scope: `--workspace`, one or more `-p <crate>` arguments, or an empty string |
| `base-ref` | Git ref used as the comparison base |
| `plan-file` | Path to the full planner contract for same-job consumers; published only with `mode: debug` |

Every invocation writes a job summary with the installed version, comparison base, changed-file count, scope mode,
direct and execution crates, active surfaces, top reasons, and a bounded trace preview.

### Scope semantics

`cargo-args` is the compatibility union of active package-scoped surfaces. It suits a combined build-and-test job, but
one surface can be narrower.

Use `mode: debug` and read `.surfaces.<name>.scope` from `plan-file` when a same-job task runner needs exact surface
scope. Use `scope` for execution and `impact` or `trace` for explanation. The full plan is file-scoped because it can
exceed process environment limits. Transfer `plan-file` as an artifact if another job needs it. See
[Planning and execution](https://github.com/loadingalias/cargo-rail/blob/main/docs/planning.md).

## Custom repository surfaces

Cargo-Rail can classify non-Cargo work without pretending path globs define package ownership.

```toml
# rail.toml
[change-detection.custom]
frontend = ["web/**"]
protos = ["proto/**"]
```

Export the complete map from the planner job:

```yaml
jobs:
  plan:
    runs-on: ubuntu-latest
    outputs:
      surfaces: ${{ steps.rail.outputs.surfaces-json }}
    steps:
      - uses: actions/checkout@v7
      - uses: loadingalias/cargo-rail-action@v7
        id: rail
        with:
          version: 0.23.0
          since: ${{ github.event_name == 'push' && github.event.before || '' }}

  frontend:
    needs: plan
    if: ${{ fromJSON(needs.plan.outputs.surfaces)['custom:frontend'] }}
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - run: npm test
```

Globs classify repository surfaces. Cargo package ownership and dependent impact still come from the resolved graph.

## Base-ref rules

The action selects the comparison base in this order:

1. explicit `since` input;
2. pull-request base as `origin/$GITHUB_BASE_REF`;
3. `origin/main`;
4. `origin/master`; or
5. `HEAD~1`.

For `push` workflows, pass `github.event.before` as shown above. Otherwise, the default branch can resolve to the
checked-out commit and produce an empty comparison.

GitHub uses an all-zero `before` SHA for some first-push and force-push cases. The action detects that value and falls back to automatic base selection.

## Inputs

| Input | Default | Meaning |
|---|---|---|
| `version` | `0.23.0` | Cargo-Rail release to install; `latest` explicitly opts into a floating core version |
| `checksum` | `required` | Release checksum policy: `required`, `if-available`, or `off` |
| `components` | `core` | Verified component set: `core`, `surface`, `distributed`, or `complete`; Surface selections run the exact-toolchain readiness preflight |
| `since` | automatic | Explicit Git comparison ref |
| `args` | `""` | Additional planner arguments; format and output overrides are rejected |
| `working-directory` | `.` | Directory containing the workspace `Cargo.toml` |
| `token` | `${{ github.token }}` | Token used to download release assets |
| `mode` | `minimal` | `minimal` or `debug`; legacy `full` maps to `debug` with a warning |

The planner needs only `core`. Select `surface` before a same-job `cargo rail surface` invocation, `distributed`
before configuring a distributed worker, or `complete` when the job needs every native capability. The installer
authenticates the archive and records the exact selected inventory; the next action run repairs a damaged partial
install. Selecting `surface` or `complete` makes the action run `cargo rail surface --prepare -f json` immediately
after installation. The preflight installs `rustc-dev` when absent and authenticates the driver for the workspace's
exact selected toolchain without changing the default toolchain, so the job fails before planning if that producer is
not ready.

## Compiler cache

Add the cache action to each execution job that should reuse compiler results:

```yaml
- uses: loadingalias/cargo-rail-action/cache@v7
  with:
    version: 0.23.0
    url: ${{ vars.CARGO_RAIL_CACHE_URL }}
    mode: read
```

Use `read-write` only in trusted jobs that cannot execute untrusted code. Configure provider credentials before the
action. Its `url` accepts an AWS `s3://`, Cloudflare `r2://`, or Azure `azure://` authority and contains no credentials.
Later Cargo commands need no cache arguments or wrapper command.

| Input | Default | Meaning |
|---|---|---|
| `url` | required | AWS S3, Azure Blob Storage, or Cloudflare R2 cache authority |
| `mode` | `read-write` | Maximum remote authority: `read` or `read-write` |
| `max-size` | `10GiB` | Positive binary size bound for the job-local verified cache |
| `local-dir` | Cargo home | Optional base directory for the job-local verified cache |
| `version` | `0.23.0` | Cargo-Rail release to install |
| `checksum` | `required` | Release checksum policy: `required`, `if-available`, or `off` |
| `token` | `${{ github.token }}` | Token used to download release assets |
| `working-directory` | `.` | Workspace directory used for setup |

Provide credentials through the provider's standard job environment. Limit them to the selected bucket, container,
or prefix. See
[Cache sharing](https://github.com/loadingalias/cargo-rail/blob/main/docs/cache-sharing.md) for provider permissions
and trust boundaries.

## Trust and compatibility

- Checksum verification is required by default.
- Installation tries an already matching binary, a release archive, `cargo-binstall`, then `cargo install --locked`.
- Planner and scope contracts are validated before outputs are published.
- Action major `v7` consumes planner contract `v7` and scope contract `v4`.
- Planner scopes include optional-feature and target-gated dependents; the action does not reinterpret them.
- The action and the installed Cargo-Rail version are selected independently.
- Release binaries support Linux and Windows on x86-64 and ARM64, plus macOS on ARM64.
- Additional planner arguments cannot override the action-owned output format or path.

Use `@v7` to follow compatible fixes within the action major. Pin a full commit SHA for immutable execution.

## Project

- [Cargo-Rail](https://github.com/loadingalias/cargo-rail)
- [Action Issues](https://github.com/loadingalias/cargo-rail-action/issues)
- [Core Issues](https://github.com/loadingalias/cargo-rail/issues)
- [Contributing](CONTRIBUTING.md)
- [MIT license](LICENSE)
