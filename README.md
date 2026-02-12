# cargo-rail-action

<p align="center">
  <img src="https://socialify.git.ci/loadingalias/cargo-rail-action/image?font=Jost&language=1&name=1&owner=1&pattern=Solid&theme=Auto" alt="cargo-rail-action" width="640" height="320" />
</p>

<p align="center">
  <a href="https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml"><img src="https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml/badge.svg" alt="Test"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

Planner-first GitHub Action for Rust monorepos.

This action is a thin transport layer over:

```bash
cargo rail plan --quiet --since "$BASE_REF" -f github
```

No action-side planning policy. It forwards planner outputs and adds pure projections for common CI usage.

Minimum supported planner contract: `cargo-rail >= 0.9.1` (enforced by the action).

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

      - name: Run tests for selected crates
        if: steps.rail.outputs.test == 'true'
        run: cargo rail test --since "${{ steps.rail.outputs.base-ref }}"

      - name: Run docs pipeline
        if: steps.rail.outputs.docs == 'true'
        run: cargo doc --workspace --no-deps
```

## Inputs

| Input | Default | Description |
|-------|---------|-------------|
| `version` | `latest` | cargo-rail version to install |
| `checksum` | `required` | Binary checksum mode: `required`, `if-available`, `off` |
| `since` | auto | Git ref to compare against |
| `args` | | Extra args passed to `cargo rail plan` |
| `working-directory` | `.` | Directory containing workspace root |
| `token` | `${{ github.token }}` | GitHub token for downloading releases |

## Outputs

### Planner-native

| Output | Description |
|--------|-------------|
| `files` | JSON array of changed files |
| `direct-crates` | Space-separated direct impacted crates |
| `transitive-crates` | Space-separated transitive impacted crates |
| `surfaces` | JSON object of surface decisions |
| `plan-json` | Full compact planner payload |
| `trace` | Planner trace payload |

### Surface projections

| Output | Description |
|--------|-------------|
| `build` | `true` when build surface is active |
| `test` | `true` when test surface is active |
| `bench` | `true` when bench surface is active |
| `docs` | `true` when docs surface is active |
| `infra` | `true` when infra surface is active |
| `custom-surfaces` | JSON map of custom surface booleans |
| `active-surfaces` | JSON array of active surfaces |

### Crate projections

| Output | Description |
|--------|-------------|
| `crates` | Space-separated impacted crates (direct + transitive) |
| `cargo-args` | `-p` flags derived from impacted crates |
| `count` | Impacted crate count |
| `matrix` | JSON array of impacted crates |
| `changed-files-count` | Number of changed files |

### Operational metadata

| Output | Description |
|--------|-------------|
| `base-ref` | Git ref used for comparison |
| `install-method` | `binary`, `binstall`, `cargo-install`, `cached` |
| `cargo-rail-version` | Installed cargo-rail version |

## Planner-first CI patterns

### Gate jobs by surface

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

## Migration from legacy coarse outputs

| Legacy | Planner-first replacement |
|--------|---------------------------|
| `docs-only` | `docs=true` and `test=false` |
| `rebuild-all` | `infra=true` |
| `custom-categories` | `custom-surfaces` |
| `cargo-args` from `affected` | `cargo-args` projection from `plan` |
| `command: affected|test` | Removed. Action always runs `plan`. |

## Supported runners

- Linux: x86_64, ARM64
- Windows: x86_64, ARM64
- macOS: ARM64 (Intel not supported)

## Security

When downloading binaries, checksum validation is enabled by default (`checksum: required`) using release `SHA256SUMS`.
