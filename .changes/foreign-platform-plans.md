---
"cargo-rail-action" = "patch"
---

Selectors and the planner now reject a plan created on another platform
before checkout verification.
The error names both platforms and tells the operator to create a plan on this platform.
The README adds a grouped-job example that guards each selector with its own work ID,
and a macOS job that routes on a Linux plan but plans again before reading selectors.
The planner summary now lists the platform, source, Cargo, toolchain,
and target bindings that every selector verifies.
The summary also states whether portable evidence was supplied,
and each widened item names the changed files that lack negative evidence.
