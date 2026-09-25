# Cargo-Rail Action

Cargo-Rail Action brings Cargo-Rail planning, verified compiler reuse,
and durable releases to GitHub Actions.
It installs an exact Cargo-Rail release and validates each plan, cache,
and release boundary before use.
Cargo and the workflow still run the work.

## Reduce work at each layer

| Action  | Resource effect |
| ------- | --------------- |
| Planner | Lets the workflow skip jobs that are not required and gives required jobs exact Cargo selectors. |
| Cache   | Restores compatible compiler results inside the jobs that still run. |
| Release | Carries reviewed Rust changesets into one durable, resumable publication transaction. |

```text
all declared workflow jobs
└─ planner keeps required jobs and emits exact Cargo scope
   └─ cache restores compatible compiler results in those jobs
      └─ Cargo runs freshness checks and the remaining misses
```

Planning and caching solve different problems, so their reductions stack.
Start with the planner alone.
Add caching when the job has explicit remote authority and credentials.
Add the release Action only to a protected publication job.
Use the plan summary and cache report to see the actual result;
the Action does not invent a time-saved estimate.

## Plan and run work in one job

Install your Rust toolchain and command runners, such as nextest, before these steps.

```yaml
- uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
  with:
    persist-credentials: false

- uses: loadingalias/cargo-rail-action@v10
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

Every selector validates the complete plan, recomputes its canonical identity,
and asks the exact installed Cargo-Rail to verify the current checkout before emitting stdout.
Mutating the repository after selector emission and
before the consuming command remains a caller error.
The planner publishes `cargo-rail` and the stable `cargo-rail-action` launcher to later steps in the same job.

The planner publishes only:

- `version`: the exact installed Cargo-Rail version;
- `plan-file`: the absolute validated plan path; and
- `required-work`: a compact JSON array of required work IDs.

See the [planner inputs](action.yaml) for comparison refs, full verification, evidence, and workspace selection.

Set `components` to `surface` or `complete` when planning needs Surface.
During explicit Surface preparation, Cargo-Rail may install `rustc-dev` and compile its authenticated,
toolchain-bound compiler fact driver.
Native cache preparation can also compile that driver from the authenticated source package
when development components for the selected compiler are already installed.
Release mode does not compile the Action runtime or Cargo-Rail itself in the workflow.

## Transfer a plan across jobs

Transfer only `plan.json`.
Install the required Rust toolchain and command runners in each job.
Use the setup action to install the same Cargo-Rail version in the consumer:

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
      - uses: loadingalias/cargo-rail-action@v10
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
      - uses: loadingalias/cargo-rail-action/setup@v10
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
Match the planning job's source, Rust toolchain, platform, and relative workspace directory.
Run selectors from that workspace directory; checkout verification rejects source drift.
Create a separate plan for each platform when the workflow spans platforms.

## Group work and route other platforms

These jobs extend the `plan` job above.
A grouped job runs when any work it owns is required.
Guard each command with its own work ID.
Repository work such as `cargo.fmt` can route the job, but it has no Cargo selector.
The Linux plan's `required-work` can route a macOS job.
Only a plan created on macOS authorizes macOS selectors,
so that job plans again with the same Cargo-Rail version:

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
      - uses: loadingalias/cargo-rail-action/setup@v10
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
      - uses: loadingalias/cargo-rail-action@v10
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

The macOS step routes on its own plan because its decision can differ from the Linux plan.
A reader rejects a plan from another platform before checkout verification.
Split a grouped job when its commands have different inputs, setup cost, or failure ownership.

## Run Cargo-Rail checks with setup only

Use `setup` when a job runs Cargo-Rail commands without the planner:

```yaml
- uses: loadingalias/cargo-rail-action/setup@v10
  id: cargo-rail
