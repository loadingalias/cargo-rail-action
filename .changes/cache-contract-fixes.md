---
cargo-rail-action = "patch"
---

- The cache action accepts the `recovery_action` field that Cargo-Rail reports for an unhealthy installation.
  The action now rejects such an installation for its state, instead of for an unknown field.
- The job record schema `schemas/cache-job-v1.schema.json` accepts every exact stable Cargo-Rail version.
  The cache report and the cache status schema already accepted them.
  Before this change, the job record schema accepted only Cargo-Rail 0.26 releases.
- Bootstrap errors name the Action release that runs, instead of v10.
