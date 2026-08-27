# Cargo-Rail for GitHub Actions

**Plan less work. Reuse more compiler work.**

`cargo-rail-action` brings two independent Cargo-Rail capabilities to GitHub Actions:

| Action | What it removes |
|---|---|
| `loadingalias/cargo-rail-action` | Unaffected jobs, packages, targets, and matrix rows |
| `loadingalias/cargo-rail-action/cache` | Verified compiler work already completed in local or remote cache authority |

[![Test](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml/badge.svg)](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

The planner does not replace Cargo, nextest, Miri, Kani, Just, Make, Docker, or your CI commands. It creates one
validated decision file. Your existing command remains the execution authority.

## Plan Named Work

```yaml
- uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1

- uses: loadingalias/cargo-rail-action@v8
  id: rail
  with:
    version: 0.24.0

- name: Test affected packages
  if: contains(fromJSON(steps.rail.outputs.required-work), 'cargo.test')
  shell: bash
  env:
    PLAN_FILE: ${{ steps.rail.outputs.plan-file }}
    PLAN_READER: ${{ steps.rail.outputs.plan-reader }}
  run: |
    CARGO_ARGS=()
    while IFS= read -r -d '' arg; do CARGO_ARGS+=("$arg"); done \
      < <(python3 "$PLAN_READER" cargo-args "$PLAN_FILE" cargo.test)
    python3 "$PLAN_READER" verify-checkout "$PLAN_FILE"
    cargo nextest run "${CARGO_ARGS[@]}" --locked
```

The Action selects a pull request's merge base, a push event's previous commit, or an available default-branch merge
base; fetches missing shallow history; runs `cargo rail plan --json` once; validates the complete v8 identity and typed
contract; verifies the captured execution authority; and publishes a bounded summary. An all-zero push base safely
becomes `--all`.

For cross-job execution, export `required-work` from the planning job and upload `plan-file` plus `plan-reader` as one
artifact. Install the exact Cargo-Rail release used by the planning job in every consumer, download the artifact to a
fixed path, then run verification from the checked-out workspace root immediately before execution:

```bash
python3 .cargo-rail-plan/read.py verify-checkout .cargo-rail-plan/plan.json
```

The reader resolves `cargo-rail` from `PATH` (or `CARGO_RAIL_BIN`) and delegates complete execution-authority
verification to `cargo rail plan --verify`. It fails closed when the binary is unavailable, the verifier rejects the
plan, or the verifier emits unexpected stdout. Do not reconstruct package scope from changed paths or transfer a
second derived plan.

### Work model

Cargo-Rail owns the built-in Cargo decisions. A repository declares only the positive inputs for additional commands:

```toml
# .config/rail.toml
[plan.work.miri]
scope = "cargo"
cargo = ["cargo.test"]
paths = [".github/workflows/miri.yml", "scripts/miri/**"]

[plan.work.kani]
scope = "repository"
cargo = ["cargo.test"]
paths = [".github/workflows/kani.yml", "kani/**"]

[plan.work.benchmarks]
scope = "cargo"
cargo = ["cargo.test"]
paths = ["benches/**", "scripts/bench/**"]

[plan.work.container]
scope = "repository"
cargo = ["cargo.build"]
paths = ["Dockerfile", "docker/**"]
```

The distinction is deliberate:

- `scope = "cargo"` emits exact package and target selectors. Use it when the command accepts Cargo-style package
  selection, as `cargo test`, nextest, and Miri do.
- `scope = "repository"` is only a yes/no gate. Use it when a command is indivisible or does not accept the emitted
  selectors. Kani, a Docker build, or a legacy Make target can still avoid an unrelated job without pretending that
  Cargo-Rail can partially execute it.
- `scope = "variants"` selects checked-in matrix rows. It is for target/toolchain/feature/platform matrices, not shell
  commands.

Subscriptions such as `cargo = ["cargo.test"]` inherit that built-in's exact changed-input package scope. The
declaration remains command-free: flags, profiles, environment, timeouts, and tool installation stay in Just, Make,
scripts, or workflow YAML.

This covers mixed test runners naturally. Use `cargo.test` scope with nextest or `cargo test`; consume
`cargo.doctest` separately for `cargo test --doc`. Register Miri, Kani, benchmarks, profiling, generated-code checks,
or containers only when they have policy different from a built-in.

Markdown and arbitrary TOML do not trigger compilation merely because they changed. They trigger named work only
when a positive path declaration owns them, when Cargo-Rail understands a semantic Cargo/configuration change, or
when compatible compiler evidence proves the file is an observed input. `cargo.package` remains conservative over a
package's source tree because packaging owns those bytes.

### Planner outputs

| Output | Meaning |
|---|---|
| `required-work` | Compact JSON array used to route any built-in or repository-defined work ID |
| `plan-file` | Complete validated v8 plan for same-job use or artifact upload |
| `plan-reader` | Bundled strict consumer and final execution-authority verifier |
| `plan-identity` | Root-independent identity of the plan decisions |
| `base` | Authoritative comparison base recorded in the plan |
| `head-commit` | Commit component of the complete saved-plan authority binding |

The reader emits Cargo and target arguments as NUL-delimited argv. Never shell-split or `eval` them. `required-work`
is bounded routing data; the complete plan stays file-scoped to avoid GitHub output limits.

| Planner input | Default | Meaning |
|---|---|---|
| `version` | `0.24.0` | Exact compatible Cargo-Rail release |
| `components` | `core` | Verified native component set |
| `since` | event-aware | Explicit Git comparison ref |
| `all` | `false` | Require every registered item with complete scope |
| `evidence` | empty | Optional planning-evidence-v1 file |
| `working-directory` | `.` | Workspace directory |

Checksum verification is mandatory for downloaded release archives. The Action rejects floating versions and
incompatible planner output before publishing any plan outputs.

## Share Verified Compiler Work

```yaml
- uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1

- uses: loadingalias/cargo-rail-action/cache@v8
  id: cache
  with:
    version: 0.24.0
    url: ${{ vars.CARGO_RAIL_CACHE_URL }}
    mode: read

- run: cargo test --workspace --locked
```

`mode` is required. Use `read` in untrusted jobs. Grant `read-write` only to trusted cache-seeding jobs that cannot
execute unreviewed code. Omitting the input never grants remote publication authority.

The Action installs the authenticated cache component set, configures one bounded local L1 plus AWS S3, Cloudflare
R2, or Azure Blob Storage L2, then runs a network-free local status check. Later Cargo invocations in that job inherit
the setup, including Cargo launched by nextest, Just, or Make. Unsupported compiler shapes, incomplete observation,
provider failures, and rejected results fall back to normal compilation.

Host cache setup does not automatically enter `docker build`. Configure Cargo-Rail inside the container or explicitly
mount the required machine-owned state and credentials. Planning can still gate the Docker command independently.
Benchmark and profiling *results* are never compiler-cache objects; only eligible compilation leading to those runs
can be reused.

| Cache input | Default | Meaning |
|---|---|---|
| `url` | required | Secret-free S3, R2, or Azure cache authority |
| `mode` | required | Explicit `read` or `read-write` authority |
| `max-size` | `10GiB` | Job-local verified-cache bound |
| `local-dir` | Cargo home | Optional local-cache base directory |
| `version` | `0.24.0` | Exact Cargo-Rail release |

The cache Action publishes a small versioned `status-json` projection plus `healthy`, `provider`, `mode`, `activation`,
and `max-bytes`. Outputs and the job summary exclude the input URL, credentials, local paths, complete status document,
and cache object identities. A valid authenticated `complete` installation can satisfy a later `cache` request without
a second download.

For L1-only reuse inside one job, run `cargo rail cache setup` directly. See Cargo-Rail's
[planning](https://github.com/loadingalias/cargo-rail/blob/main/docs/planning.md),
[cache execution matrix](https://github.com/loadingalias/cargo-rail/blob/main/docs/caching.md#execution-and-reuse-support),
and [remote trust model](https://github.com/loadingalias/cargo-rail/blob/main/docs/cache-sharing.md).

## Compatibility and release

Action v8 installs Cargo-Rail 0.24.0 by default and rejects incompatible planner output before publishing outputs.
Use `@v8` for compatible Action fixes, or pin a full commit SHA for immutable execution. Core installation can fall
back to `cargo-binstall` or `cargo install --locked`; native component sets require a matching verified release
archive. The hosted release gate covers GNU Linux and Windows x86-64; qualify other published targets before relying
on them for a release.

## Project

- [Cargo-Rail](https://github.com/loadingalias/cargo-rail)
- [Action issues](https://github.com/loadingalias/cargo-rail-action/issues)
- [Core issues](https://github.com/loadingalias/cargo-rail/issues)
- [Contributing](CONTRIBUTING.md)
- [MIT license](LICENSE)
