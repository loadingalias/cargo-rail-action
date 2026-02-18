# Contributing to cargo-rail-action

Thanks for contributing.

## Development Setup

Prerequisites:

- `python3` (summary renderer tests)
- `ruby` (YAML parse validation)
- `jq` (used by workflow verification steps)
- `actionlint` (recommended)

Run fixture tests locally:

```bash
./tests/test_summary.sh
```

Validate `action.yaml` structure:

```bash
ruby -ryaml -e 'YAML.load_file("action.yaml")'
```

Recommended semantic lint:

```bash
actionlint
```

## Change Requirements

- Keep changes focused and deterministic.
- Update README and tests when output contracts change.
- Preserve compatibility with documented outputs.
- For changes that touch both repos, validate cargo-rail with `just check-all && just test` before opening a PR.

## Pull Requests

- Use a clear title and summary.
- Include behavior changes and migration notes if outputs changed.
- Link related issues when applicable.

## Security

Please do not open public issues for vulnerabilities. See [SECURITY.md](SECURITY.md).
