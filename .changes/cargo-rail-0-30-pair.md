---
"cargo-rail-action" = "minor"
---

Every public action now installs Cargo-Rail 0.30.0 by default,
the only version this release supports.
Plans are still contract v9 and cache status is still schema 18; planning evidence is `planning-evidence-v2`,
which 0.30.0 records.
Compiler caches start cold once after the upgrade,
because Cargo-Rail 0.30.0 changed its action keys.
CI tool installation on Linux and Windows now authenticates `cargo-binstall`'s GitHub API lookups,
as macOS already did.