- run: cargo rail unify --check
```

Setup installs authenticated prebuilt components.
It compiles no Rust, so your workspace toolchain
and MSRV stay independent of Cargo-Rail's build version.
The `version` input defaults to the release in this Action's `.github/cargo-rail.lock`; the action metadata and the lock always agree.
The step logs `Cargo-Rail setup ready: VERSION` and publishes the same value as its `version` output.

Reproduce that version locally with one command, then check it:

```bash
cargo install cargo-rail --locked --version VERSION
cargo rail --version
```

A source install needs Cargo-Rail's `rust-version`, not your workspace toolchain.
See [Cargo-Rail installation paths](https://github.com/loadingalias/cargo-rail#installation-paths) for archives and compiler components.

A repository wrapper that skips a check when Cargo-Rail is missing keeps local runs convenient,
but policy drift then appears first in CI.
Install the matching version instead of relying on the skip.

Before you make `cargo rail unify --check` a required check, establish a clean baseline: review the proposed edits,
apply them in one reviewed change, and confirm that `--check` passes on the default branch.
Do not keep an optional check that always fails; reviewers learn to ignore it.

## Migrate from an earlier version

Before it plans, the planner audits every tracked or unignored YAML file in the repository
for Cargo-Rail Action references.
It prints each reference with its version, pin, job, and inputs,
and the job conditions that read Cargo-Rail outputs.
These errors stop the planner before any job runs:

- a reference to an earlier major version, so a partial migration cannot run;
- an input, output, or action path that the current release does not provide,
  such as a Boolean output from v7 or earlier;
- a `needs.JOB.outputs.NAME` that the planning job does not export, which GitHub evaluates as empty;
- a file that mentions the Action but is not valid YAML.

Warnings annotate the file and line: `plan-file` exported as a job output
(it is a path on one runner),
a job that reads selectors from a plan made on another runner label,
and a pin whose release cannot be read (add a `# vX.Y.Z` comment to a commit pin).

Run the same audit locally from the release you migrate to:

```bash
cargo install --locked --git https://github.com/loadingalias/cargo-rail-action --tag vX.Y.Z cargo-rail-action
cargo-rail-action audit
```

Migrate in this order; each step proves one claim:

