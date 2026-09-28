---
"cargo-rail-action" = "patch"
---

The README no longer recommends `mode: read` for pull requests and other untrusted jobs.
A job that runs untrusted code gets no remote cache credentials and does not run the cache action,
because read access exposes every cached result to that job.
`read` is for trusted jobs that must not publish, and `read-write` for trusted jobs that seed the cache.
The README also shows how to verify a release asset's attestation with `gh attestation verify`.
