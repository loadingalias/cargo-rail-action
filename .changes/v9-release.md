---
"cargo-rail-action" = "major"
---

Use the native Action runtime and compatible Cargo-Rail 0.26 components
for independently validated planning, compiler-cache setup, and release execution.
Install authenticated component archives, including their source license,
and reject unknown or inconsistent contracts before exposing outputs.

Add the release Action for caller-owned GitHub publication jobs.
Validate the original release record, repository, workflow, dispatch,
and reviewed merge before invoking Cargo-Rail.
Expose the transaction ID, exact release commit, state, and executor URL.
Recover the same request after runner loss without an Action-specific publication engine.

Keep runtime manifest generation as product packaging.
Delegate immutable releases and explicit major-tag promotion to Cargo-Rail,
including conflict detection and recovery after uncertain effects.
