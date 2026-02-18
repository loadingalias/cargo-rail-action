# cargo-rail-action

> GitHub Action wrapper for `cargo rail plan -f github`.

[![Test](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml/badge.svg)](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE) 

## What It Does

- Runs `cargo rail plan ... -f github`.
- Publishes planner outputs for job gating.
- Keeps CI behavior aligned with local `plan` + `run` workflows.

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

      - uses: loadingalias/cargo-rail-action@v3
        id: rail
        with:
          version: "0.10.8"

      - name: Run targeted tests
        if: steps.rail.outputs.test == 'true'
        run: cargo rail run --since "${{ steps.rail.outputs.base-ref }}" --profile ci

      - name: Run docs pipeline
        if: steps.rail.outputs.docs == 'true'
        run: cargo rail run --since "${{ steps.rail.outputs.base-ref }}" --surface docs
```

Use the stable major tag `@v3`, or pin a commit SHA for maximum reproducibility. Pin `version` for deterministic
`cargo-rail` installs.

## Inputs

| Input | Default | Description |
|---|---|---|
| `version` | `0.10.8` | `cargo-rail` version to install (use `latest` only if you intentionally want floating upgrades) |
| `checksum` | `required` | `required`, `if-available`, or `off` |
| `since` | auto | Git ref for planner comparison |
| `args` | `""` | Extra planner args (see below) |
| `working-directory` | `.` | Workspace directory |
| `token` | `${{ github.token }}` | Token for release download API |
| `mode` | `minimal` | `minimal` (recommended) or `full` |

**Optional `args` examples:**

```yaml
# Explain plan decisions in job logs
- uses: loadingalias/cargo-rail-action@v3
  with:
    args: '--explain'

# Custom format (default is github for this action)
- uses: loadingalias/cargo-rail-action@v3
  with:
    args: '-f json'

# Use different comparison ref
- uses: loadingalias/cargo-rail-action@v3
  with:
    since: 'origin/develop'
```

## Outputs

### Modes and gates

**Gates** are boolean outputs (`true`/`false`) used in job conditions. They answer: "should this CI surface run?"

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
| `files` | JSON | JSON array of changed workspace-relative file paths |
| `surfaces` | JSON | JSON object of surface decisions (`enabled` + `reasons`) |
| `trace` | JSON | Detailed trace of plan decisions (why each surface enabled/disabled) |
| `install-method` | string | Installation source (`cached`, `binary`, `binstall`, `cargo-install`) |
| `cargo-rail-version` | string | Installed `cargo-rail` version |

**Note:** Most workflows should use `minimal` mode. The `plan-json` output contains all plan data in structured form — use `jq` to extract what you need rather than relying on legacy string outputs.

## Security and Runtime

### Installation order

The action tries installation methods in this order:

1. **Cached binary** (fastest) — uses GitHub Actions cache (`actions/cache`)
   - Why: Subsequent runs on same runner are instant (no download)
   - Cache reuse requires exact version match; when `version: latest`, the action resolves latest first, then compares

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

Default is `required`: download `SHA256SUMS` and verify binary integrity.

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

| Repository | Crates | Fork |
|---|---|---|
| [tokio-rs/tokio](https://github.com/tokio-rs/tokio) | 10 | [Config + Guide](https://github.com/loadingalias/cargo-rail-testing/tree/change-detection/tokio) |
| [helix-editor/helix](https://github.com/helix-editor/helix) | 14 | [Config + Guide](https://github.com/loadingalias/cargo-rail-testing/tree/change-detection/helix) |
| [meilisearch/meilisearch](https://github.com/meilisearch/meilisearch) | 23 | [Config + Guide](https://github.com/loadingalias/cargo-rail-testing/tree/change-detection/meilisearch) |
| [helixdb/helix-db](https://github.com/helixdb/helix-db) | 6 | [Config + Guide](https://github.com/loadingalias/cargo-rail-testing/tree/change-detection/helix-db) |

**Validation forks**: [cargo-rail-testing](https://github.com/loadingalias/cargo-rail-testing) — full configs, integration guides, and reproducible artifacts.

**Real-world benefits:**
- Docs-only PRs skip build/test entirely (run only docs checks)
- Infrastructure changes (CI config, scripts) trigger full rebuild (no false-negatives)
- Isolated crate changes run targeted tests (not entire workspace)
- Plan matches local behavior (`cargo rail plan --merge-base` previews CI gates)

**Measured impact (last 20 commits per repo):**

| Repository | Could Skip Build | Could Skip Tests | Targeted (Not Full Run) |
|---|---:|---:|---:|
| tokio | 10% | 0% | 95% |
| meilisearch | 35% | 35% | 60% |
| helix | 30% | 30% | 40% |
| helix-db | 10% | 10% | 75% |
| **Aggregate (80 commits)** | **21%** | **19%** | **68%** |

**Each fork includes:**
- `.config/rail.toml` — production-ready config with custom surfaces
- `docs/cargo-rail-integration-guide.md` — step-by-step CI integration with workflow examples
- `docs/CHANGE_DETECTION_METRICS.md` — measured impact analysis on recent commits

Reproducible command matrix and examples live in `cargo-rail`:

- https://github.com/loadingalias/cargo-rail/blob/main/examples/validation-protocol.md
- https://github.com/loadingalias/cargo-rail/tree/main/examples/change_detection

## Getting Help

- Action issues: [GitHub Issues](https://github.com/loadingalias/cargo-rail-action/issues)
- Core tool: [loadingalias/cargo-rail](https://github.com/loadingalias/cargo-rail)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## Security

See [SECURITY.md](SECURITY.md).

## Code of Conduct

See [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## License

Licensed under [MIT](LICENSE).
