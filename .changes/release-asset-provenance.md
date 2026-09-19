---
"cargo-rail-action" = "patch"
---

Attest the complete Action release asset set, including the runtime manifest and license, so every downloaded asset
can be verified against the exact GitHub-hosted package workflow and source commit.

Document that cache setup must precede plan capture when both actions run in one job,
so later selectors validate the same Cargo configuration that the planner captured.
