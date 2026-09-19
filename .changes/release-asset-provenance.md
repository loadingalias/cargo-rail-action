---
"cargo-rail-action" = "patch"
---

Attest the complete Action release asset set, including the runtime manifest and license, so every downloaded asset
can be verified against the exact GitHub-hosted package workflow and source commit.

Accept Cargo-Rail release-record v10 while retaining independent fail-closed schema, identity, checkout, invocation,
artifact, and effect-order validation.

Synchronize every public Action entry point with the authenticated Cargo-Rail release that writes release-record v10.

Preserve the Action's historical `v{version}` tag namespace under Cargo-Rail's crate-qualified default.

Document that cache setup must precede plan capture when both actions run in one job,
so later selectors validate the same Cargo configuration that the planner captured.
