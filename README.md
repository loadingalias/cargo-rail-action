# cargo-rail-action

<p align="center">
  <img src="https://socialify.git.ci/loadingalias/cargo-rail-action/image?font=Jost&language=1&name=1&owner=1&pattern=Solid&theme=Auto" alt="cargo-rail-action" width="640" height="320" />
</p>

<p align="center">
  <a href="https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml"><img src="https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml/badge.svg" alt="Test"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

<p align="center">
  <strong>Graph-aware change detection for Rust monorepos. Test only what changed.</strong>
</p>

---

## Why This vs Hand-Rolled Filters

- Graph-aware, not path-only: uses `cargo-rail`’s workspace graph instead of hard-coded `paths-filter` rules.
- Single source of truth: CI classification rules live in `rail.toml`, shared with the CLI.
- Shared engine: same logic as `cargo rail affected`/`cargo rail test`, so local runs match CI.

If this action saves you CI time or complexity, consider starring the main project: [cargo-rail](https://github.com/loadingalias/cargo-rail).

## What It Does

- Detects which crates are affected by your changes (including transitive dependencies)
- Installs in ~3 seconds (pre-built binaries, no Rust toolchain needed)
- Works with PRs, push events, and manual runs
- Outputs `docs-only` and `rebuild-all` flags for smart CI skipping

```yaml
- uses: loadingalias/cargo-rail-action@v1
```

---

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
        if: steps.affected.outputs.count != '0' && steps.affected.outputs.docs-only != 'true'
        run: cargo test -p ${{ steps.affected.outputs.crates }}

      - name: Test all (infrastructure changed)
        if: steps.affected.outputs.rebuild-all == 'true'
        run: cargo test --workspace
```

---

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
      docs-only: ${{ steps.affected.outputs.docs-only }}
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - uses: loadingalias/cargo-rail-action@v1
        id: affected

  test:
    needs: detect
    if: |
      needs.detect.outputs.count != '0' &&
      needs.detect.outputs.rebuild-all != 'true' &&
      needs.detect.outputs.docs-only != 'true'
    strategy:
      fail-fast: false
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

---

## Inputs

| Input | Default | Description |
|-------|---------|-------------|
| `version` | `latest` | cargo-rail version to install |
| `since` | auto | Git ref to compare against (auto-detects PR base or `origin/main`) |
| `command` | `affected` | Command to run: `affected` or `test` |
| `args` | | Additional arguments passed to cargo-rail |
| `working-directory` | `.` | Directory containing Cargo.toml |
| `token` | `${{ github.token }}` | GitHub token for downloading releases |

---

## Outputs

### Primary

| Output | Description |
|--------|-------------|
| `crates` | Space-separated affected crates: `core api cli` |
| `matrix` | JSON array for strategy.matrix: `["core","api","cli"]` |
| `count` | Number of affected crates |

### Classification Flags

| Output | Description |
|--------|-------------|
| `rebuild-all` | `true` if infrastructure files changed (Cargo.lock, CI, etc.) |
| `docs-only` | `true` if only documentation changed |

### Detailed Breakdown

<details>
<summary>Additional outputs</summary>

| Output | Description |
|--------|-------------|
| `direct` | Crates with direct file changes |
| `transitive` | Crates affected through dependency graph |
| `changed-files` | Number of files changed |
| `infrastructure-files` | JSON array of files that triggered `rebuild-all` |
| `custom-categories` | Custom category matches from rail.toml config |
| `install-method` | How cargo-rail was installed (`binary`, `binstall`, `cargo-install`, `cached`) |
| `cargo-rail-version` | Installed version |

</details>

---

## Configuration

Optional. Generate with `cargo rail init`:

```toml
# .config/rail.toml
[change-detection]
infrastructure = [".github/**", "Cargo.lock", "rust-toolchain.toml"]

[change-detection.custom]
benchmarks = ["benches/**"]
```

When infrastructure files change, `rebuild-all` is set to `true`. Use this to trigger full workspace tests.

---

## Examples

<details>
<summary>Skip CI on docs-only changes</summary>

```yaml
- uses: loadingalias/cargo-rail-action@v1
  id: affected

- name: Run tests
  if: steps.affected.outputs.docs-only != 'true'
  run: cargo test -p ${{ steps.affected.outputs.crates }}
```

</details>

<details>
<summary>Run tests directly (let cargo-rail handle it)</summary>

```yaml
- uses: loadingalias/cargo-rail-action@v1
  with:
    command: test
```

</details>

<details>
<summary>With cargo-nextest</summary>

```yaml
- uses: loadingalias/cargo-rail-action@v1
  id: affected

- name: Install nextest
  if: steps.affected.outputs.count != '0'
  uses: taiki-e/install-action@nextest

- name: Test affected
  if: steps.affected.outputs.count != '0' && steps.affected.outputs.docs-only != 'true'
  run: cargo nextest run -p ${{ steps.affected.outputs.crates }}
```

</details>

<details>
<summary>Custom base ref</summary>

```yaml
- uses: loadingalias/cargo-rail-action@v1
  with:
    since: origin/develop
```

</details>

<details>
<summary>Monorepo with Rust in subdirectory</summary>

```yaml
- uses: loadingalias/cargo-rail-action@v1
  with:
    working-directory: rust/
```

</details>

<details>
<summary>Pin to specific version</summary>

```yaml
- uses: loadingalias/cargo-rail-action@v1
  with:
    version: "0.2.0"
```

</details>

<details>
<summary>Migration from dorny/paths-filter</summary>

Before (hand-maintained paths):

```yaml
- uses: dorny/paths-filter@v3
  id: changes
  with:
    filters: |
      core:
        - "crates/core/**"
      api:
        - "crates/api/**"
      cli:
        - "crates/cli/**"
```

After (graph-aware, config in `rail.toml`):

```yaml
- uses: loadingalias/cargo-rail-action@v1
  id: affected

- name: Test affected crates
  if: steps.affected.outputs.count != '0' && steps.affected.outputs.docs-only != 'true'
  run: cargo test -p ${{ steps.affected.outputs.crates }}
```

This moves change classification into `rail.toml` and lets the dependency graph, not hand-written path lists, decide which crates are affected.

</details>

---

## How It Works

1. **Installs cargo-rail** — Downloads pre-built binary (fast) or falls back to `cargo install`
2. **Detects base ref** — Uses PR base, `origin/main`, or provided `since` input
3. **Runs change detection** — Maps changed files to crates via dependency graph
4. **Sets outputs** — Provides `crates`, `matrix`, `count`, `docs-only`, `rebuild-all`
5. **Writes summary** — Adds a summary to the GitHub Actions job

---

## Links

- [cargo-rail](https://github.com/loadingalias/cargo-rail) — The CLI tool
- [Configuration Reference](https://github.com/loadingalias/cargo-rail/blob/main/docs/config.md)
- [Command Reference](https://github.com/loadingalias/cargo-rail/blob/main/docs/commands.md)

---

## License

MIT
