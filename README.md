# Cargo-Rail for GitHub Actions

**Run the planner once. Make every CI job do less.**

[![Test](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml/badge.svg)](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Rust CI often duplicates Cargo's package graph in path filters and package-selection scripts. `cargo-rail-action` installs Cargo-Rail, runs its read-only planner against one base ref, validates the planner contracts, and exports the affected surfaces and Cargo package scope.

**The action does not build, test, cache, release, or publish crates.** Keep those commands in their existing jobs and gate them with planner output. Your jobs keep their current toolchains, runners, caches, and test backends.

## Quick start

```yaml
name: CI
on: [push, pull_request]

jobs:
  plan:
    runs-on: ubuntu-latest
    outputs:
      test: ${{ steps.rail.outputs.test }}
      cargo_args: ${{ steps.rail.outputs.cargo-args }}
    steps:
      - uses: actions/checkout@v6

      - uses: loadingalias/cargo-rail-action@v6
        id: rail
        with:
          version: 0.19.1

  test:
    needs: plan
    if: needs.plan.outputs.test == 'true'
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - name: Test affected packages
        env:
          CARGO_ARGS: ${{ needs.plan.outputs.cargo_args }}
        run: cargo test $CARGO_ARGS
```

The action maps source changes through Cargo's resolved dependency graph. In a multi-crate workspace, scope contains the changed crates and affected dependents. In a single-crate repository, scope contains that crate or no Rust work. Documentation and infrastructure surfaces remain independent of package scope.

Use the floating `@v6` tag for the current action major, or pin a full commit SHA for immutable execution. The `version` input selects the installed `cargo-rail` release independently.

## Outputs

Minimal mode publishes:

| Output | Meaning |
|---|---|
| `build`, `test`, `bench`, `docs`, `infra` | Boolean built-in surface decisions |
| `surfaces-json` | Boolean map of every built-in and custom surface |
| `scope-json` | Versioned execution-scope handoff |
| `cargo-args` | Cargo projection: `--workspace`, one or more `-p <crate>` arguments, or an empty string |
| `base-ref` | Git ref used for the plan |

Debug mode also publishes `plan-json`, the full diagnostic planner payload. Use `scope-json` or `cargo-args` for execution. Use `plan-json` to inspect file classification, graph impact, and reason codes.

## Inputs

| Input | Default | Meaning |
|---|---|---|
| `version` | `0.19.1` | `cargo-rail` release to install; `latest` opts into a floating core version |
| `checksum` | `required` | Release checksum policy: `required`, `if-available`, or `off` |
| `since` | automatic | Explicit Git comparison ref |
| `args` | `""` | Additional planner arguments; format and output overrides are rejected |
| `working-directory` | `.` | Directory containing the workspace `Cargo.toml` |
| `token` | `${{ github.token }}` | Token used to download release assets |
| `mode` | `minimal` | `minimal` or `debug` output surface |

Without `since`, the action selects the pull-request base, `origin/main`, `origin/master`, or `HEAD~1`, in that order. If a shallow checkout lacks the selected ref, the action fetches the missing history before planning.

## Trust and compatibility

- Checksum verification is required by default.
- Installation tries an already matching binary, a release archive, `cargo-binstall`, then `cargo install`.
- The action validates `plan_contract_version` and `scope_contract_version` before publishing outputs.
- Action major v6 consumes planner contract v5 and scope contract v3.
- Linux x86-64/ARM64, Windows x86-64/ARM64, and Apple Silicon macOS use release binaries. Intel macOS is rejected.

The planner and scope contracts version independently. Diagnostic fields can evolve without changing the smaller execution handoff.

## Project

- [Cargo-Rail](https://github.com/loadingalias/cargo-rail)
- [Action issues](https://github.com/loadingalias/cargo-rail-action/issues)
- [Core issues](https://github.com/loadingalias/cargo-rail/issues)
- [Contributing](CONTRIBUTING.md)
- [MIT license](LICENSE)