1. `cargo rail config validate --strict` proves the policy is valid for the workspace's Cargo graph.
1. `cargo-rail-action audit` proves every workflow reference and output consumer matches this release.
1. `cargo rail plan --cases routes.toml` proves reviewed path changes route as expected, before you remove the previous selector.
   See [route parity](https://github.com/loadingalias/cargo-rail/blob/main/docs/planning.md#check-route-parity-before-a-migration).
1. `cargo rail plan --all --json` proves every work item has valid full scope.
   It proves no routing decision.

## Read selectors

The direct consumer surface is intentionally small:

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
Line-oriented selectors emit one compact value and newline.
Argument and package selectors emit NUL-delimited values.
`target-args` emits `--test NAME` pairs only when every selected target is an integration test.
It emits nothing for an empty, mixed,
or unsupported target set so Cargo runs the selected packages without unsafe narrowing.
`matrix` emits an `include` object for selected rows, but emits the literal `all` for unrestricted variant scope.
Handle `all` by using the work item's complete checked-in catalog before calling `fromJSON`.
`--family FAMILY` filters selected rows and nests each row under that family name.
It does not expand `all`.
Skipped variant work emits `{"include":[]}`.
Skip the consuming job when no rows are selected.

Contract rejections exit `2`; rejected plan selectors emit no stdout.
Operational failures, including I/O and failed subprocesses, exit `1`.

## Configure compiler caching

```yaml
- uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
  with:
    persist-credentials: false

- uses: loadingalias/cargo-rail-action/cache@v10
  id: cache
  with:
    remote: s3://cargo-rail-cache/team?region=us-east-1&owner=123456789012
    mode: read
    max-size: 10GiB
    root-portability: physical
    verify-remote: false
```

When one job uses both actions, configure `loadingalias/cargo-rail-action/cache` before `loadingalias/cargo-rail-action`.
Cache setup changes Cargo configuration, and plan verification binds that configuration.
Capturing the plan first causes later selectors to reject the changed execution authority.

`mode` is always explicit.
Use `read` for pull requests and other untrusted jobs.
`verify-remote: true` authenticates to the selected provider and requires the protocol marker
before publishing Action outputs.
See the [cache inputs](cache/action.yaml) for local cache location and workspace selection.

The cache installer requires the core executable, native wrapper and worker,
matched compiler driver, and authenticated driver source package.
Archives or installation receipts missing either driver component are rejected.
Every component set also requires the authenticated `LICENSE` entry and retains it in the installation.
Cargo-Rail owns toolchain matching and cache preparation.

The cache action publishes only `version`
and one compact `status` value conforming to [`schemas/cache-status-v1.schema.json`](schemas/cache-status-v1.schema.json).
It contains provider, mode, local byte bound, root portability,
and whether remote verification was requested and passed.
It never contains the remote URL, credentials, local paths, authority identity, protocol marker,
or component receipts.

## One cache report for the workflow

Configure caching once in each participating job.
Collect after its final Cargo command, including failure paths.
The setup and collection actions publish no summaries.
The final report combines all jobs into one summary.

```yaml
# Append these steps to each cache-enabled job.
- uses: loadingalias/cargo-rail-action/cache/collect@v10
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

Use a unique `job` label for each matrix row: 1–128 ASCII letters, digits, dots, hyphens, or underscores.
Use a separate label when a runner expression contains other characters.
For a job without a matrix, use its workflow job ID.
Keep `output-directory` absolute and outside the checkout, without `..` components;
symlinks into the checkout are rejected.
Transfer only the collected record.
Each record follows the [job record schema](schemas/cache-job-v1.schema.json), is bounded to 32 KiB, and contains totals,
configuration, and measurement gaps.
It contains no per-target events, paths, remote URLs, or credentials.

Add one final job.
Here, `test` has two matrix rows named `ubuntu-latest` and `macos-latest`:

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
    - uses: loadingalias/cargo-rail-action/cache/report@v10
      if: always()
      with:
        records-directory: ${{ runner.temp }}/cache-records
        expected-jobs: '["ubuntu-latest", "macos-latest"]'
```

`expected-jobs` must list the rows that enabled caching.
Missing or cancelled jobs stay visible as missing reports;
missing measurements never become zero counts.
Records from other runs or attempts and conflicting duplicates are rejected.
Exact duplicate records count once.
Collection finishes the interval, so run it after all compiler processes exit.

The report shows reuse, misses, bypasses, wrapper failures,
measured local reads and remote transfer, and grouped failure reasons.
Expand the job details for storage and configuration.
Storage is kept per job because caches may share backing storage.
The report does not estimate time saved.
Cargo freshness can avoid compiler invocations entirely;
zero recorded outcomes does not establish that the cache was unused or
that every compilation was reused.

## Read the plan summary

Directly affected selections stay visible with their package or variant names.
Dependency details are expandable;
their counts remain visible and their execution selectors remain intact.
Missing-evidence warnings and `--all` remain visible.
No fixed number of directly affected items is silently hidden.
Attribution comes from Cargo-Rail's captured plan;
the Action does not infer impact from filenames or explanation prose.

The summary states whether portable evidence was supplied.
The Action does not create portable evidence.
Without the `evidence` input, Cargo work widens when a changed file could be read by a compiler, build script,
or procedural macro, even a README.
Each widened item names the changed files that lack negative evidence.
That widening is conservative, not a routing error; do not replace it with path filters.

## Runtime and compatibility

Version 10 uses one prebuilt Rust runtime with verified release checksums.
Bootstrap keeps the Action checkout's MIT `LICENSE` beside the runtime and rejects modified
or missing installed license text.

The `runtime-source` input defaults to `release`.
Use `source` only to validate an immutable Action commit before publishing its runtime release.
Source mode requires the pinned Rust toolchain, builds the checked-out runtime with `--locked`,
and authenticates the executable before installation.

By default, the Action installs the exact Cargo-Rail release recorded in `.github/cargo-rail.lock`.
Set `version` to another exact stable release only when the workflow needs a different compatible version.
The Action validates the installed binary, plan, cache, and component contracts before use.

### Supported version pairs

Each Action release is qualified with exactly one Cargo-Rail release: the version in its lock.
Only that pair is supported.
The Action checks contracts, not version numbers.
Another exact version, or `latest`, installs only when its archive, component manifest,
and license validate.
Every later step then fails closed on a contract version this Action does not accept.
It accepts plan v9, release record v10, cache status schema 18, and component manifest v1.
`latest` can therefore select a Cargo-Rail release that this Action rejects.

The Action runs these `cargo rail` commands: `plan`, `surface --prepare`, `cache`, and `release`.
Cargo-Rail treats their arguments and machine output as compatibility contracts.

Repository configuration belongs to Cargo-Rail; the Action never reads it.
Before you change `version`, run `cargo rail config validate --strict` with the new Cargo-Rail release,
because a newer release can reject retired configuration fields.

The planner accepts plan contract v9, including identity-bound impact attribution.
Regenerate plans from older contracts.

Release-record compatibility is separate from plan compatibility.
The runtime accepts Cargo-Rail [release execution records](schemas/release-record-v10.schema.json) at v10
and fails closed on other versions.
It validates intent identity, expected source and repository, required workflow evidence,
and upload attempt identities.
The locked Cargo-Rail release writes that contract.

The release Action validates hosted requests and reviewed merges
before invoking the same Cargo-Rail transaction.
This repository uses that engine for its own releases and promotes `v10` only
after verifying the immutable release.

## Supported runners

V10.0 advertises exactly:

| Runner              | Native target               | Requirement |
| ------------------- | --------------------------- | ----------- |
| Linux x86-64        | `x86_64-unknown-linux-gnu`  | glibc 2.39 or newer |
| Linux ARM64         | `aarch64-unknown-linux-gnu` | glibc 2.39 or newer |
| macOS Apple silicon | `aarch64-apple-darwin`      | native execution |
| Windows x86-64      | `x86_64-pc-windows-msvc`    | the runner's Bash shell |

Bootstrap requires Bash, Git, `curl`, and `sha256sum` or `shasum`.
Unsupported hosts fail before Cargo-Rail download or workspace mutation.
Runtime support and Cargo-Rail compiler-cache host eligibility are separate claims.

The Action reads Cargo-Rail release ZIPs through the pure-Rust `zlib-rs` DEFLATE backend,
without a native compression library.
It validates the component manifest and every archive entry
before installing any selected component.

## Security boundary

Use ephemeral hosted runners or equivalently isolated single-tenant runners.
The runtime validates bounded manifests, checksums, complete archives, component receipts, plans,
GitHub environment files, and exact versions.
The runtime downloads each checksum from the same release as its executable or archive.
That checksum detects inconsistent bytes but is not an independent publisher signature.
The installer verifies checksums, component manifests, and the installed license,
but it does not verify GitHub artifact attestations.
Verify the exact release's immutability and every release-asset attestation
before trusting its publication authority.

The planner's `repository-token` is used only for a same-repository Git fetch when required history is absent.
It is never placed in a URL, argv, repository configuration, output, summary,
or unrelated child process.

## Release integration

`loadingalias/cargo-rail-action/release` runs the Cargo-Rail release engine from a caller-owned GitHub publication job.
Pin the Action to a reviewed immutable commit.
Configure `release.hosted_workflow` and required validation in Cargo-Rail first.

Inputs are `version`, `packages` (a JSON array; `[]` selects all), `bump`, `publish`, and `review`.
Publication defaults to false.
The workflow accepts `transaction`, `intent`, and `source` dispatch inputs so Cargo-Rail can continue a retained request.
A merged same-repository release PR can invoke the same Action through `pull_request_target`.
The adapter validates the original record, workflow, repository, event, prepared commit,
and merged tree before invoking the core.

The job owns credentials and environment approvals.
Serialize release jobs across dispatch and review events, preserve Git push credentials,
and use full history.
Outputs are `transaction-id`, `release-sha`, `state`, and `run-url`.
See the [Cargo-Rail release guide](https://github.com/loadingalias/cargo-rail/blob/main/docs/releases.md) for the complete workflow contract.

The public Actions default to the exact Cargo-Rail release in `.github/cargo-rail.lock`.
The release and package workflows read that version and its release commit from the same lock.
The package workflow reads the independent `tooling` commit when it installs CI tools.
Update `version` and `commit` only after publishing and verifying the selected Cargo-Rail release.

Push and pull-request CI call the same package workflow used for release validation,
but only a direct release-validation dispatch uploads runtime assets.
The release workflow runs from `main`, uses the protected `release` environment, and fixes its package selection,
bump policy, registry publication, and review mode.
Leave its recovery inputs empty for a new release.
Supply all three retained values only when Cargo-Rail continues an existing transaction.

Cargo-Rail owns publication and the configured `v10` alias promotion.
The bootstrap derives the Action runtime release from the package version in `Cargo.toml`.

## Validate changes

Run `just check` for formatting, Clippy, unit and CLI tests, metadata contracts, and bootstrap syntax.
When changing Cargo-Rail alongside the Action,
run `just package-release OUTPUT_DIRECTORY` in the Cargo-Rail checkout to build and package its authenticated components.
Then run `just check-cargo-rail ABSOLUTE_BINARY_PATH ABSOLUTE_ARCHIVE_PATH VERSION` here.
This runs source plan and cache contracts, the real archive installer,
strict release-record rejection, and release adapter recovery through real Git fixtures.
Cache setup uses an isolated temporary Cargo home;
the tests do not publish releases or contact remote cache storage.
After updating `.github/cargo-rail.lock`, use `just check-locked-cargo-rail ABSOLUTE_BINARY_PATH ABSOLUTE_ARCHIVE_PATH` to read the expected version from the lock.

Push and pull-request CI build authenticated Cargo-Rail archives from the exact release source
pinned by `commit`.
Tool installation uses the independently pinned `tooling` commit.
Release-validation dispatches download the published Cargo-Rail version from the same lock.
Both paths run the same independent contract tests.
Update the lock's version and commit together after publishing Cargo-Rail and
before dispatching the Action release.
The event selects the archive source; failed downloads never fall back to a source build.
The current archive contract requires `LICENSE`; older archives without it are rejected.
Release lookup and download errors fail CI.

## Support

- [Action issues](https://github.com/loadingalias/cargo-rail-action/issues)
- [Cargo-Rail issues](https://github.com/loadingalias/cargo-rail/issues)

Cargo-Rail Action is licensed under [MIT](LICENSE).
Report vulnerabilities privately using the [Cargo-Rail security policy](https://github.com/loadingalias/cargo-rail/blob/main/SECURITY.md);
include the Action version and installed Cargo-Rail version.
