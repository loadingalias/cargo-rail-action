# Contributing to Cargo-Rail Action

## Set up the repository

Install Bash, Git, Python 3, Ruby, `jq`, and a Rust toolchain. The Rust toolchain is needed by the saved-plan verifier
fixtures.

Run the fixture suite used by CI:

```bash
bash tests/test_summary.sh
bash tests/test_contracts.sh
bash tests/test_plan_verification.sh
bash tests/test_install.sh
bash tests/test_cache_setup.sh
bash tests/test_ensure_history.sh
bash tests/test_release.sh
bash tests/test_release_availability.sh
```

The worktree-drift test also needs a matching source-built Cargo-Rail binary:

```bash
CARGO_RAIL_BIN=/absolute/path/to/cargo-rail bash tests/test_plan_drift.sh
```

After changing either composite action's metadata, validate both YAML files:

```bash
ruby -ryaml -e 'YAML.load_file("action.yaml"); YAML.load_file("cache/action.yaml")'
```

## Preserve the action contracts

- Treat existing input names, output names, defaults, and meanings as public API. Breaking changes require a new action
  major.
- Keep `scripts/plan.py` aligned with Cargo-Rail's strict plan reader. It may publish bounded outputs and summaries,
  but it must not classify work or reconstruct selectors.
- Keep cache setup in `scripts/cache_setup.py`; composite metadata must not create a second setup path.
- Keep shell and Python behavior deterministic and independent of developer-global configuration.
- Test shallow-history changes with `tests/test_ensure_history.sh`; a full local clone does not exercise that boundary.

## Coordinate changes with Cargo-Rail

When a patch changes both repositories:

1. Change and validate Cargo-Rail's owning contract first.
2. Update this repository's independent fail-closed consumer.
3. Run `just check` and `just test` in the Cargo-Rail repository.
4. Run this repository's fixture suite and `tests/test_plan_drift.sh` with the matching binary.

Update tests and public documentation when an input, output, default, installation path, component set, plan contract,
cache status contract, or supported runner changes.

## Open a pull request

- Explain the workflow behavior that changes.
- List the exact validation commands and their results.
- Identify changes to inputs, outputs, defaults, plan contracts, runners, component sets, or checksum handling.
- Link the issue when one exists.

## Release a coordinated version

When an Action release defaults to a new Cargo-Rail version, release in this order:

1. Publish the Cargo-Rail crate and every required native release archive.
2. Rerun the `Test Action` workflow with that exact version. Require the planner, cache, Linux, and Windows jobs to
   pass; qualify each additional archive target before claiming support for it.
3. Dispatch this repository's `Release` workflow from `main` with the new Action version.

The integration matrix installs the Cargo-Rail version in `release-train.json`. A missing required archive defers the
ordinary integration jobs and fails the Action release gate. The `Release` workflow's `version` input selects the
Action version, not the Cargo-Rail version.

Before the Cargo-Rail release exists, push and pull-request workflows run fixture and native-Windows contract tests,
then explicitly defer release-dependent integration jobs. The Action release workflow requires the exact Cargo-Rail
release assets and cannot defer those jobs.
