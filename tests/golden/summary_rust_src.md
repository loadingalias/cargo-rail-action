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

**Changed direct crates:** `lib-a`
**Execution scope:** full workspace
**Top reasons:** Rust source file changed; Transitive dependency of changed crate

<details><summary>Trace details (file -> crate -> surface)</summary>

- r1 FILE_OWNS_CRATE_DIRECT file=crates/lib-a/src/lib.rs crate=lib-a
- r2 FILE_KIND_RUST_SRC file=crates/lib-a/src/lib.rs surface=build
- r3 TRANSITIVE_DEPENDS_ON_DIRECT crate=lib-b depends_on=lib-a surface=build

</details>
