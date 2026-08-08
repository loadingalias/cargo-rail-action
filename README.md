# Cargo-Rail for GitHub Actions

**Plan Once. Gate Every Job. Pass (Cargo-Derived) Scope Forward.**

[![Test](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml/badge.svg)](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

GitHub Actions does not understand Cargo package ownership, active dependency edges, or reverse impact. We all compensate w/ path filters and package selection scripts, often with separate logic in each job.

That creates a shadow workspace model in YAML. It drifts from Cargo, disagrees across jobs, and turns every manifest or shared-crate change into a choice between overbuilding and undertesting.

`cargo-rail-action` runs the Cargo-Rail planner **once**, validates its versioned contracts, and publishes one decision for the rest of the workflow:

```text
┌───────────────────────────────┐
│ Git History + Cargo Workspace │
└───────────────┬───────────────┘
                ▼
       ┌───────────────────┐
       │ cargo-rail-action │
       └───────────┬───────┘
                   │
       ┌───────────┼───────────────┐
       ▼           ▼               ▼
┌────────────┐ ┌─────────────┐ ┌──────────────────┐
│  Surfaces  │ │ Cargo Scope │ │ Decision Summary │
└─────┬──────┘ └──────┬──────┘ └────────┬─────────┘
      └───────────────┴─────────────────┘
                      │
                      ▼
┌──────────────────────────────────────────────────────────┐
│ Existing CI jobs                                         │
│ Build · Test · Docs · Benchmarks · Lint · Task runner    │
└──────────────────────────────────────────────────────────┘
```

The action does **not** build, test, cache, release, or publish crates. That separation is deliberate. Your jobs keep their current toolchains, runners, caches, nextest configs, matrices, and task runners. The action replaces duplicated selection logic, not the execution stack that already works.

## Quick Start

This workflow handles pull requests and pushes. Pull requests use the PR base branch. Pushes compare against the event's previous SHA.

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

      - uses: loadingalias/cargo-rail-action@v6
        id: rail
        with:
          version: 0.20.1
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

The planner maps changed source through Cargo ownership and active reverse dependencies. A shared library change selects affected dependents. A docs-only change can skip package tests. A semantic manifest change can localize work or widen to the workspace when resolution evidence is incomplete.

The downstream job consumes that decision instead of reimplementing it.

## What One Planner Job Buys You

| Before | After |
|---|---|
| Every job owns path filters and base-ref logic | One job owns the comparison and publishes a validated contract |
| Package scope is reconstructed from directory names | Package scope comes from Cargo's resolved graph |
| Shared-crate impact is maintained by hand | Reverse-dep impact is computed from active edges |
| A skipped job is hard to explain | The job summary shows active surfaces, scope, and stable reasons |
| Changing selection logic means rewriting execution | Existing jobs keep their commands and consume planner outputs |

The action fetches missing comparison history when a shallow checkout does not contain the selected base. You do not need to make every checkout full-depth merely to plan affected work.

## Outputs

GitHub Actions outputs are strings. Compare convenience booleans with `'true'` when crossing job boundaries.

| Output | Meaning |
|---|---|
| `build`, `test`, `bench`, `docs`, `infra` | `'true'` or `'false'` for each built-in planner surface |
| `surfaces-json` | Boolean map containing every built-in and configured custom surface |
| `scope-json` | Versioned union execution scope across active package-scoped surfaces |
| `cargo-args` | Shell projection of that union scope: `--workspace`, one or more `-p <crate>` arguments, or an empty string |
| `base-ref` | Git ref used as the comparison base |
| `plan-json` | Full diagnostic planner payload; published only with `mode: debug` |

Every invocation writes a GitHub job summary with the installed version, comparison base, changed-file count, scope mode, direct and execution crates, active surfaces, top reasons, and a bounded trace preview.

### Scope Semantics

`cargo-args` is the conservative compat union of all active package-scoped surfaces. That is correct for a combined build-and-test job, but an individual surface can be narrower.

Use `mode: debug` and consume `plan-json` at `.surfaces.<name>.scope` when a task runner needs the exact scope for one surface. Use `scope` for execution; use `impact` and `trace` to explain the decision. See [Planning and execution](https://github.com/loadingalias/cargo-rail/blob/main/docs/planning.md).

## Custom Repo Surfaces

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
      - uses: loadingalias/cargo-rail-action@v6
        id: rail
        with:
          version: 0.20.1
          since: ${{ github.event_name == 'push' && github.event.before || '' }}

  frontend:
    needs: plan
    if: ${{ fromJSON(needs.plan.outputs.surfaces)['custom:frontend'] }}
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - run: npm test
```

Globs classify repository surfaces. Cargo package ownership and dependent impact still come from the resolved workspace graph.

## Base-Ref Rules

The action selects the comparison base in this order:

1. explicit `since` input;
2. pull-request base as `origin/$GITHUB_BASE_REF`;
3. `origin/main`;
4. `origin/master`; or
5. `HEAD~1`.

For workflows that run on `push`, pass `github.event.before` as shown above. Otherwise, a push to the default branch can resolve `origin/main` or `origin/master` to the checked-out commit and produce an empty comparison.

GitHub uses an all-zero `before` SHA for some first-push and force-push cases. The action detects that value and falls back to automatic base selection.

## Inputs

| Input | Default | Meaning |
|---|---|---|
| `version` | `0.20.1` | Cargo-Rail release to install; `latest` explicitly opts into a floating core version |
| `checksum` | `required` | Release checksum policy: `required`, `if-available`, or `off` |
| `since` | automatic | Explicit Git comparison ref |
| `args` | `""` | Additional planner arguments; format and output overrides are rejected |
| `working-directory` | `.` | Directory containing the workspace `Cargo.toml` |
| `token` | `${{ github.token }}` | Token used to download release assets |
| `mode` | `minimal` | `minimal` or `debug`; legacy `full` maps to `debug` with a warning |

## Trust and Compat

- Checksum verification is required by default.
- Installation tries an already matching binary, a release archive, `cargo-binstall`, then `cargo install --locked`.
- Planner and scope contracts are validated before outputs are published.
- Action major `v6` consumes planner contract `v5` and scope contract `v3`.
- The action and the installed Cargo-Rail version are selected independently.
- Release binaries support Linux, Windows, and macOS on x86-64 and ARM64.
- Additional planner arguments cannot override the action-owned output format or path.

Use the floating `@v6` tag to follow compatible fixes within the current action major. Pin a full commit SHA when immutable third-party action execution is required.

## Project

- [Cargo-Rail](https://github.com/loadingalias/cargo-rail)
- [Action Issues](https://github.com/loadingalias/cargo-rail-action/issues)
- [Core Issues](https://github.com/loadingalias/cargo-rail/issues)
- [Contributing](CONTRIBUTING.md)
- [MIT license](LICENSE)
