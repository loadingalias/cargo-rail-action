---
"cargo-rail-action" = "patch"
---

The cache action now accepts only Cargo-Rail cache status schema 18.
It rejects the retired schema 16, which no Cargo-Rail release has emitted since v0.28.0.
The README states that each Action release supports exactly the Cargo-Rail version in its lock.
