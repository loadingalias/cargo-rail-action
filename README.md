# cargo-rail-action

Graph-aware change detection for Rust monorepos. This allows you to only test/bench what changed.

```yaml
- uses: loadingalias/cargo-rail-action@v1
```

## What It Does

1. Detects which files changed (via git diff)
2. Maps files to crates in your workspace
3. Computes transitive dependents
4. Outputs the minimal set of crates to test

**Result:** Significant reduction in CI time for monorepos

## Quick Start

```yaml
name: CI

on:
  pull_request:
  push:
    branches: [main]

jobs:
  detect:
    runs-on: ubuntu-latest
    outputs:
      matrix: ${{ steps.rail.outputs.matrix }}
      count: ${{ steps.rail.outputs.count }}
      docs-only: ${{ steps.rail.outputs.docs-only }}
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0  # Required for change detection

      - uses: loadingalias/cargo-rail-action@v1
        id: rail

  test:
    needs: detect
    if: needs.detect.outputs.docs-only != 'true' && needs.detect.outputs.count > 0
    runs-on: ubuntu-latest
    strategy:
      fail-fast: false
      matrix: ${{ fromJson(needs.detect.outputs.matrix) }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test -p ${{ matrix.crate }}
```

## Inputs

| Input | Description | Default |
|-------|-------------|---------|
| `version` | cargo-rail version to install | `latest` |
| `base` | Git ref to compare against | Auto-detects |
| `all` | Analyze all crates (ignore changes) | `false` |
| `working-directory` | Where Cargo.toml lives | `.` |

## Outputs

### Primary (for workflow logic)

| Output | Description | Example |
|--------|-------------|---------|
| `crates` | Space-separated affected crates | `core api cli` |
| `count` | Number of affected crates | `3` |
| `matrix` | JSON for `strategy.matrix` | `["core","api","cli"]` |

### Classification (for conditional jobs)

| Output | Description |
|--------|-------------|
| `docs-only` | `true` if only documentation changed |
| `rebuild-all` | `true` if infrastructure files changed |

### Detailed

| Output | Description |
|--------|-------------|
| `direct` | Crates with direct file changes |
| `transitive` | Crates affected via dependencies |
| `changed-files` | Number of files that changed |
| `infrastructure-files` | JSON array of infra files that changed |
| `custom-categories` | JSON object of custom category matches |

## Config

The action respects your `rail.toml` configuration. No duplication needed.

```toml
# rail.toml (in your repo)
[change-detection]
# Files that trigger full rebuild
infrastructure = [".github/**", "Dockerfile", "docker-compose.yaml"]

# Custom categories for conditional jobs
[change-detection.custom]
benchmarks = ["benches/**"]
verify = ["verify/**/*.rs"]
```

Then in your workflow:

```yaml
benchmark:
  needs: detect
  if: contains(fromJson(needs.detect.outputs.custom-categories), 'benchmarks')
  runs-on: ubuntu-latest
  steps:
    - run: cargo bench
```

## Examples

### Skip CI on Docs-Only Changes

```yaml
test:
  needs: detect
  if: needs.detect.outputs.docs-only != 'true'
  # ...
```

### Full Rebuild on Infra Changes

```yaml
jobs:
  detect:
    # ... outputs rebuild-all

  test-affected:
    needs: detect
    if: needs.detect.outputs.rebuild-all != 'true'
    strategy:
      matrix: ${{ fromJson(needs.detect.outputs.matrix) }}
    # ...

  test-all:
    needs: detect
    if: needs.detect.outputs.rebuild-all == 'true'
    steps:
      - run: cargo test --workspace
```

### Without Matrix

For simpler workflows, iterate over crates in a single job:

```yaml
test:
  needs: detect
  if: needs.detect.outputs.count > 0
  steps:
    - run: |
        for crate in ${{ needs.detect.outputs.crates }}; do
          cargo test -p "$crate"
        done
```

## Requirements

- `fetch-depth: 0` on checkout (for git history)
- Rust toolchain (action will `cargo install cargo-rail`)

## Links

- [cargo-rail](https://github.com/loadingalias/cargo-rail)
- [Config Reference](https://github.com/loadingalias/cargo-rail/blob/main/docs/config.md)
