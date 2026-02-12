# cargo-rail-action

> Planner-first GitHub Action for Rust monorepos, built as a thin transport over `cargo rail plan`.

[![Test](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml/badge.svg)](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## What It Is

This action runs:

```bash
cargo rail plan ... -f github
```

Then publishes planner-native outputs plus deterministic convenience projections (crate matrix, cargo args, active surfaces, counts).

- No separate action-side planning policy.
- Same planning contract for local and CI usage.
- Minimum supported planner contract: `cargo-rail >= 0.10.0`.

## Quick Start

```yaml
name: CI
on: [push, pull_request]

jobs:
  plan:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - uses: loadingalias/cargo-rail-action@v1
        id: rail

      - name: Run targeted tests
        if: steps.rail.outputs.test == 'true'
        run: cargo rail test --since "${{ steps.rail.outputs.base-ref }}"

      - name: Run docs pipeline
        if: steps.rail.outputs.docs == 'true'
        run: cargo doc --workspace --no-deps
```

## Inputs

| Input | Default | Description |
|---|---|---|
| `version` | `latest` | `cargo-rail` version to install |
| `checksum` | `required` | Binary checksum mode: `required`, `if-available`, `off` |
| `since` | auto | Git ref for plan comparison |
| `args` | `""` | Extra args passed to planner |
| `working-directory` | `.` | Workspace directory |
| `token` | `${{ github.token }}` | Token for release download API |

## Outputs You'll Actually Use

### Surface gates

| Output | Use |
|---|---|
| `build` | Build jobs |
| `test` | Test jobs |
| `bench` | Benchmark jobs |
| `docs` | Docs jobs |
| `infra` | Infra/tooling jobs |
| `custom-surfaces` | Custom policy gates |

### Crate targeting

| Output | Use |
|---|---|
| `crates` | Space-separated impacted crates |
| `cargo-args` | `-p crate` flags |
| `matrix` | `strategy.matrix` JSON |
| `count` | Impacted crate count |
| `changed-files-count` | Changed file count |

### Planner-native contract

| Output | Description |
|---|---|
| `files` | JSON array of changed file paths |
| `direct-crates` | Directly impacted crates |
| `transitive-crates` | Transitively impacted crates |
| `surfaces` | Full surface decision object |
| `trace` | Planner trace payload |
| `plan-json` | Compact full planner payload |

### Operational metadata

| Output | Description |
|---|---|
| `base-ref` | Ref used for comparison |
| `install-method` | `binary`, `binstall`, `cargo-install`, `cached` |
| `cargo-rail-version` | Installed version |

## Common CI Patterns

### Gate jobs by planner surfaces

```yaml
- uses: loadingalias/cargo-rail-action@v1
  id: rail

- name: Build
  if: steps.rail.outputs.build == 'true'
  run: cargo build --workspace

- name: Bench
  if: steps.rail.outputs.bench == 'true'
  run: cargo bench --workspace
```

### Matrix over impacted crates

```yaml
jobs:
  detect:
    runs-on: ubuntu-latest
    outputs:
      matrix: ${{ steps.rail.outputs.matrix }}
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - uses: loadingalias/cargo-rail-action@v1
        id: rail

  test:
    needs: detect
    strategy:
      fail-fast: false
      matrix:
        crate: ${{ fromJson(needs.detect.outputs.matrix) }}
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo test -p "${{ matrix.crate }}"
```

## Runtime + Security Notes

- Install order: cached binary -> release binary download -> `cargo-binstall` -> `cargo install`.
- Checksum verification defaults to `required` and validates against release `SHA256SUMS`.
- macOS Intel (`macOS-x64`) is intentionally unsupported in this action.

## Compatibility

- Action target: composite GitHub Action for Rust monorepos
- Planner contract: `cargo-rail >= 0.9.1`
- Supported runners: Linux (x86_64/ARM64), Windows (x86_64/ARM64), macOS (ARM64)

## Development

Validation in this repo includes:

```bash
./tests/test_mapping.sh
```

CI workflow: [test.yaml](.github/workflows/test.yaml)

## Getting Help

- Action issues: [GitHub Issues](https://github.com/loadingalias/cargo-rail-action/issues)
- Core tool: [loadingalias/cargo-rail](https://github.com/loadingalias/cargo-rail)

## Contributing

PRs are welcome. If you change mappings or summary behavior, update fixtures and golden files in `tests/`.

## License

Licensed under [MIT](LICENSE).
