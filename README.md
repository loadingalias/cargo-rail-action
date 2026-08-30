# Cargo-Rail for GitHub Actions

Use either action or both:

| Action | Removes |
|---|---|
| [Plan Work](#plan-work) | Unaffected jobs, packages, targets, and matrix rows |
| [Cache](#cache) | Compiler work already verified by Cargo-Rail |

## Plan Work

Add the planner before commands you want to gate:

```yaml
- uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1

- uses: loadingalias/cargo-rail-action@v8
  id: rail

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

`required-work` skips the step when `cargo.test` is unaffected. `cargo-args` emits exact NUL-delimited arguments.
`verify-checkout` rejects drift before Cargo runs. Never shell-split or `eval` reader output.

Use `cargo-scope PLAN WORK` before consuming package names. It emits exactly `skipped`, `workspace`, or `packages`.
`package-names PLAN WORK` emits canonical NUL-delimited names only for package scope and rejects ambiguous duplicate
names. It emits no bytes for skipped or workspace scope:

```bash
scope="$(python3 "$PLAN_READER" cargo-scope "$PLAN_FILE" cargo.test)"
if [[ "$scope" == packages ]]; then
  PACKAGES=()
  while IFS= read -r -d '' package; do PACKAGES+=("$package"); done \
    < <(python3 "$PLAN_READER" package-names "$PLAN_FILE" cargo.test)
fi
```

The action selects and fetches a safe Git base, runs `cargo rail plan --json` once, validates the v8 plan, then
publishes `required-work`, the exact plan, and its strict reader. An all-zero push base runs all work.

For a machine-contract boundary, pin this Action by full commit SHA. Its major tag is convenient but mutable. Use the
Action's exact default Cargo-Rail release, or set `version` to one exact compatible release; do not float the binary
independently of the bundled reader.

Cargo-Rail scopes work. Your existing commands execute it.

### Use the plan across jobs

1. Export `required-work` from the planning job.
2. Upload `plan-file` and `plan-reader` together.
3. Install the same Cargo-Rail version in each consumer job.
4. Download both files and verify from the workspace root before execution:

```bash
python3 .cargo-rail-plan/read.py verify-checkout .cargo-rail-plan/plan.json
```

Do not derive selectors from changed paths or transfer another plan.

### Add custom work

Built-in Cargo work needs no configuration. Add custom work only for repository-owned operations with distinct
triggers, such as Miri, Kani, benchmarks, generated-code checks, or containers.

| Scope | Use it for | Result |
|---|---|---|
| `cargo` | Commands that accept Cargo package selection | Exact package and target selectors |
| `repository` | Indivisible commands | A yes/no gate |
| `variants` | Checked-in CI matrices | Selected matrix rows |

Keep commands, flags, environment, timeouts, and setup outside `.config/rail.toml`.

See the complete [planner contract](action.yaml) and
[planning guide](https://github.com/loadingalias/cargo-rail/blob/main/docs/planning.md).

## Cache

Add the cache action after checkout and before Cargo:

```yaml
- uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1

- uses: loadingalias/cargo-rail-action/cache@v8
  with:
    url: ${{ vars.CARGO_RAIL_CACHE_URL }}
    mode: read
    root-portability: remap
    strict-probe: true

- run: cargo test --workspace --locked
```

Use `read` in untrusted jobs. Use `read-write` only in trusted cache-seeding jobs that cannot run unreviewed code.
`mode` is required. Keep credentials out of `url`. Root portability is typed: use `physical` for one checkout root or
`remap` for authenticated reuse across roots. `strict-probe: true` makes setup contact the provider and fail unless the
selected object store and Cargo-Rail protocol marker are ready. The v8 default, Cargo-Rail 0.25.0, provides that
strict probe contract.

The action installs authenticated cache components, configures a bounded local cache plus AWS S3, Cloudflare R2, or
Azure Blob Storage in one setup transaction, then validates local status. A requested strict probe reuses Cargo-Rail's
authenticated object-store and protocol-marker path. Later Cargo calls in the job inherit the setup. Unsupported work,
incomplete observation, provider failures, and rejected results compile normally.

Cache outputs expose only redacted health and policy fields. They omit the URL, credentials, local paths, full status,
and object identities. `remote-ready`, `protocol-marker`, and `probe-json` expose redacted readiness when strict
probing is enabled; `root-portability` reports the selected policy.

Host setup does not enter `docker build`; configure Cargo-Rail inside the container or mount the required state and
credentials explicitly.

For local-only reuse, run `cargo rail cache setup` directly.

See the complete [cache contract](cache/action.yaml) and
[caching guide](https://github.com/loadingalias/cargo-rail/blob/main/docs/caching.md).

## Compatibility

- Action v8 installs Cargo-Rail 0.25.0 by default and accepts only v8 plans.
- Use `@v8` for compatible fixes or a full commit SHA for immutable execution.
- Core installation can fall back to `cargo-binstall` or `cargo install --locked`.
- Cache and other native components require a matching verified release archive.
- Hosted release gates cover GNU Linux and Windows x86-64; qualify other targets separately.

## Support

- [Test workflow](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml)
- [Action issues](https://github.com/loadingalias/cargo-rail-action/issues)
- [Cargo-Rail issues](https://github.com/loadingalias/cargo-rail/issues)
- [Contributing](CONTRIBUTING.md)
- [MIT license](LICENSE)
