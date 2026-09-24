---
"cargo-rail-action" = "patch"
---

Authenticate shallow pull-request history fetches with exactly one repository credential,
including checkouts that persisted their own credentials.
Report a failed history fetch with a safe cause and one recovery action instead of an exit code.
`target-args` now narrows Cargo work only when every selected target is an integration test.
