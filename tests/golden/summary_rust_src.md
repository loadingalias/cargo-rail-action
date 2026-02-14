## cargo-rail plan

| | |
|---|---|
| **Version** | `0.10.0` |
| **Install** | Binary download |
| **Base** | `origin/main` |
| **Changed files** | 1 |
| **Direct crates** | 1 |
| **Transitive crates** | 1 |
| **Active surfaces** | build, test |

**Direct crates:** `lib-a`
**Transitive crates:** `lib-b`

### Why Surfaces Are Active
- `build`: 2 reason(s)
- `test`: 2 reason(s)

<details><summary>Trace details (file -> crate -> surface)</summary>

- r1 FILE_OWNS_CRATE_DIRECT file=crates/lib-a/src/lib.rs crate=lib-a
- r2 FILE_KIND_RUST_SRC file=crates/lib-a/src/lib.rs surface=build
- r3 TRANSITIVE_DEPENDS_ON_DIRECT crate=lib-b depends_on=lib-a surface=build

</details>
