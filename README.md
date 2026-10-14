# Cargo-Rail Action

Cargo-Rail Action adds Cargo-Rail planning, verified compiler reuse,
and one-command releases to GitHub Actions.
It installs an exact Cargo-Rail release.
It validates each plan, cache, and release boundary before use.
Cargo and the workflow still run the work.

## Reduce work at each layer

| Action  | Resource effect |
| ------- | --------------- |
| Planner | Lets the workflow skip jobs that are not required and gives required jobs exact Cargo selectors. |
| Cache   | Restores compatible compiler results inside the jobs that still run. |
| Release | Releases the commit that CI already checked, without running CI again; a rerun finishes an interrupted release. |

```text
all declared workflow jobs
└─ planner keeps required jobs and emits exact Cargo scope
   └─ cache restores compatible compiler results in those jobs
      └─ Cargo runs freshness checks and the remaining misses
```

Planning and caching solve different problems, so their reductions add together.
Start with the planner alone.
Add caching when the job has explicit remote authority and credentials.
Add the release Action only to a protected publication job.
Use the plan summary and the cache report to see the actual result.
The Action does not estimate the time saved.

## Plan and run work in one job

Before these steps, install your Rust toolchain and command runners, such as nextest.

```yaml
- uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
  with:
    persist-credentials: false

- uses: loadingalias/cargo-rail-action@v12
  id: rail

- name: Run selected tests
  shell: bash
  if: contains(fromJSON(steps.rail.outputs.required-work), 'cargo.test')
  env:
    PLAN_FILE: ${{ steps.rail.outputs.plan-file }}
  run: |
    ARGS_FILE="$(mktemp "$RUNNER_TEMP/cargo-rail-args.XXXXXX")"
    cargo-rail-action plan cargo-args "$PLAN_FILE" cargo.test > "$ARGS_FILE" || exit "$?"
    CARGO_ARGS=()
    while IFS= read -r -d '' argument; do CARGO_ARGS+=("$argument"); done < "$ARGS_FILE"
    rm -- "$ARGS_FILE"
    cargo nextest run "${CARGO_ARGS[@]}" --locked
```

Before a selector writes to stdout,
it validates the complete plan and recomputes the canonical identity of the plan.
The selector also asks the exact installed Cargo-Rail to verify the current checkout.
A repository change after the selector output and
before the consuming command is still a caller error.
The planner publishes `cargo-rail` and the stable `cargo-rail-action` launcher to later steps in the same job.

The planner publishes only:

- `version`: the exact installed Cargo-Rail version;
- `plan-file`: the absolute validated plan path; and
- `required-work`: a compact JSON array of required work IDs.

See the [planner inputs](action.yaml) for comparison refs, full verification, evidence, and workspace selection.

When planning needs Surface, set `components` to `surface` or `complete`.
During explicit Surface preparation, Cargo-Rail can install `rustc-dev`.
It can also compile its compiler fact driver, which is authenticated and bound to the toolchain.
When the development components for the selected compiler are already installed,
native cache preparation can also compile that driver from the authenticated source package.
Release mode does not compile the Action runtime or Cargo-Rail itself in the workflow.

## Transfer a plan across jobs

Transfer only `plan.json`.
Install the required Rust toolchain and command runners in each job.
Use the setup action to install the same Cargo-Rail version in the consuming job:

```yaml
jobs:
  plan:
    runs-on: ubuntu-latest
    outputs:
      required-work: ${{ steps.rail.outputs.required-work }}
      cargo-rail-version: ${{ steps.rail.outputs.version }}
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - uses: loadingalias/cargo-rail-action@v12
        id: rail
      - uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7.0.1
        with:
          name: cargo-rail-plan
          path: ${{ steps.rail.outputs.plan-file }}
          if-no-files-found: error

  test:
    needs: plan
    if: contains(fromJSON(needs.plan.outputs.required-work), 'cargo.test')
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - uses: actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c # v8.0.1
        with:
          name: cargo-rail-plan
          path: ${{ runner.temp }}/cargo-rail-plan
      - uses: loadingalias/cargo-rail-action/setup@v12
        with:
          version: ${{ needs.plan.outputs.cargo-rail-version }}
      - name: Run selected tests
        shell: bash
        env:
          PLAN_FILE: ${{ runner.temp }}/cargo-rail-plan/plan.json
        run: |
          ARGS_FILE="$(mktemp "$RUNNER_TEMP/cargo-rail-args.XXXXXX")"
          cargo-rail-action plan cargo-args "$PLAN_FILE" cargo.test > "$ARGS_FILE" || exit "$?"
          CARGO_ARGS=()
          while IFS= read -r -d '' argument; do CARGO_ARGS+=("$argument"); done < "$ARGS_FILE"
          rm -- "$ARGS_FILE"
          cargo nextest run "${CARGO_ARGS[@]}" --locked
```

