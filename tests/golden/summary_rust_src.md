## cargo-rail plan

| | |
|---|---|
| **Version** | `0.20.0` |
| **Install** | Binary download |
| **Base** | `origin/main` |
| **Changed files** | 1 |
| **Scope mode** | `workspace` |
| **Direct crates** | 1 |
| **Active surfaces** | build, test |

**Changed direct crates (1):** `lib-a`
**Execution scope:** full workspace
**Top reasons:** Rust source file changed; Transitive dependency of changed crate

<details><summary>Trace summary (4 reasons)</summary>

**Reason counts**
- Balanced confidence profile active: 1
- Rust source file changed: 1
- File directly owns a crate: 1
- Transitive dependency of changed crate: 1

**Sample trace entries (4 of 4)**
- r1 CONFIDENCE_PROFILE_BALANCED
- r2 FILE_OWNS_CRATE_DIRECT file=crates/lib-a/src/lib.rs crate=lib-a
- r3 FILE_KIND_RUST_SRC file=crates/lib-a/src/lib.rs surfaces=build,test
- r4 TRANSITIVE_DEPENDS_ON_DIRECT crate=lib-b depends_on=lib-a surfaces=build,test

</details>
