# Contributing to cargo-rail-action

## Before changing the action

Required tools:

- `bash`
- `python3`
- `ruby`
- `jq`

Optional:

- `actionlint`

Run all contract, summary, and git-history tests:

```bash
bash tests/test_summary.sh
bash tests/test_contracts.sh
bash tests/test_ensure_history.sh
```

Validate `action.yaml` after changing inputs, outputs, or composite steps:

```bash
ruby -ryaml -e 'YAML.load_file("action.yaml")'
```

## Change requirements

- Keep shell and Python behavior deterministic and independent of a developer's global configuration.
- Update tests and README tables when an input, output, default, installation path, or planner contract changes.
- Treat existing output names and meanings as public API. Breaking changes require a new action major.
- Test shallow-history changes with `tests/test_ensure_history.sh`; local full clones do not exercise that path.
- When a patch also changes cargo-rail, run `just check && just test` in the cargo-rail repository.

## Pull requests

- Explain the workflow behavior that changes.
- Include the commands used to verify it.
- Call out changes to inputs, outputs, defaults, planner contracts, supported runners, or checksum handling.
- Link the issue when one exists.
