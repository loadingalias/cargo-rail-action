# cargo-rail-action

> GitHub Action for `cargo rail plan` gates and execution scope.

[![Test](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml/badge.svg)](https://github.com/loadingalias/cargo-rail-action/actions/workflows/test.yaml) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## Quick Start

```yaml
name: CI
on: [push, pull_request]

jobs:
  plan:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: loadingalias/cargo-rail-action@v4
        id: rail

      - name: Run selected CI profile
        if: steps.rail.outputs.build == 'true' || steps.rail.outputs.test == 'true'
        run: cargo rail run --since "${{ steps.rail.outputs.base-ref }}" --profile ci

      - name: Run docs pipeline
        if: steps.rail.outputs.docs == 'true'
        run: cargo rail run --since "${{ steps.rail.outputs.base-ref }}" --surface docs
```

The default `version` tracks the latest published stable `cargo-rail` release this action is tested against. Set `version` only when you need a different published release.

## What It Publishes

Minimal mode publishes:

- `build`
- `test`
- `bench`
- `docs`
- `infra`
- `scope-json`
- `base-ref`
- `custom_<name>` for custom surfaces

Debug mode adds:

- `plan-json`

`scope-json` is the execution handoff. `plan-json` is for debugging.

## Inputs

| Input | Default | Description |
|---|---|---|
| `version` | `0.13.0` | Published `cargo-rail` release to install |
| `checksum` | `required` | `required`, `if-available`, or `off` |
| `since` | auto | Git ref for planner comparison |
| `args` | `""` | Extra planner args except format/output flags |
| `working-directory` | `.` | Workspace directory |
| `token` | `${{ github.token }}` | Token for release download API |
| `mode` | `minimal` | `minimal` or `debug` |

## Compatibility

The action validates both planner contracts before publishing outputs.

- `plan_contract_version` covers the full diagnostic planner payload
- `scope_contract_version` covers the execution handoff payload

They are separate on purpose. `scope-json` should be able to stay stable even when `plan-json` grows.

## Notes

- Checksum verification is on by default.
- Shallow checkouts are handled automatically when more history is needed.
- Use `@v4` for the stable action major, or pin a commit SHA if you want maximum reproducibility.

## Getting Help

- Action Issues: [GitHub Issues](https://github.com/loadingalias/cargo-rail-action/issues)
- Core Issues: [loadingalias/cargo-rail](https://github.com/loadingalias/cargo-rail/issues)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## Security

See [SECURITY.md](SECURITY.md).

## License

Licensed under [MIT](LICENSE).
