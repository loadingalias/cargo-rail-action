---
"cargo-rail-action" = "minor"
---

The planner's `evidence` input accepts a directory and passes each `*.json` file in it to Cargo-Rail,
so a workflow can restore one evidence file per recorded work item and tolerate a cache miss.
The plan reader accepts `planning-evidence-v2` identities, which the matching Cargo-Rail release records,
and the summary reports how many evidence manifests the plan used.
The README shows how to record evidence in default-branch jobs, save it by commit,
and restore the base commit's evidence before planning.
