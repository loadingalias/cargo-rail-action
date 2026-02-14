# cargo-rail-action

> Thin GitHub Action transport for `cargo rail plan -f github`.

[![Test](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml/badge.svg)](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE) 

## What It Does

- Runs `cargo rail plan ... -f github`.
- Publishes planner outputs for job gating.
- Keeps CI behavior aligned with local `plan` + `run`.

Minimum planner contract: `cargo-rail >= 0.10.0`.

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
        run: cargo rail run --since "${{ steps.rail.outputs.base-ref }}" --profile ci

      - name: Run docs pipeline
        if: steps.rail.outputs.docs == 'true'
        run: cargo rail run --since "${{ steps.rail.outputs.base-ref }}" --surface docs
```

## Inputs

| Input | Default | Description |
|---|---|---|
| `version` | `latest` | `cargo-rail` version to install |
| `checksum` | `required` | `required`, `if-available`, or `off` |
| `since` | auto | Git ref for planner comparison |
| `args` | `""` | Extra planner args (see below) |
| `working-directory` | `.` | Workspace directory |
| `token` | `${{ github.token }}` | Token for release download API |
| `mode` | `minimal` | `minimal` (recommended) or `full` |

**Optional `args` examples:**

```yaml
# Explain plan decisions in job logs
- uses: loadingalias/cargo-rail-action@v1
  with:
    args: '--explain'

# Custom format (default is github for this action)
- uses: loadingalias/cargo-rail-action@v1
  with:
    args: '-f json'

# Use different comparison ref
- uses: loadingalias/cargo-rail-action@v1
  with:
    since: 'origin/develop'
