# cargo-rail-action

> Thin GitHub Action transport for `cargo rail plan -f github`.

[![Test](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml/badge.svg)](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## What It Does

- Runs `cargo rail plan ... -f github`.
- Publishes planner outputs for job gating.
- Keeps CI behavior aligned with local `plan` + `run`.

Minimum planner contract: `cargo-rail >= 0.9.1`.

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
| `args` | `""` | Extra planner args |
| `working-directory` | `.` | Workspace directory |
| `token` | `${{ github.token }}` | Token for release download API |
| `mode` | `minimal` | `minimal` (recommended) or `full` |

## Outputs

### Minimal mode (default)

| Output | Use |
|---|---|
| `build` | Build gates |
| `test` | Test gates |
| `bench` | Benchmark gates |
| `docs` | Docs gates |
| `infra` | Infra/tooling gates |
| `matrix` | `strategy.matrix` JSON |
| `base-ref` | Baseline for downstream `run` calls |
| `plan-json` | Full deterministic planner output |

### Full mode (`mode: full`)

Includes all minimal outputs plus compatibility fields (`count`, `crates`, `files`, `surfaces`, `trace`, install metadata).

## Security and Runtime

- Install order: cached binary -> release binary -> `cargo-binstall` -> `cargo install`.
- Checksum verification defaults to `required` against release `SHA256SUMS`.
- macOS Intel (`macOS-x64`) is intentionally unsupported in this action.

## Proven On Large Repos

Action + planner flow validated on:

- [tokio-rs/tokio](https://github.com/tokio-rs/tokio)
- [helix-editor/helix](https://github.com/helix-editor/helix)
- [meilisearch/meilisearch](https://github.com/meilisearch/meilisearch)

Reproducible command matrix and examples live in `cargo-rail`:

- https://github.com/loadingalias/cargo-rail/blob/main/docs/large-repo-validation.md
- https://github.com/loadingalias/cargo-rail/tree/main/examples/change_detection

## Getting Help

- Action issues: [GitHub Issues](https://github.com/loadingalias/cargo-rail-action/issues)
- Core tool: [loadingalias/cargo-rail](https://github.com/loadingalias/cargo-rail)

## License

Licensed under [MIT](LICENSE).