Keep the downloaded plan outside the checkout.
Use the same source, Rust toolchain, platform, and relative workspace directory as the planning job.
Run selectors from that workspace directory.
Checkout verification rejects source drift.
When the workflow uses more than one platform, create a separate plan for each platform.

## Group work and route other platforms

These jobs use the `plan` job from the example above.
A grouped job runs when any work that it owns is required.
Guard each command with its own work ID.
Repository work, such as `cargo.fmt`, can route the job.
However, repository work has no Cargo selector.
The `required-work` output of the Linux plan can route a macOS job.
Only a plan created on macOS authorizes macOS selectors.
For this reason, the macOS job plans again with the same Cargo-Rail version:

```yaml
  validate:
    needs: plan
    if: >-
      contains(fromJSON(needs.plan.outputs.required-work), 'cargo.fmt') ||
      contains(fromJSON(needs.plan.outputs.required-work), 'cargo.test')
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - uses: actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c # v8.0.1
        with:
          name: cargo-rail-plan
          path: ${{ runner.temp }}/cargo-rail-plan
      - uses: loadingalias/cargo-rail-action/setup@v12
        with:
          version: ${{ needs.plan.outputs.cargo-rail-version }}
      - name: Check formatting
        if: contains(fromJSON(needs.plan.outputs.required-work), 'cargo.fmt')
        run: cargo fmt --all --check
      - name: Run selected tests
        if: contains(fromJSON(needs.plan.outputs.required-work), 'cargo.test')
        shell: bash
        env:
          PLAN_FILE: ${{ runner.temp }}/cargo-rail-plan/plan.json
        run: |
          ARGS_FILE="$(mktemp "$RUNNER_TEMP/cargo-rail-args.XXXXXX")"
          cargo-rail-action plan cargo-args "$PLAN_FILE" cargo.test > "$ARGS_FILE" || exit "$?"
          CARGO_ARGS=()
          while IFS= read -r -d '' argument; do CARGO_ARGS+=("$argument"); done < "$ARGS_FILE"
          rm -- "$ARGS_FILE"
          cargo nextest run "${CARGO_ARGS[@]}" --locked

  macos-test:
    needs: plan
    if: contains(fromJSON(needs.plan.outputs.required-work), 'cargo.test')
    runs-on: macos-latest
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - uses: loadingalias/cargo-rail-action@v12
        id: rail
        with:
          version: ${{ needs.plan.outputs.cargo-rail-version }}
      - name: Run selected tests
        if: contains(fromJSON(steps.rail.outputs.required-work), 'cargo.test')
        shell: bash
        env:
          PLAN_FILE: ${{ steps.rail.outputs.plan-file }}
        run: |
          ARGS_FILE="$(mktemp "$RUNNER_TEMP/cargo-rail-args.XXXXXX")"
          cargo-rail-action plan cargo-args "$PLAN_FILE" cargo.test > "$ARGS_FILE" || exit "$?"
          CARGO_ARGS=()
          while IFS= read -r -d '' argument; do CARGO_ARGS+=("$argument"); done < "$ARGS_FILE"
          rm -- "$ARGS_FILE"
          cargo nextest run "${CARGO_ARGS[@]}" --locked
```

The macOS step routes on its own plan,
because its decision can be different from the decision of the Linux plan.
A reader rejects a plan from another platform before checkout verification.
When the commands of a grouped job have different inputs, setup cost, or failure ownership,
split the job.

## Run Cargo-Rail checks with setup only

When a job runs Cargo-Rail commands without the planner, use `setup`:

```yaml
- uses: loadingalias/cargo-rail-action/setup@v12
  id: cargo-rail
- run: cargo rail unify --check
```

Setup installs authenticated prebuilt components.
It does not compile Rust code.
Thus, your workspace toolchain and MSRV stay independent of Cargo-Rail's build version.
The `version` input defaults to the release in the `.github/cargo-rail.lock` file of this Action.
The action metadata and the lock always agree.
The step logs `Cargo-Rail setup ready: VERSION` and publishes the same value as its `version` output.

