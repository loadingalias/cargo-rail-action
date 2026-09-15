---
"cargo-rail-action" = "minor"
---

Add a native Linux ARM64 runtime and validate Cargo-Rail Surface readiness correctly.
Bind packaging and public defaults to one released Cargo-Rail version and commit, separate from the CI tooling pin.
Allow immutable pre-release integrations to build and authenticate the checked-out Action runtime explicitly.
Install the complete cataloged CI tool set on macOS, including Nextest.
Publish cache record paths in the Windows form accepted by GitHub artifact upload.
