# cargo-rail-action

> Install cargo-rail, calculate the affected CI surfaces and Cargo packages, and expose that scope to later GitHub Actions steps or jobs.

[![Test](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml/badge.svg)](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## What the action does

The action runs `cargo rail plan` against the current checkout and a base Git ref. It publishes two kinds of output:

- Boolean surfaces such as `build`, `test`, and `docs` for job or step conditions.
- Execution scope as `scope-json` and `cargo-args`, so existing Cargo commands can target the affected workspace packages.

The action does not build, test, or publish crates. Keep those commands in your workflow and gate them with the planner outputs. This separates change detection from execution and lets each job use its existing toolchain, cache, runner, and test backend.

In a multi-crate workspace, changing a crate can select that crate and affected dependents. In a single-crate repository, package scope resolves to that package or no Rust work, while surface outputs can still distinguish source, documentation, and infrastructure changes.

## Quick Start

```yaml
name: CI
on: [push, pull_request]

jobs:
  plan:
    runs-on: ubuntu-latest
    outputs:
      build: ${{ steps.rail.outputs.build }}
      test: ${{ steps.rail.outputs.test }}
      cargo_args: ${{ steps.rail.outputs.cargo-args }}
    steps:
      - uses: actions/checkout@v6

      - uses: loadingalias/cargo-rail-action@v6.0.0
        id: rail
        with:
          version: 0.19.0

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

The default `version` is the cargo-rail release tested with this action major. Set `version` to test or hold another published release.

## Outputs

Minimal mode publishes:

- `build`
- `test`
- `bench`
- `docs`
- `infra`
- `scope-json`
- `surfaces-json`
- `cargo-args`
- `base-ref`

Debug mode adds:

- `plan-json`

`scope-json` is the stable execution handoff. `cargo-args` is its Cargo package selection: `--workspace`, one or more `-p <crate>` arguments, or an empty string. `surfaces-json` contains every built-in and custom surface as a boolean map. `plan-json` contains diagnostic detail and is available only in debug mode.

## Inputs

| Input | Default | Description |
|---|---|---|
| `version` | `0.19.0` | Published `cargo-rail` release tested by default; override only with a contract-compatible release |
| `checksum` | `required` | `required`, `if-available`, or `off` |
| `since` | auto | Git ref for planner comparison |
| `args` | `""` | Extra planner args except format/output flags |
| `working-directory` | `.` | Workspace directory |
| `token` | `${{ github.token }}` | Token for release download API |
| `mode` | `minimal` | `minimal` or `debug` |

## Compatibility

The action validates both planner contracts before publishing outputs.

- `plan_contract_version` covers the full diagnostic planner payload
- `scope_contract_version` covers the execution handoff payload

The contracts version independently. Diagnostic fields can be added to `plan-json` without changing the smaller execution handoff in `scope-json`.

## Operational behavior

- Checksum verification is on by default.
- The action fetches missing history when a shallow checkout does not contain the selected base ref.
- Installation tries a matching cached binary, a release archive, `cargo-binstall`, then `cargo install`.
- `@v6.0.0` supports cargo-rail v0.19, planner contract v5, and scope contract v3; pin its commit SHA for immutable action execution.
- Linux x86-64/ARM64, Windows x86-64/ARM64, and Apple Silicon macOS use published release binaries. Intel macOS is not supported.

## Getting Help

- Action Issues: [GitHub Issues](https://github.com/loadingalias/cargo-rail-action/issues)
- Core Issues: [loadingalias/cargo-rail](https://github.com/loadingalias/cargo-rail/issues)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## Security

See [SECURITY.md](SECURITY.md).

## License

Licensed under [MIT](LICENSE).
