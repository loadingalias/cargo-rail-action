## cargo-rail plan

| | |
|---|---|
| **Version** | `0.10.0` |
| **Install** | Binary download |
| **Base** | `origin/main` |
| **Changed files** | 1 |
| **Scope mode** | `workspace` |
| **Direct crates** | 1 |
| **Active surfaces** | build, test |

**Changed direct crates (1):** `lib-a`
**Execution scope:** full workspace
**Top reasons:** Rust source file changed; Transitive dependency of changed crate

<details><summary>Trace summary (3 reasons)</summary>

**Reason counts**
- Rust source file changed: 1
- File directly owns crate: 1
- Transitive dependency of changed crate: 1

**Sample trace entries (3 of 3)**
- r1 FILE_OWNS_CRATE_DIRECT file=crates/lib-a/src/lib.rs crate=lib-a
- r2 FILE_KIND_RUST_SRC file=crates/lib-a/src/lib.rs surface=build
- r3 TRANSITIVE_DEPENDS_ON_DIRECT crate=lib-b depends_on=lib-a surface=build

</details>
