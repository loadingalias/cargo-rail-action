# cargo-rail-action

[![Test](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml/badge.svg)](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Graph-aware change detection for Rust monorepos. **Test only what changed.**

- Detects which crates are affected by your changes (including transitive dependencies)
- Installs in ~3 seconds (pre-built binaries, no Rust toolchain needed)
- Works with PRs, push events, and manual runs

```yaml
- uses: loadingalias/cargo-rail-action@v1
```

## Quick Start

```yaml
name: CI
on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0  # Required for change detection

      - uses: loadingalias/cargo-rail-action@v1
        id: affected

      - name: Test affected crates
        if: steps.affected.outputs.count != '0'
        run: cargo test -p ${{ steps.affected.outputs.crates }}

      - name: Test all (infrastructure changed)
        if: steps.affected.outputs.rebuild-all == 'true'
        run: cargo test --workspace
```

## Parallel Testing (Matrix Strategy)

For large monorepos, run each crate in parallel:

```yaml
jobs:
  detect:
    runs-on: ubuntu-latest
    outputs:
      matrix: ${{ steps.affected.outputs.matrix }}
      count: ${{ steps.affected.outputs.count }}
      rebuild-all: ${{ steps.affected.outputs.rebuild-all }}
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - uses: loadingalias/cargo-rail-action@v1
        id: affected

  test:
    needs: detect
    if: needs.detect.outputs.count != '0' && needs.detect.outputs.rebuild-all != 'true'
    strategy:
      matrix:
        crate: ${{ fromJson(needs.detect.outputs.matrix) }}
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test -p ${{ matrix.crate }}

  test-all:
    needs: detect
    if: needs.detect.outputs.rebuild-all == 'true'
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test --workspace
```

## Inputs

| Input | Default | Description |
|-------|---------|-------------|
| `version` | `latest` | cargo-rail version to install |
| `since` | auto | Git ref to compare against (auto-detects PR base or `origin/main`) |
| `command` | `affected` | Command: `affected`, `test`, or `unify` |
| `args` | | Additional arguments passed to cargo-rail |
| `working-directory` | `.` | Directory containing Cargo.toml |

## Outputs

| Output | Description |
|--------|-------------|
| `crates` | Space-separated affected crates: `core api cli` |
| `matrix` | JSON array for strategy.matrix: `["core","api","cli"]` |
| `count` | Number of affected crates |
| `rebuild-all` | `true` if infrastructure files changed (Cargo.lock, CI, etc.) |
| `docs-only` | `true` if only documentation changed |

<details>
<summary>Additional outputs</summary>

| Output | Description |
|--------|-------------|
| `direct` | Crates with direct file changes |
| `transitive` | Crates affected through dependency graph |
| `changed-files` | Number of files changed |
| `infrastructure-files` | Files that triggered `rebuild-all` |
| `custom-categories` | Custom category matches from config |
| `install-method` | Installation method used (`binary`, `binstall`, `cargo-install`) |
| `cargo-rail-version` | Installed version |

</details>

## Configuration

Optional. Generate with `cargo rail init`:

```toml
# .config/rail.toml
[change-detection]
infrastructure = [".github/**", "Cargo.lock", "rust-toolchain.toml"]

[change-detection.custom]
benchmarks = ["benches/**"]
```

## More Examples

<details>
<summary>Run tests directly</summary>

```yaml
- uses: loadingalias/cargo-rail-action@v1
  with:
    command: test
```

</details>

<details>
<summary>Check dependency unification</summary>

```yaml
- uses: loadingalias/cargo-rail-action@v1
  with:
    command: unify
```

</details>

<details>
<summary>Custom base ref / subdirectory / pinned version</summary>

```yaml
- uses: loadingalias/cargo-rail-action@v1
  with:
    version: "0.1.0"
    since: origin/develop
    working-directory: rust/
```

</details>

## Links

- [cargo-rail](https://github.com/loadingalias/cargo-rail) — CLI tool
- [Configuration](https://github.com/loadingalias/cargo-rail/blob/main/docs/config.md)
- [Commands](https://github.com/loadingalias/cargo-rail/blob/main/docs/commands.md)

## License

MIT
