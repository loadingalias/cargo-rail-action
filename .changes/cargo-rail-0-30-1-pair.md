---
"cargo-rail-action" = "patch"
---

Every public action now installs Cargo-Rail 0.30.1 by default.
It resolves programs found on PATH to one spelling per file,
so a plan this Action creates on Windows now verifies with `cargo-rail rail plan --verify` from a workflow shell as well
as through `cargo-rail-action plan`.
With 0.30.0, a PATH entry with a doubled separator or an uppercase PATHEXT extension made
that direct verification reject the plan.
Plan and cache contracts are unchanged.
