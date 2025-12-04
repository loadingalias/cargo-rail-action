# cargo-rail-action

Graph-aware change detection for Rust monorepos. **Test only what changed.**

```yaml
- uses: loadingalias/cargo-rail-action@v1
```

## Quick Start

### Minimal Setup

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
        id: rail

      - name: Test affected crates
        if: steps.rail.outputs.docs-only != 'true'
        run: |
          for crate in ${{ steps.rail.outputs.crates }}; do
            cargo test -p "$crate"
          done
```

### Matrix Strategy (Parallel Testing)

```yaml
name: CI
on: [push, pull_request]

jobs:
  detect:
    runs-on: ubuntu-latest
    outputs:
      matrix: ${{ steps.rail.outputs.matrix }}
      count: ${{ steps.rail.outputs.count }}
      docs-only: ${{ steps.rail.outputs.docs-only }}
      rebuild-all: ${{ steps.rail.outputs.rebuild-all }}
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - uses: loadingalias/cargo-rail-action@v1
        id: rail

  test:
    needs: detect
    if: needs.detect.outputs.docs-only != 'true' && needs.detect.outputs.count != '0'
    runs-on: ubuntu-latest
    strategy:
      fail-fast: false
      matrix:
        crate: ${{ fromJson(needs.detect.outputs.matrix) }}
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
| `version` | `latest` | cargo-rail version |
| `since` | auto | Base ref (auto-detects PR base or `origin/main`) |
| `command` | `affected` | `affected`, `test`, or `unify` |
| `args` | | Additional CLI arguments |
| `working-directory` | `.` | Path to Cargo.toml |

## Outputs

| Output | Description |
|--------|-------------|
| `crates` | Space-separated list: `core api cli` |
| `matrix` | JSON array for matrix strategy: `["core","api","cli"]` |
| `count` | Number of affected crates |
| `docs-only` | `true` if only docs changed (skip tests) |
| `rebuild-all` | `true` if infrastructure changed (test everything) |

<details>
<summary>All outputs</summary>

| Output | Description |
|--------|-------------|
| `direct` | Crates with direct changes |
| `transitive` | Crates affected via dependencies |
| `changed-files` | Number of files changed |
| `infrastructure-files` | Files that triggered rebuild-all |
| `custom-categories` | Custom category matches |
| `install-method` | How cargo-rail was installed |
| `cargo-rail-version` | Installed version |

</details>

## Configuration

Optional `rail.toml` for custom behavior (generate with `cargo rail init`):

```toml
[change-detection]
infrastructure = [".github/**", "Cargo.lock", "rust-toolchain.toml"]

[change-detection.custom]
benchmarks = ["benches/**"]
```

## More Examples

<details>
<summary>Run tests directly in action</summary>

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
<summary>Pin version / custom base ref / subdirectory</summary>

```yaml
- uses: loadingalias/cargo-rail-action@v1
  with:
    version: "0.5.0"
    since: origin/develop
    working-directory: rust/
```

</details>

## Requirements

`fetch-depth: 0` on checkout. No Rust toolchain needed (pre-built binaries).

## Links

- [cargo-rail repository](https://github.com/loadingalias/cargo-rail)
- [Configuration reference](https://github.com/loadingalias/cargo-rail/blob/main/docs/config.md)
- [Command reference](https://github.com/loadingalias/cargo-rail/blob/main/docs/commands.md)

## License

MIT
