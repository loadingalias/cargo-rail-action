# Cargo-Rail Action

Cargo-Rail Action installs Cargo-Rail, plans required CI work,
and emits exact selectors for your commands.
Version 9 uses one prebuilt Rust runtime with verified release checksums.
Bootstrap preserves the Action checkout’s MIT `LICENSE` beside the runtime and rejects modified
or missing installed license text.

Cargo-Rail Action v9 accepts only stable Cargo-Rail `0.26.PATCH` releases and defaults to `0.26.0`.
It rejects every other Cargo-Rail minor line and validates the exact installed binary, plan, cache,
and component contracts before use.
The planner requires plan contract v9, including identity-bound impact attribution.
Regenerate plans from older contracts.

## Plan and run work in one job

Install your Rust toolchain and command runners, such as nextest, before these steps.

```yaml
- uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
  with:
    persist-credentials: false

- uses: loadingalias/cargo-rail-action@v9
  id: rail
  with:
    version: 0.26.0

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
Neither the Action runtime nor Cargo-Rail itself is compiled in the workflow.

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
      - uses: loadingalias/cargo-rail-action@v9
        id: rail
        with:
          version: 0.26.0
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
      - uses: loadingalias/cargo-rail-action/setup@v9
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

- uses: loadingalias/cargo-rail-action/cache@v9
  id: cache
  with:
    version: 0.26.0
    remote: s3://cargo-rail-cache/team?region=us-east-1&owner=123456789012
    mode: read
    max-size: 10GiB
    root-portability: physical
    verify-remote: false
```

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
- uses: loadingalias/cargo-rail-action/cache/collect@v9
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
    - uses: loadingalias/cargo-rail-action/cache/report@v9
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

## Supported runners

V9.0 advertises exactly:

| Runner              | Native target              | Requirement |
| ------------------- | -------------------------- | ----------- |
| Linux x86-64        | `x86_64-unknown-linux-gnu` | glibc 2.39 or newer |
| macOS Apple silicon | `aarch64-apple-darwin`     | native execution |
| Windows x86-64      | `x86_64-pc-windows-msvc`   | the runner's Bash shell |

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
Checksums bind bytes to the immutable Action or Cargo-Rail release authority;
they are not an independent publisher signature.

The planner's `repository-token` is used only for a same-repository Git fetch when required history is absent.
It is never placed in a URL, argv, repository configuration, output, summary,
or unrelated child process.

## Validate changes

Run `just check` for formatting, Clippy, unit and CLI tests, metadata contracts, and bootstrap syntax.
When changing Cargo-Rail alongside the Action,
run `just package-release OUTPUT_DIRECTORY` in the Cargo-Rail checkout to build and package its authenticated components.
Then run `just check-cargo-rail ABSOLUTE_BINARY_PATH ABSOLUTE_ARCHIVE_PATH VERSION` here.
This explicitly runs the source plan and cache contracts and the real archive installer tests.
Cache setup uses an isolated temporary Cargo home;
the tests do not publish releases or contact remote cache storage.

CI validates the published Cargo-Rail archive when available.
Before that release exists, it builds the authenticated archive from the Cargo-Rail commit pinned in
the CI workflow and runs the same contract tests.
Update that pin when validating newer unreleased source.
The current archive contract requires `LICENSE`; older archives without it are rejected.
Release lookup and download errors fail CI.

## Support

- [Action issues](https://github.com/loadingalias/cargo-rail-action/issues)
- [Cargo-Rail issues](https://github.com/loadingalias/cargo-rail/issues)

Cargo-Rail Action is licensed under [MIT](LICENSE).
Report vulnerabilities privately using the [Cargo-Rail security policy](https://github.com/loadingalias/cargo-rail/blob/main/SECURITY.md);
include the Action version and installed Cargo-Rail version.