Install that version locally with one command, then check it:

```bash
cargo install cargo-rail --locked --version VERSION
cargo rail --version
```

A source install needs Cargo-Rail's `rust-version`, not your workspace toolchain.
See [Cargo-Rail installation paths](https://github.com/loadingalias/cargo-rail#installation-paths) for archives and compiler components.

A repository wrapper can skip a check when Cargo-Rail is missing.
This makes local runs easier, but policy drift then appears first in CI.
Install the matching version.
Do not rely on the skip.

Before you make `cargo rail unify --check` a required check, get a clean baseline:

1. Review the proposed edits.
1. Apply them in one reviewed change.
1. Make sure that `--check` passes on the default branch.

Do not keep an optional check that always fails.
Reviewers learn to ignore such a check.

## Migrate from an earlier version

Before it plans, the planner audits every tracked or unignored YAML file in the repository
for Cargo-Rail Action references.
It prints each reference with its version, pin, job, and inputs.
It also prints the job conditions that read Cargo-Rail outputs.
These errors stop the planner before any job runs:

- a reference to an earlier major version, so a partial migration cannot run;
- an input, output, or action path that the current release does not provide,
  such as a Boolean output from v7 or earlier;
- a `needs.JOB.outputs.NAME` that the planning job does not export, which GitHub evaluates as empty;
- a file that mentions the Action but is not valid YAML.

Warnings annotate the file and line for these conditions:

- `plan-file` exported as a job output, because it is a path on one runner;
- a job that reads selectors from a plan made on another runner label;
- a pin whose release the audit cannot read.
  To prevent this warning, add a `# vX.Y.Z` comment to a commit pin.

Run the same audit locally from the release that you migrate to:

```bash
cargo install --locked --git https://github.com/loadingalias/cargo-rail-action --tag vX.Y.Z cargo-rail-action
cargo-rail-action audit
```

Migrate in this order.
Each step proves one claim:

1. Run `cargo rail config validate --strict`.
   It proves that the policy is valid for the Cargo graph of the workspace.
1. Run `cargo-rail-action audit`.
   It proves that every workflow reference and output consumer matches this release.
1. Before you remove the previous selector, run `cargo rail plan --cases routes.toml`.
   It proves that reviewed path changes route as expected.
   See [route parity](https://github.com/loadingalias/cargo-rail/blob/main/docs/planning.md#check-route-parity-before-a-migration).
1. Run `cargo rail plan --all --json`.
   It proves that every work item has valid full scope.
   It does not prove a routing decision.

## Read selectors

The direct consumer surface is small by design:

```text
cargo-rail-action plan summary PLAN
cargo-rail-action plan required PLAN
cargo-rail-action plan is-required PLAN WORK
cargo-rail-action plan cargo-args PLAN WORK
cargo-rail-action plan cargo-scope PLAN WORK
cargo-rail-action plan package-names PLAN WORK
cargo-rail-action plan target-args PLAN WORK
cargo-rail-action plan matrix PLAN WORK [--family FAMILY]
```

`summary` emits readable Markdown.
Line-oriented selectors emit one compact value and a newline.
Argument and package selectors emit NUL-delimited values.
`target-args` emits `--test NAME` pairs only when every selected target is an integration test.
It emits nothing for an empty, mixed, or unsupported target set.
Cargo then runs the selected packages without unsafe narrowing.
`matrix` emits an `include` object for selected rows.
For unrestricted variant scope, it emits the literal `all`.
Before you call `fromJSON`, check for `all`.
If the value is `all`, use the complete checked-in catalog of the work item.
`--family FAMILY` filters selected rows and nests each row under that family name.
It does not expand `all`.
Skipped variant work emits `{"include":[]}`.
When no rows are selected, skip the consuming job.

Contract rejections exit with code `2`.
A rejected plan selector writes nothing to stdout.
Operational failures, including I/O failures and failed subprocesses, exit with code `1`.

## Configure compiler caching

```yaml
- uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
  with:
    persist-credentials: false

- uses: loadingalias/cargo-rail-action/cache@v12
  id: cache
  with:
    remote: s3://cargo-rail-cache/team?region=us-east-1&owner=123456789012
    mode: read
    max-size: 10GiB
    root-portability: physical
    verify-remote: false
```

When one job uses both actions, configure `loadingalias/cargo-rail-action/cache` before `loadingalias/cargo-rail-action`.
Cache setup changes the Cargo configuration.
Plan verification binds that configuration.
If the planner captures the plan first, later selectors reject the changed execution authority.

`mode` is always explicit.
Give remote credentials only to trusted jobs.
A job that runs untrusted code, such as a pull request from a fork, gets no credentials.
That job also does not run this action.
This action always selects a remote.
Read access does not protect confidentiality.
Any job that can read the remote cache can read every result in it.
Use `read` for trusted jobs that must not publish.
Use `read-write` only for trusted jobs that seed the cache.
With `verify-remote: true`, the action authenticates to the selected provider and requires the protocol marker
before it publishes Action outputs.
See the [cache inputs](cache/action.yaml) for the local cache location and workspace selection.

The cache installer requires these components:

- the core executable;
- the native wrapper and worker;
- the matched compiler driver;
- the authenticated driver source package.

The installer rejects an archive or an installation receipt
that does not have both driver components.
Every component set also requires the authenticated `LICENSE` entry, and the installation keeps that entry.
Cargo-Rail owns toolchain matching and cache preparation.

The cache action publishes only `version` and one compact `status` value.
The `status` value conforms to [`schemas/cache-status-v1.schema.json`](schemas/cache-status-v1.schema.json).
It contains the provider, the mode, the local byte bound, the root portability,
and whether remote verification was requested and passed.
It never contains the remote URL, credentials, local paths, the authority identity,
the protocol marker, or component receipts.

## One cache report for the workflow

Configure caching once in each participating job.
Collect the record after the final Cargo command of each job.
Also collect it on failure paths.
The setup and collection actions publish no summaries.
The final report combines all jobs into one summary.

```yaml
# Append these steps to each cache-enabled job.
- uses: loadingalias/cargo-rail-action/cache/collect@v12
  if: always()
  id: cache-record
  with:
    job: ${{ matrix.runner }}
    output-directory: ${{ runner.temp }}/cache-records
- uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7.0.1
  if: always() && steps.cache-record.outcome == 'success'
  with:
    name: cache-record-${{ matrix.runner }}
    path: ${{ steps.cache-record.outputs.record-file }}
    if-no-files-found: error
```

Use a unique `job` label for each matrix row.
A label contains 1–128 ASCII letters, digits, dots, hyphens, or underscores.
When a runner expression contains other characters, use a separate label.
For a job without a matrix, use its workflow job ID.
Keep `output-directory` absolute and outside the checkout.
Do not use `..` components in it.
The collection action rejects symlinks into the checkout.
Transfer only the collected record.
Each record follows the [job record schema](schemas/cache-job-v1.schema.json).
A record is at most 32 KiB and contains totals, configuration, and measurement gaps.
It contains no per-target events, paths, remote URLs, or credentials.

Add one final job.
In this example, `test` has two matrix rows named `ubuntu-latest` and `macos-latest`:

```yaml
cache-report:
  needs: test
  if: always()
  runs-on: ubuntu-latest
  steps:
    - uses: actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c # v8.0.1
      continue-on-error: true
      with:
        pattern: cache-record-*
        merge-multiple: true
        path: ${{ runner.temp }}/cache-records
    - uses: loadingalias/cargo-rail-action/cache/report@v12
      if: always()
      with:
        records-directory: ${{ runner.temp }}/cache-records
        expected-jobs: '["ubuntu-latest", "macos-latest"]'
```

`expected-jobs` must list the rows that enabled caching.
Missing or cancelled jobs stay visible as missing reports.
Missing measurements never become zero counts.
The report rejects records from other runs or attempts, and it rejects conflicting duplicates.
Exact duplicate records count once.
Collection finishes the interval.
For this reason, run collection after all compiler processes exit.

The report shows reuse, misses, bypasses, wrapper failures,
measured local reads and remote transfer, and grouped failure reasons.
To see storage and configuration, expand the job details.
The report shows storage per job, because caches can share backing storage.
The report does not estimate the time saved.
Cargo freshness can avoid all compiler invocations.
Thus, zero recorded outcomes do not prove that the cache was unused.
They also do not prove that every compilation was reused.

## Read the plan summary

Directly affected selections stay visible with their package or variant names.
Dependency details are expandable.
Their counts stay visible, and their execution selectors stay intact.
Missing-evidence warnings and `--all` stay visible.
The summary does not silently hide directly affected items beyond a fixed number.
Attribution comes from Cargo-Rail's captured plan.
The Action does not infer impact from filenames or explanation prose.

The summary states how many portable evidence manifests the plan used.
Without compatible evidence, Cargo work widens when a compiler, build script,
or procedural macro could read a changed file.
This rule applies even to a README.
Each widened item names the changed files that do not have negative evidence.
This widening is conservative.
It is not a routing error.
Do not replace it with path filters.
To skip unrelated files exactly, [record and transfer planning evidence](#skip-unrelated-files-with-planning-evidence).

## Skip unrelated files with planning evidence

Cargo-Rail records which workspace files each compiled unit read during an ordinary build.
Record this evidence in the jobs that already build on the default branch.
Write one file for each work item.
Save each file under the commit that it describes:

```yaml
- uses: loadingalias/cargo-rail-action/setup@v12
- name: Build tests and record planning evidence
  run: >
    cargo rail plan evidence --work cargo.test
    --output target/planning-evidence/cargo.test.json
    -- test --workspace --locked --no-run
- run: cargo test --workspace --locked
- uses: actions/cache/save@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0
  if: github.event_name == 'push'
  with:
    path: target/planning-evidence/cargo.test.json
    key: cargo-rail-evidence-${{ runner.os }}-${{ runner.arch }}-cargo.test-${{ github.sha }}
```

Record the command that the job runs, with the same packages, targets, features, and profile.
Recording adds no compilation.
The later test run finds every unit fresh.
Record `cargo.build` and `cargo.clippy` the same way in their jobs.

Before planning, restore the evidence for the commit that the planner compares against.
Then pass the directory:

```yaml
- uses: actions/cache/restore@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0
  with:
    path: target/planning-evidence/cargo.test.json
    key: cargo-rail-evidence-${{ runner.os }}-${{ runner.arch }}-cargo.test-${{ github.event.pull_request.base.sha || github.event.merge_group.base_sha || github.event.before }}
- uses: loadingalias/cargo-rail-action@v12
  id: rail
  with:
    evidence: target/planning-evidence
```

The default pull-request checkout is a merge commit.
For this checkout, the planner compares against the base commit of the pull request.
`evidence` accepts one file or a directory.
For a directory, the Action passes each `*.json` file in it to Cargo-Rail.
Cargo-Rail validates each file independently.
Each of these conditions only widens the affected work item:

- a cache miss;
- evidence from another commit, platform, or toolchain;
- evidence that recorded an incomplete build.

Make Git ignore the directory, because an unignored file is a changed path.
The Action transfers evidence.
It does not create or interpret evidence.
See [observed-input evidence](https://github.com/loadingalias/cargo-rail/blob/main/docs/planning.md#observed-input-evidence) for what the evidence covers.

## Runtime and compatibility

Version 12 uses one prebuilt Rust runtime with verified release checksums.
Bootstrap keeps the MIT `LICENSE` of the Action checkout beside the runtime.
It rejects installed license text that is modified or missing.

The `runtime-source` input defaults to `release`.
Use `source` only to validate an immutable Action commit before you publish its runtime release.
Source mode requires the pinned Rust toolchain.
It builds the checked-out runtime with `--locked` and authenticates the executable before installation.

By default, the Action installs the exact Cargo-Rail release that `.github/cargo-rail.lock` records.
Set `version` to another exact stable release only when the workflow needs a different compatible version.
The Action validates the installed binary, plan, cache, and component contracts before use.

### Supported version pairs

Each Action release is qualified with exactly one Cargo-Rail release: the version in its lock.
Only that pair is supported.
The Action checks contracts, not version numbers.
Another exact version, or `latest`, installs only when its archive, component manifest,
and license are valid.
Then every later step fails closed on a contract version that this Action does not accept.
This Action accepts plan v9, cache status schema 18, component manifest v1,
and the `cargo rail release` result of Cargo-Rail 0.32.
Thus, `latest` can select a Cargo-Rail release that this Action rejects.

The Action runs these `cargo rail` commands: `plan`, `surface --prepare`, `cache`, and `release`.
Cargo-Rail treats their arguments and machine output as compatibility contracts.

Repository configuration belongs to Cargo-Rail.
The Action never reads it.
Before you change `version`, run `cargo rail config validate --strict` with the new Cargo-Rail release.
A newer release can reject retired configuration fields.

The planner accepts plan contract v9, including identity-bound impact attribution.
Regenerate plans from older contracts.

The release Action reads the one JSON value that `cargo rail release --format json` prints.
It rejects an unknown `state`, a result for another command, and any commit, tag,
or pull request URL that is not a valid output value.
It keeps no release state between runs.
Cargo-Rail 0.31 and earlier do not have this command.

## Supported runners

Version 12 declares support for exactly these runners:

| Runner              | Native target               | Requirement |
| ------------------- | --------------------------- | ----------- |
| Linux x86-64        | `x86_64-unknown-linux-gnu`  | glibc 2.39 or newer |
| Linux ARM64         | `aarch64-unknown-linux-gnu` | glibc 2.39 or newer |
| macOS Apple silicon | `aarch64-apple-darwin`      | native execution |
| Windows x86-64      | `x86_64-pc-windows-msvc`    | the runner's Bash shell |

Bootstrap requires Bash, Git, `curl`, and `sha256sum` or `shasum`.
On an unsupported host, the Action fails before it downloads Cargo-Rail or changes the workspace.
Runtime support and host eligibility for the Cargo-Rail compiler cache are separate claims.

The Action reads Cargo-Rail release ZIPs through the pure-Rust `zlib-rs` DEFLATE backend,
without a native compression library.
It validates the component manifest and every archive entry
before it installs any selected component.

## Security boundary

Use ephemeral hosted runners, or single-tenant runners with equivalent isolation.
The runtime validates bounded manifests, checksums, complete archives, component receipts, plans,
GitHub environment files, and exact versions.
The runtime downloads each checksum from the same release as its executable or archive.
That checksum detects inconsistent bytes.
It is not an independent publisher signature.
The installer verifies checksums, component manifests, and the installed license.
It does not verify GitHub artifact attestations.
Before you trust the publication authority of a release, verify that the exact release is immutable.
Also verify every release-asset attestation, for example with `gh attestation verify cargo-rail-action-x86_64-unknown-linux-gnu -R loadingalias/cargo-rail-action`.

The planner uses its `repository-token` only for a Git fetch from the same repository when required history is absent.
The planner never puts the token in a URL, argv, repository configuration, output, summary,
or unrelated child process.

## Release integration

`loadingalias/cargo-rail-action/release` installs Cargo-Rail and runs `cargo rail release --yes` in the job.
This is the same command that a maintainer runs locally.
Thus, CI adds credentials and runners, but not a different release engine.
The [Cargo-Rail release guide](https://github.com/loadingalias/cargo-rail/blob/main/docs/releases.md) is the authority for its preconditions, configuration,
and complete workflow, including binary assets.
Pin the Action to a reviewed immutable commit.

```yaml
name: Release
on:
  workflow_dispatch:
    inputs:
      bump:
        type: string
        default: auto
permissions: {}
concurrency:
  group: release
  cancel-in-progress: false
jobs:
  release:
    runs-on: ubuntu-latest
    environment: release
    permissions:
      contents: write
      actions: read
      checks: read
      pull-requests: read
      statuses: read
    steps:
      - uses: actions/checkout@<sha>
        with:
          fetch-depth: 0
          persist-credentials: false
      - uses: loadingalias/cargo-rail-action/release@<sha> # v12
        with:
          bump: ${{ inputs.bump }}
        env:
          CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
```

| Input          | Default        | Meaning |
| -------------- | -------------- | ------- |
| `crates`       | empty          | Crates to release, separated by whitespace; empty releases every crate with release-worthy changes. |
| `bump`         | `auto`         | `auto`, `major`, `minor`, `patch`, `prerelease`, `release`, or an exact version. |
| `pull-request` | `false`        | Land the release commit through a pull request (`--pr`) for a protected branch. |
| `dry-run`      | `false`        | Preview the release and run every check without any effect. |
| `token`        | `github.token` | Token for the GitHub API and the release push. |
| `version`      | the lock       | Exact Cargo-Rail release to install. |

| Output         | Value |
| -------------- | ----- |
| `state`        | `nothing_to_release`, `planned`, `pull_request`, `awaiting_checks`, `awaiting_assets`, or `released`. |
| `commit`       | The release commit, once it exists. |
| `tags`         | JSON array of the release's tags. |
| `pull-request` | The release pull request URL, with `pull-request: true`. |

A pending check on the source commit fails the step before any effect.
When the checks finish, run the workflow again.
Every other state succeeds.
The step summary lists the registry state and the GitHub release state of each crate.
When you run the workflow again after any interruption, it finishes the release.

The job owns credentials and environment approvals.
The `token` input reaches only the GitHub API and the release push,
through process-scoped Git configuration.
The checkout keeps no credentials.
The read permissions let the release read the checks of the source commit, this workflow run,
and release pull requests.
`pull-request: true` needs `pull-requests: write`.
When branch rules block `github.token`, do one of these steps:

- Pass a token that has permission to push to the branch.
- Set `pull-request: true`.

The release commit uses the `github-actions[bot]` identity unless the checkout configures one.
Cargo reads registry credentials from the job environment.
An example is a `CARGO_REGISTRY_TOKEN` that crates.io trusted publishing issues.

`loadingalias/cargo-rail-action/release/asset` attests files and attaches them to the draft release that `cargo rail release` creates for a crate with `release.assets`.
Its inputs are:

- `tag`, from the `tags` output of the release step;
- `files`, with one path or glob on each line;
- `attest`, with the default `true`;
- `token`.

The asset job needs `contents`, `id-token`, `attestations`, and `artifact-metadata` write permissions.
Before Cargo-Rail publishes the draft, it verifies the digest of each file.
It also verifies the attestation of each file, unless `release.asset_attestations = "off"`.

The public Actions default to the exact Cargo-Rail release in `.github/cargo-rail.lock`.
The release and package workflows read that version and its release commit from the same lock.
The package workflow reads the independent `tooling` commit when it installs CI tools.
Update `version` and `commit` only after you publish and verify the selected Cargo-Rail release.

This repository releases itself with the same runner, which it builds from the checked-out source.
Its release workflow runs from `main` in the protected `release` environment.
The release job runs `cargo rail release` and stops at `awaiting_assets`.
The package workflow builds and tests every runtime from the release tag.
The assets job attests the runtimes and attaches them to the draft release.
Then the publish job runs the release again.
This second run verifies the assets, publishes the release, and moves the `v{major}` alias.
Push and pull-request CI call the same package workflow without a tag.
These runs need only read access and upload nothing.
The bootstrap derives the Action runtime release from the package version in `Cargo.toml`.

## Validate changes

Run `just check` for formatting, Clippy, unit and CLI tests, metadata contracts, and bootstrap syntax.
Run `just check-markdown` for Markdown lint.

When you change Cargo-Rail together with the Action, do these steps:

1. In the Cargo-Rail checkout, run `just package-release OUTPUT_DIRECTORY`.
   This command builds and packages the authenticated components of Cargo-Rail.
1. In this repository, run `just check-cargo-rail ABSOLUTE_BINARY_PATH ABSOLUTE_ARCHIVE_PATH VERSION`.
   This command runs the source plan and cache contracts, the real archive installer,
   and a release through the source engine against a local Git remote.

Cache setup uses an isolated temporary Cargo home.
The tests do not publish releases or contact remote cache storage.
After you update `.github/cargo-rail.lock`, use `just check-locked-cargo-rail ABSOLUTE_BINARY_PATH ABSOLUTE_ARCHIVE_PATH`.
This command reads the expected version from the lock.

Push and pull-request CI build authenticated Cargo-Rail archives from the exact release source
that `commit` pins.
Tool installation uses the independently pinned `tooling` commit.
Release packaging downloads the published Cargo-Rail version from the same lock.
Both paths run the same independent contract tests.
Update the version and the commit in the lock together.
Do this after you publish Cargo-Rail and before you dispatch the Action release.
The event selects the archive source.
If a download fails, CI does not build from source instead.
The current archive contract requires `LICENSE`.
Older archives without it are rejected.
Release lookup and download errors fail CI.

## Support

- [Action issues](https://github.com/loadingalias/cargo-rail-action/issues)
- [Cargo-Rail issues](https://github.com/loadingalias/cargo-rail/issues)

Cargo-Rail Action is licensed under [MIT](LICENSE).
Report vulnerabilities privately through the [Cargo-Rail security policy](https://github.com/loadingalias/cargo-rail/blob/main/SECURITY.md).
Include the Action version and the installed Cargo-Rail version.
