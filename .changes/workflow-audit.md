---
"cargo-rail-action" = "minor"
---

The planner now audits every tracked or unignored YAML file for Cargo-Rail Action references
before planning, and `cargo-rail-action audit` runs the same audit locally.
An earlier major version, an input or output the current release does not provide,
a `needs` consumer of an output the planning job does not export, or an unparseable file stops the planner.
Exporting `plan-file` as a job output and reading a plan on another runner label produce warnings.
