# Contributing to cargo-rail-action

## Local Setup

Required tools:

- `bash`
- `python3`
- `ruby`
- `jq`

Recommended:

- `actionlint`

Run the local test set:

```bash
bash tests/test_summary.sh
bash tests/test_contracts.sh
bash tests/test_ensure_history.sh
```

Validate `action.yaml` structure:

```bash
ruby -ryaml -e 'YAML.load_file("action.yaml")'
```

## Expectations

- Keep changes focused and deterministic.
- Update README and tests when documented behavior changes.
- Preserve compatibility with documented outputs.
- If a change spans both repos, validate `cargo-rail` with `just check && just test`.

## Pull Requests

- Use a clear title and summary.
- Call out output or compatibility changes.
- Link related issues when applicable.

## Security

Do not open public issues for vulnerabilities. See [SECURITY.md](SECURITY.md).