```

## Outputs

### What are modes and gates?

**Gates** are boolean outputs (`true`/`false`) used for job conditional execution. They answer: "should this CI surface run?"

**How gates work:**
1. The planner (`cargo rail plan`) analyzes changed files against your `.config/rail.toml` rules
2. It classifies changes into **surfaces**: `build`, `test`, `bench`, `docs`, `infra`
3. The action publishes these as GitHub Action outputs: `build=true`, `test=false`, etc.
4. You use these in job `if:` conditions to skip unnecessary work

**Why gates exist:**
- Deterministic: same plan locally and in CI (run `cargo rail plan --merge-base` to preview)
- Configurable: you define infrastructure files, doc-only paths, etc. in `rail.toml`
- Composable: combine gates (`if: steps.rail.outputs.build == 'true' || steps.rail.outputs.infra == 'true'`)

**Modes** control what outputs are published:
- `minimal` (default): only gates, matrix, base-ref, plan-json — optimized for modern workflows
- `full`: includes all minimal outputs plus legacy compatibility fields (counts, file lists, etc.)

### Minimal mode (default)

| Output | Type | Use |
|---|---|---|
| `build` | `true`/`false` | Gate build jobs — set when code changes affect compilation |
| `test` | `true`/`false` | Gate test jobs — set when changes affect test outcomes |
| `bench` | `true`/`false` | Gate benchmark jobs — set when changes affect performance tests |
| `docs` | `true`/`false` | Gate docs jobs — set for doc comments, README, markdown changes |
| `infra` | `true`/`false` | Gate infra jobs — set for CI config, scripts, toolchain changes |
| `matrix` | JSON | `strategy.matrix` for dynamic job matrices (crate-level parallelism) |
| `base-ref` | string | Git ref for downstream `run` calls (`--since "${{ steps.rail.outputs.base-ref }}"`) |
| `plan-json` | JSON | Full deterministic planner output (all surfaces, crates, files, metadata) |

### Full mode (`mode: full`)

Includes all minimal outputs plus additional fields for compatibility and debugging.

**When to use full mode:**
- Migrating from legacy workflows that parse file lists or crate counts
- Debugging plan decisions (use `trace` output)
- Building custom CI logic that needs raw file/crate lists

**Additional outputs in full mode:**

| Output | Type | Description |
|---|---|---|
| `count` | number | Total number of affected crates (for logging/metrics) |
| `crates` | string | Space-separated list of affected crate names |
| `files` | string | Newline-separated list of changed files (raw git diff output) |
| `surfaces` | string | Comma-separated list of enabled surface names (`build,test,docs`) |
| `trace` | JSON | Detailed trace of plan decisions (why each surface enabled/disabled) |
| `installed-version` | string | Actual `cargo-rail` version installed (useful when `version: latest`) |
| `installed-from` | string | Installation source (`cache`, `release`, `binstall`, or `cargo-install`) |
| `checksum-verified` | `true`/`false` | Whether binary checksum was verified against `SHA256SUMS` |

**Note:** Most workflows should use `minimal` mode. The `plan-json` output contains all plan data in structured form — use `jq` to extract what you need rather than relying on legacy string outputs.

## Security and Runtime

### Installation order and rationale

The action tries installation methods in this order:

1. **Cached binary** (fastest) — uses GitHub Actions cache (`actions/cache`)
   - Why: Subsequent runs on same runner are instant (no download)
   - Cache key includes version + platform, auto-invalidates on version change

2. **Release binary** (fast) — downloads from GitHub Releases
   - Why: Pre-built binaries for all supported platforms, verified against `SHA256SUMS`
   - Supports all platforms except macOS Intel (see below)

3. **`cargo-binstall`** (fallback) — installs via binstall if available
   - Why: Faster than `cargo install` (downloads binary instead of compiling)
   - Used when platform is unsupported for release binaries

4. **`cargo install`** (slowest, last resort) — compiles from source
   - Why: Guaranteed to work on any platform with Rust toolchain
   - Adds ~2-5 minutes to workflow run time

### Checksum verification

Defaults to `required` — downloads `SHA256SUMS` from release and verifies binary integrity.

**Why:** Supply chain security. Detects corrupted downloads and tampering.

**Options:**
- `required` (default): fail if checksum missing or mismatched
- `if-available`: verify if `SHA256SUMS` exists, skip if not (for pre-release testing)
- `off`: skip verification (not recommended for production)

### macOS Intel unsupported

macOS Intel (`x86_64-apple-darwin`) binaries are **not** published in releases.

**Why:** GitHub Actions deprecated `macos-latest` Intel runners in 2024. All macOS runners are now ARM (`macos-14`, `macos-15`). Publishing Intel binaries wastes release bandwidth for a platform no longer used in CI.

**Workaround:** The action falls back to `cargo-binstall` or `cargo install` on Intel Macs (primarily for local testing, not CI).

## Proven On Large Repos

Action + planner flow validated on production repos with real merge history:

| Repository | Crates | Validation Scenarios | Results |
|---|---|---|---|
| [tokio-rs/tokio](https://github.com/tokio-rs/tokio) | 10 | 5 merge commits | 60% avg surface reduction, docs-only detection working |
| [helix-editor/helix](https://github.com/helix-editor/helix) | 12 | 5 merge commits | 55% avg surface reduction, infra change detection accurate |
| [meilisearch/meilisearch](https://github.com/meilisearch/meilisearch) | 19 | 5 merge commits | 67% avg surface reduction, isolated infra-only changes detected |

**Aggregate results (15 merge scenarios):**
- **55% execution reduction** — fewer CI surfaces run per merge
- **64% weighted reduction** — compute units saved (accounts for surface cost: bench > test > build > docs)
- **710ms avg plan time** — deterministic plan generation (excludes git operations)
- **Quality audit:** 2 potential false-negatives, 1 potential false-positive (all flagged for review)

**Real-world benefits:**
- Docs-only PRs skip build/test entirely (run only docs checks)
- Infrastructure changes (CI config, scripts) trigger full rebuild (no false-negatives)
- Isolated crate changes run targeted tests (not entire workspace)
- Plan matches local behavior (`cargo rail plan --merge-base` previews CI gates)

Reproducible command matrix and examples live in `cargo-rail`:

- https://github.com/loadingalias/cargo-rail/blob/main/docs/large-repo-validation.md
- https://github.com/loadingalias/cargo-rail/tree/main/examples/change_detection

## Getting Help

- Action issues: [GitHub Issues](https://github.com/loadingalias/cargo-rail-action/issues)
- Core tool: [loadingalias/cargo-rail](https://github.com/loadingalias/cargo-rail)

## License

Licensed under [MIT](LICENSE).
