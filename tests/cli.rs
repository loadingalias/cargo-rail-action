use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temporary_directory() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "cargo-rail-action-cli-test.{}.{}",
        std::process::id(),
        TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).expect("temporary directory");
    path
}

#[test]
fn rejected_plan_exits_two_without_stdout() {
    let directory = temporary_directory();
    let plan = directory.join("plan.json");
    fs::write(&plan, b"{}\n").expect("invalid plan fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-rail-action"))
        .args(["plan", "required"])
        .arg(&plan)
        .output()
        .expect("run selector");
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("::error::plan is missing"));
    fs::remove_dir_all(directory).expect("remove fixture");
}

#[test]
fn missing_plan_is_operational_and_keeps_stdout_empty() {
    let directory = temporary_directory();
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-rail-action"))
        .args(["plan", "required"])
        .arg(directory.join("missing.json"))
        .output()
        .expect("run selector");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("::error::cannot inspect plan"));
    fs::remove_dir_all(directory).expect("remove fixture");
}

#[test]
#[ignore = "requires the current Cargo-Rail source binary"]
fn source_planner_authenticates_shallow_history_and_publishes_valid_plans() {
    let output = Command::new(if cfg!(windows) { "python" } else { "python3" })
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/history.py"))
        .arg(env!("CARGO_BIN_EXE_cargo-rail-action"))
        .arg(std::env::var_os("CARGO_RAIL_TEST_BINARY").expect("source binary"))
        .output()
        .expect("run source planner history fixture");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires the current Cargo-Rail source binary"]
fn source_plan_selectors_preserve_target_coverage_and_reject_checkout_drift() {
    let binary = PathBuf::from(std::env::var_os("CARGO_RAIL_TEST_BINARY").expect("source binary"));
    let directory = temporary_directory();
    let workspace = directory.join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::write(
        workspace.join("Cargo.toml"),
        "[package]\nname = 'source-contract'\nversion = '0.1.0'\nedition = '2024'\n[lib]\npath = 'lib.rs'\n",
    )
    .unwrap();
    fs::write(
        workspace.join("lib.rs"),
        "mod helper;\npub fn value() -> u8 { helper::value() }\n",
    )
    .unwrap();
    fs::write(workspace.join("helper.rs"), "pub fn value() -> u8 { 7 }\n").unwrap();
    fs::create_dir(workspace.join("tests")).unwrap();
    fs::write(workspace.join("tests/integration.rs"), "#[test]\nfn integration() {}\n").unwrap();
    fs::write(workspace.join(".gitignore"), "target/\n").unwrap();
    fs::create_dir(workspace.join(".config")).unwrap();
    fs::write(
        workspace.join(".config/rail.toml"),
        "[plan.work.compatibility]\nscope = 'variants'\nvariant_catalog = 'variants.json'\n",
    )
    .unwrap();
    fs::write(
        workspace.join("variants.json"),
        serde_json::to_vec(&serde_json::json!({
            "variant_catalog_version": 2,
            "work": "compatibility",
            "variants": [
                {"id": "linux", "dimensions": {"family": "native", "runner": "ubuntu-latest"}, "external_paths": ["linux.txt"]},
                {"id": "windows", "dimensions": {"family": "native", "runner": "windows-latest"}, "external_paths": ["windows.txt"]}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(workspace.join("linux.txt"), "before\n").unwrap();
    fs::write(workspace.join("windows.txt"), "before\n").unwrap();
    let lockfile = Command::new("cargo")
        .current_dir(&workspace)
        .args(["generate-lockfile", "--offline"])
        .output()
        .unwrap();
    assert!(lockfile.status.success(), "{lockfile:?}");
    for arguments in [
        vec!["init", "--initial-branch=main"],
        vec!["add", "."],
        vec![
            "-c",
            "user.name=Contract Test",
            "-c",
            "user.email=contract@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "Initial fixture",
        ],
    ] {
        let output = Command::new("git")
            .current_dir(&workspace)
            .args(arguments)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    let plan = Command::new(&binary)
        .current_dir(&workspace)
        .args(["rail", "plan", "--since", "HEAD", "--all", "--json"])
        .output()
        .expect("source plan");
    assert!(plan.status.success(), "{plan:?}");
    let value: serde_json::Value = serde_json::from_slice(&plan.stdout).unwrap();
    let plan_path = directory.join("plan.json");
    fs::write(&plan_path, plan.stdout).unwrap();
    let path = std::env::join_paths(
        std::iter::once(binary.parent().unwrap().to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())),
    )
    .unwrap();
    let select = |operation: &str, work: Option<&str>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-rail-action"));
        command
            .current_dir(&workspace)
            .env("PATH", &path)
            .args(["plan", operation])
            .arg(&plan_path);
        if let Some(work) = work {
            command.arg(work);
        }
        command.output().expect("source plan selector")
    };
    let accepted = select("required", None);
    assert!(accepted.status.success(), "{accepted:?}");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&accepted.stdout).unwrap(),
        value["required"]
    );
    for (operation, work, expected) in [
        ("is-required", "cargo.test", b"true\n".as_slice()),
        ("cargo-scope", "cargo.test", b"workspace\n".as_slice()),
        ("cargo-args", "cargo.test", b"".as_slice()),
        ("package-names", "cargo.test", b"".as_slice()),
        ("target-args", "cargo.test", b"".as_slice()),
        ("matrix", "compatibility", b"all\n".as_slice()),
    ] {
        let selected = select(operation, Some(work));
        assert_eq!(selected.status.code(), Some(0), "{operation}: {selected:?}");
        assert_eq!(selected.stdout, expected, "{operation}");
    }
    for operation in ["cargo-args", "package-names", "target-args"] {
        let rejected = select(operation, Some("compatibility"));
        assert_eq!(rejected.status.code(), Some(2), "{operation}: {rejected:?}");
        assert!(rejected.stdout.is_empty(), "{operation}: {rejected:?}");
        assert!(String::from_utf8_lossy(&rejected.stderr).contains("does not have Cargo scope"));
    }
    fs::write(workspace.join("helper.rs"), "pub fn value() -> u8 { 8 }\n").unwrap();
    fs::write(
        workspace.join("tests/integration.rs"),
        "#[test]\nfn integration() { assert_eq!(2 + 2, 4); }\n",
    )
    .unwrap();
    let mixed = Command::new(&binary)
        .current_dir(&workspace)
        .args(["rail", "plan", "--since", "HEAD", "--json"])
        .output()
        .expect("mixed-target source plan");
    assert!(mixed.status.success(), "{mixed:?}");
    let mixed_value: serde_json::Value = serde_json::from_slice(&mixed.stdout).unwrap();
    assert_eq!(
        mixed_value["work"]["cargo.test"]["scope"]["selection"]["targets"],
        serde_json::json!([]),
        "partial exact-target coverage must widen to package scope"
    );
    fs::write(&plan_path, mixed.stdout).unwrap();
    let targets = select("target-args", Some("cargo.test"));
    assert_eq!(targets.status.code(), Some(0), "{targets:?}");
    assert!(
        targets.stdout.is_empty(),
        "mixed target selection narrowed work: {targets:?}"
    );
    // Ordinary Cargo runs the emitted selection with every compiler wrapper disabled.
    let cargo_args = select("cargo-args", Some("cargo.test"));
    assert_eq!(cargo_args.status.code(), Some(0), "{cargo_args:?}");
    let tested = Command::new("cargo")
        .current_dir(&workspace)
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .env("CARGO_TARGET_DIR", directory.join("target"))
        .arg("test")
        .args(
            cargo_args
                .stdout
                .split(|byte| *byte == 0)
                .filter(|argument| !argument.is_empty())
                .map(|argument| std::str::from_utf8(argument).expect("UTF-8 Cargo argument")),
        )
        .arg("--locked")
        .output()
        .expect("ordinary Cargo test");
    let tested_stdout = String::from_utf8_lossy(&tested.stdout);
    assert!(tested.status.success(), "{tested:?}");
    assert!(
        tested_stdout.contains("test integration ... ok"),
        "selected integration test did not run: {tested_stdout}"
    );
    fs::write(workspace.join("linux.txt"), "after\n").unwrap();
    let changed = Command::new(&binary)
        .current_dir(&workspace)
        .args(["rail", "plan", "--since", "HEAD", "--json"])
        .output()
        .expect("source variant plan");
    assert_eq!(changed.status.code(), Some(0), "{changed:?}");
    fs::write(&plan_path, changed.stdout).unwrap();
    let matrix = select("matrix", Some("compatibility"));
    assert_eq!(matrix.status.code(), Some(0), "{matrix:?}");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&matrix.stdout).unwrap(),
        serde_json::json!({"include": [{"id": "linux", "runner": "ubuntu-latest"}]})
    );
    fs::write(workspace.join("lib.rs"), "pub fn value() -> u8 { 9 }\n").unwrap();
    let rejected = select("required", None);
    assert_eq!(rejected.status.code(), Some(2), "{rejected:?}");
    assert!(rejected.stdout.is_empty(), "{rejected:?}");
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("cargo-rail rejected current execution authority"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "requires the current Cargo-Rail source binary"]
fn source_plan_selectors_qualify_empty_precise_widened_and_drifted_plans() {
    let binary = PathBuf::from(std::env::var_os("CARGO_RAIL_TEST_BINARY").expect("source binary"));
    let directory = temporary_directory();
    let workspace = directory.join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::write(
        workspace.join("Cargo.toml"),
        "[workspace]\nmembers = ['alpha', 'leaf']\nresolver = '2'\n",
    )
    .unwrap();
    for (name, dependency) in [("alpha", ""), ("leaf", "alpha = { path = '../alpha' }\n")] {
        fs::create_dir_all(workspace.join(name).join("src")).unwrap();
        fs::write(
            workspace.join(name).join("Cargo.toml"),
            format!("[package]\nname = '{name}'\nversion = '0.1.0'\nedition = '2024'\n[dependencies]\n{dependency}"),
        )
        .unwrap();
        fs::write(workspace.join(name).join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n").unwrap();
    }
    fs::write(workspace.join("README.md"), "# Fixture\n").unwrap();
    fs::write(workspace.join(".gitignore"), "target/\n").unwrap();
    let lockfile = Command::new("cargo")
        .current_dir(&workspace)
        .args(["generate-lockfile", "--offline"])
        .output()
        .unwrap();
    assert!(lockfile.status.success(), "{lockfile:?}");
    for arguments in [
        vec!["init", "--initial-branch=main"],
        vec!["add", "."],
        vec![
            "-c",
            "user.name=Contract Test",
            "-c",
            "user.email=contract@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "Initial fixture",
        ],
    ] {
        let output = Command::new("git")
            .current_dir(&workspace)
            .args(arguments)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    let path = std::env::join_paths(
        std::iter::once(binary.parent().unwrap().to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())),
    )
    .unwrap();
    // The runner-local plan lives outside every checkout that reads it.
    let plan_path = directory.join("plan.json");
    let create_plan = || {
        let output = Command::new(&binary)
            .current_dir(&workspace)
            .args(["rail", "plan", "--since", "HEAD", "--json"])
            .output()
            .expect("source plan");
        assert!(output.status.success(), "{output:?}");
        fs::write(&plan_path, &output.stdout).unwrap();
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    let select_in = |checkout: &std::path::Path, environment: &[(&str, &std::ffi::OsStr)], arguments: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-rail-action"));
        command.current_dir(checkout).env("PATH", &path).arg("plan");
        command.arg(arguments[0]).arg(&plan_path).args(&arguments[1..]);
        for (name, value) in environment {
            command.env(name, value);
        }
        command.output().expect("source plan selector")
    };
    let select = |arguments: &[&str]| select_in(&workspace, &[], arguments);
    let accepted = |arguments: &[&str]| {
        let output = select(arguments);
        assert_eq!(output.status.code(), Some(0), "{arguments:?}: {output:?}");
        output.stdout
    };
    let rejected = |output: std::process::Output, subject: &str| {
        assert_eq!(output.status.code(), Some(2), "{subject} was accepted: {output:?}");
        assert!(output.stdout.is_empty(), "{subject} emitted stdout: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("create a new plan"),
            "{subject}: {output:?}"
        );
    };

    // Empty work: nothing is required and Cargo selectors emit nothing.
    let empty = create_plan();
    assert_eq!(empty["required"], serde_json::json!([]));
    assert_eq!(accepted(&["required"]), b"[]\n");
    assert_eq!(accepted(&["is-required", "cargo.test"]), b"false\n");
    assert_eq!(accepted(&["cargo-scope", "cargo.test"]), b"skipped\n");
    assert_eq!(accepted(&["cargo-args", "cargo.test"]), b"");
    assert!(
        String::from_utf8(accepted(&["summary"]))
            .unwrap()
            .contains("No work required.")
    );

    // Precise scope: a leaf change selects only the leaf package.
    fs::write(workspace.join("leaf/src/lib.rs"), "pub fn value() -> u8 { 2 }\n").unwrap();
    let precise = create_plan();
    assert_eq!(precise["work"]["cargo.test"]["cause"], "changed_input");
    assert_eq!(accepted(&["cargo-scope", "cargo.test"]), b"packages\n");
    assert_eq!(accepted(&["cargo-args", "cargo.test"]), b"-p\0leaf\0");
    assert_eq!(accepted(&["package-names", "cargo.test"]), b"leaf\0");

    // Untracked drift after planning rejects the saved plan.
    fs::write(workspace.join("leaf/src/extra.rs"), "pub fn extra() {}\n").unwrap();
    rejected(select(&["cargo-args", "cargo.test"]), "untracked drift");
    fs::remove_file(workspace.join("leaf/src/extra.rs")).unwrap();
    assert_eq!(accepted(&["cargo-args", "cargo.test"]), b"-p\0leaf\0");

    // A different compiler rejects the saved plan.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let sysroot = Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
        assert!(sysroot.status.success(), "{sysroot:?}");
        let rustc = PathBuf::from(String::from_utf8(sysroot.stdout).unwrap().trim()).join("bin/rustc");
        let wrapper = directory.join("rustc-wrapper");
        fs::write(&wrapper, format!("#!/bin/sh\nexec '{}' \"$@\"\n", rustc.display())).unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
        rejected(
            select_in(
                &workspace,
                &[("RUSTC", wrapper.as_os_str())],
                &["cargo-args", "cargo.test"],
            ),
            "compiler drift",
        );
    }

    // Relocation: the same commit and worktree at another path verifies.
    fs::write(workspace.join("leaf/src/lib.rs"), "pub fn value() -> u8 { 1 }\n").unwrap();
    create_plan();
    let clone = directory.join("relocated");
    let cloned = Command::new("git")
        .arg("clone")
        .arg("--quiet")
        .arg(&workspace)
        .arg(&clone)
        .output()
        .unwrap();
    assert!(cloned.status.success(), "{cloned:?}");
    let relocated = select_in(&clone, &[], &["required"]);
    assert_eq!(relocated.status.code(), Some(0), "{relocated:?}");
    assert_eq!(relocated.stdout, b"[]\n");

    // Evidence recorded from the ordinary test build of HEAD.
    let evidence = workspace.join("target/planning-evidence/cargo.test.json");
    let recorded = Command::new(&binary)
        .current_dir(&workspace)
        .args(["rail", "plan", "evidence", "--work", "cargo.test", "--output"])
        .arg(&evidence)
        .args(["--", "test", "--workspace", "--no-run", "--offline"])
        .output()
        .expect("record planning evidence");
    assert!(recorded.status.success(), "{recorded:?}");

    // Without compatible evidence, an unrelated file widens Cargo work explicitly.
    fs::write(workspace.join("README.md"), "# Changed fixture\n").unwrap();
    let widened = create_plan();
    assert_eq!(widened["work"]["cargo.test"]["cause"], "incomplete_evidence");
    assert_eq!(accepted(&["is-required", "cargo.test"]), b"true\n");
    assert_eq!(accepted(&["cargo-scope", "cargo.test"]), b"workspace\n");
    assert_eq!(accepted(&["cargo-args", "cargo.test"]), b"");
    let summary = String::from_utf8(accepted(&["summary"])).unwrap();
    assert!(summary.contains("**Scope expanded:**"), "{summary}");

    // The recorded evidence proves the same change unrelated, and the reader accepts its identity.
    let output = Command::new(&binary)
        .current_dir(&workspace)
        .args(["rail", "plan", "--since", "HEAD", "--json", "--evidence"])
        .arg(&evidence)
        .output()
        .expect("plan with evidence");
    assert!(output.status.success(), "{output:?}");
    fs::write(&plan_path, &output.stdout).unwrap();
    let skipped = serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap();
    assert_eq!(skipped["work"]["cargo.test"]["state"], "skipped");
    assert_eq!(accepted(&["is-required", "cargo.test"]), b"false\n");
    let summary = String::from_utf8(accepted(&["summary"])).unwrap();
    assert!(summary.contains("Portable evidence: 1 manifest used."), "{summary}");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn documented_selectors_stop_before_execution_or_publication_on_rejection() {
    fn scripts(value: &serde_json::Value, found: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(object) => {
                if let Some(run) = object.get("run").and_then(serde_json::Value::as_str)
                    && (run.contains("cargo nextest run") || run.contains("plan matrix"))
                {
                    assert_eq!(object["shell"], "bash");
                    found.push(run.to_string());
                }
                for child in object.values() {
                    scripts(child, found);
                }
            }
            serde_json::Value::Array(array) => {
                for child in array {
                    scripts(child, found);
                }
            }
            _ => {}
        }
    }
    let directory = temporary_directory();
    let plan = directory.join("plan.json");
    let published = directory.join("outputs");
    let executed = directory.join("executed");
    fs::write(&plan, b"{}\n").expect("invalid fixture");
    fs::write(&published, b"").expect("output fixture");
    let mut examples = Vec::new();
    for block in include_str!("../README.md").split("```yaml").skip(1) {
        let yaml = block.split("```").next().expect("code block");
        let value: serde_json::Value = serde_saphyr::from_str(yaml).expect("README YAML");
        scripts(&value, &mut examples);
    }
    assert_eq!(examples.len(), 4, "every documented Cargo selector example");
    for example in examples {
        let script = format!(
            "cargo-rail-action() {{ \"$RUNTIME\" \"$@\"; }}\ncargo() {{ printf invoked > \"$EXECUTED\"; }}\n{example}"
        );
        let bash = std::env::var_os("CARGO_RAIL_TEST_BASH").unwrap_or_else(|| "bash".into());
        let output = Command::new(bash)
            .args(["-euo", "pipefail", "-c", &script])
            .env("RUNTIME", env!("CARGO_BIN_EXE_cargo-rail-action"))
            .env("PLAN_FILE", &plan)
            .env("RUNNER_TEMP", &directory)
            .env("GITHUB_OUTPUT", &published)
            .env("EXECUTED", &executed)
            .output()
            .expect("run README example");
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(!executed.exists(), "rejected selector reached Cargo");
        assert!(fs::read(&published).expect("output file").is_empty());
    }
    fs::remove_dir_all(directory).expect("remove fixture");
}

#[test]
fn documented_grouped_jobs_guard_each_selector_with_its_own_work() {
    fn jobs<'a>(value: &'a serde_json::Value, found: &mut Vec<&'a serde_json::Map<String, serde_json::Value>>) {
        match value {
            serde_json::Value::Object(object) => {
                if object.get("steps").is_some_and(serde_json::Value::is_array) {
                    found.push(object);
                }
                object.values().for_each(|child| jobs(child, found));
            }
            serde_json::Value::Array(array) => array.iter().for_each(|child| jobs(child, found)),
            _ => {}
        }
    }
    fn work_ids(condition: &str) -> Vec<&str> {
        condition.split('\'').skip(1).step_by(2).collect()
    }
    let blocks = include_str!("../README.md")
        .split("```yaml")
        .skip(1)
        .map(|block| serde_saphyr::from_str::<serde_json::Value>(block.split("```").next().unwrap()).unwrap())
        .collect::<Vec<_>>();
    let mut found = Vec::new();
    blocks.iter().for_each(|block| jobs(block, &mut found));
    let mut grouped = 0;
    for job in found {
        let job_condition = job.get("if").and_then(serde_json::Value::as_str).unwrap_or("");
        let owned = work_ids(job_condition);
        grouped += usize::from(owned.len() > 1);
        for step in job["steps"].as_array().unwrap() {
            let step_condition = step.get("if").and_then(serde_json::Value::as_str).unwrap_or("");
            for guarded in work_ids(step_condition) {
                assert!(
                    job_condition.is_empty() || owned.contains(&guarded),
                    "job condition omits {guarded}: {job_condition}"
                );
            }
            let Some(run) = step.get("run").and_then(serde_json::Value::as_str) else {
                continue;
            };
            for line in run.lines().filter(|line| line.contains("cargo-rail-action plan ")) {
                let words = line.split_whitespace().collect::<Vec<_>>();
                let plan = words.iter().position(|word| *word == "\"$PLAN_FILE\"").unwrap();
                let work = words[plan + 1];
                assert!(
                    work_ids(step_condition).contains(&work) || owned == [work],
                    "selector for {work} is not guarded by its own work: {step_condition:?} in job {job_condition:?}"
                );
            }
        }
    }
    assert!(grouped > 0, "README documents a grouped job");
}

#[test]
fn workflow_report_publishes_one_summary_and_marks_missing_jobs() {
    let directory = temporary_directory();
    let records = directory.join("records");
    fs::create_dir(&records).unwrap();
    let summary = directory.join("summary");
    fs::write(&summary, b"").unwrap();
    let record = serde_json::json!({
        "schema_version": 1, "run": {"repository": "owner/project", "run_id": "123", "attempt": "1"}, "job": "linux",
        "configuration": {"schema_version": 1, "cargo_rail": "0.26.0", "provider": "aws-s3", "mode": "read", "max_bytes": 1024,
            "root_portability": "physical", "remote_verification": "not_requested"},
        "measurements": {"hits": 9, "misses": 1, "bypasses": 0, "failures": 0, "local_bytes_read": 10,
            "remote_bytes_read": 20, "remote_bytes_written": 0, "bypass_reasons": {}, "failure_reasons": {}, "incomplete": false},
        "storage": {"bytes": 512, "max_bytes": 1024}, "gaps": []
    });
    fs::write(records.join("linux.json"), serde_json::to_vec(&record).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-rail-action"))
        .args(["run", "cache-report"])
        .env("GITHUB_REPOSITORY", "owner/project")
        .env("GITHUB_RUN_ID", "123")
        .env("GITHUB_RUN_ATTEMPT", "1")
        .env("GITHUB_STEP_SUMMARY", &summary)
        .env("INPUT_RECORDS_DIRECTORY", &records)
        .env("INPUT_EXPECTED_JOBS", r#"["linux","macos"]"#)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let summary = fs::read_to_string(summary).unwrap();
    assert_eq!(summary.matches("## Cargo-Rail cache").count(), 1);
    assert!(summary.contains("Reused: 9 · Misses: 1"));
    assert!(summary.contains("Missing job reports:** macos"));
    assert!(!summary.contains("Reused: 0"));
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn bootstrap_downloads_runtime_only_when_installation_is_absent() {
    use std::os::unix::fs::PermissionsExt;

    let directory = temporary_directory();
    let fixtures = directory.join("fixtures");
    let bin = directory.join("bin");
    let runner_temp = directory.join("runner");
    let cache = directory.join("cache");
    for path in [&fixtures, &bin, &runner_temp, &cache] {
        fs::create_dir(path).unwrap();
    }
    let runtime = b"#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$INVOCATIONS\"\n";
    let digest: String = rscrypto::Sha256::digest(runtime)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let version = env!("CARGO_PKG_VERSION");
    let manifest_name = "cargo-rail-action-runtime-v1.tsv";
    let mut manifest = format!("cargo-rail-action-runtime-v1\t{version}\n");
    for target in [
        "aarch64-apple-darwin",
        "aarch64-unknown-linux-gnu",
        "x86_64-pc-windows-msvc",
        "x86_64-unknown-linux-gnu",
    ] {
        let suffix = if target.contains("windows") { ".exe" } else { "" };
        let asset = format!("cargo-rail-action-{target}{suffix}");
        manifest.push_str(&format!("{target}\t{asset}\t{}\t{digest}\n", runtime.len()));
        fs::write(fixtures.join(asset), runtime).unwrap();
    }
    fs::write(fixtures.join(manifest_name), manifest).unwrap();
    let curl = bin.join("curl");
    fs::write(
        &curl,
        r#"#!/bin/bash
set -euo pipefail
output=''
while (( $# > 0 )); do
  case "$1" in
    --output) output="$2"; shift 2 ;;
    *) url="$1"; shift ;;
  esac
done
asset="${url##*/}"
printf '%s\n' "$url" >> "$DOWNLOADS"
cp "$FIXTURES/$asset" "$output"
"#,
    )
    .unwrap();
    fs::set_permissions(&curl, fs::Permissions::from_mode(0o700)).unwrap();
    let downloads = directory.join("downloads");
    let invocations = directory.join("invocations");
    let github_path = directory.join("github-path");
    fs::write(&github_path, "").unwrap();
    let path =
        std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())))
            .unwrap();
    let run = |runner_os: &str, runner_arch: &str| {
        Command::new("bash")
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/bootstrap.sh"))
            .args(["run", "planner"])
            .env("PATH", &path)
            .env("RUNNER_OS", runner_os)
            .env("RUNNER_ARCH", runner_arch)
            .env("RUNNER_TEMP", &runner_temp)
            .env("RUNNER_TOOL_CACHE", &cache)
            .env("FIXTURES", &fixtures)
            .env("DOWNLOADS", &downloads)
            .env("INVOCATIONS", &invocations)
            .env("GITHUB_PATH", &github_path)
            .output()
            .unwrap()
    };
    let asset = "cargo-rail-action-aarch64-apple-darwin";
    let release_url = format!("https://github.com/loadingalias/cargo-rail-action/releases/download/v{version}");
    let first = run("macOS", "ARM64");
    assert!(first.status.success(), "{first:?}");
    assert_eq!(
        fs::read_to_string(&downloads).unwrap(),
        format!("{release_url}/{manifest_name}\n{release_url}/{asset}\n")
    );
    fs::write(&downloads, "").unwrap();
    let reused = run("macOS", "ARM64");
    assert!(reused.status.success(), "{reused:?}");
    assert_eq!(
        fs::read_to_string(&downloads).unwrap(),
        format!("{release_url}/{manifest_name}\n")
    );
    let calls = fs::read_to_string(&invocations).unwrap();
    let expected_calls =
        format!("self-check --expect-version {version} --expect-target aarch64-apple-darwin\nrun planner\n");
    assert_eq!(calls, expected_calls.repeat(2));

    let published_paths = fs::read_to_string(&github_path).unwrap();
    let command_directories = published_paths.lines().map(PathBuf::from).collect::<Vec<_>>();
    assert_eq!(command_directories.len(), 2);
    for command_directory in &command_directories {
        let launcher = command_directory.join("cargo-rail-action");
        let metadata = fs::symlink_metadata(&launcher).unwrap();
        assert!(metadata.is_file() && !metadata.file_type().is_symlink());
    }
    let selector_path = std::env::join_paths(
        std::iter::once(command_directories.last().unwrap().clone()).chain(std::env::split_paths(&path)),
    )
    .unwrap();
    let selector = Command::new("cargo-rail-action")
        .args(["plan", "is-required", "plan.json", "cargo.test"])
        .env("PATH", selector_path)
        .env("INVOCATIONS", &invocations)
        .output()
        .unwrap();
    assert!(selector.status.success(), "{selector:?}");
    let calls = format!("{}plan is-required plan.json cargo.test\n", expected_calls.repeat(2));
    assert_eq!(fs::read_to_string(&invocations).unwrap(), calls);

    let installed = cache.join(format!(
        "cargo-rail-action/runtime/{version}/aarch64-apple-darwin-{digest}/{asset}"
    ));
    let license = installed.parent().unwrap().join("LICENSE");
    let license_bytes = include_bytes!("../LICENSE");
    assert_eq!(fs::read(&license).unwrap(), license_bytes);
    for invalid in [Some(b"changed license".as_slice()), None] {
        if let Some(bytes) = invalid {
            fs::write(&license, bytes).unwrap();
        } else {
            fs::remove_file(&license).unwrap();
        }
        let rejected = run("macOS", "ARM64");
        assert!(!rejected.status.success(), "{rejected:?}");
        assert!(String::from_utf8_lossy(&rejected.stderr).contains("immutable runtime destination is corrupt"));
        assert_eq!(fs::read_to_string(&invocations).unwrap(), calls);
        fs::write(&license, license_bytes).unwrap();
    }
    let mut corrupt = runtime.to_vec();
    corrupt[0] = b'x';
    fs::write(installed, corrupt).unwrap();
    fs::write(&downloads, "").unwrap();
    let rejected = run("macOS", "ARM64");
    assert!(!rejected.status.success(), "{rejected:?}");
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("immutable runtime destination is corrupt"));
    assert_eq!(
        fs::read_to_string(&downloads).unwrap(),
        format!("{release_url}/{manifest_name}\n")
    );
    assert_eq!(fs::read_to_string(&invocations).unwrap(), calls);
    #[cfg(target_os = "linux")]
    {
        fs::write(&downloads, "").unwrap();
        let arm = run("Linux", "ARM64");
        assert!(arm.status.success(), "{arm:?}");
        let arm_asset = "cargo-rail-action-aarch64-unknown-linux-gnu";
        assert_eq!(
            fs::read_to_string(&downloads).unwrap(),
            format!("{release_url}/{manifest_name}\n{release_url}/{arm_asset}\n")
        );
        assert!(fs::read_to_string(&invocations).unwrap().ends_with(&format!(
            "self-check --expect-version {version} --expect-target aarch64-unknown-linux-gnu\nrun planner\n"
        )));
    }
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn bootstrap_builds_an_explicit_source_runtime_without_release_downloads() {
    use std::os::unix::fs::PermissionsExt;

    let directory = temporary_directory();
    let bin = directory.join("bin");
    let runner_temp = directory.join("runner");
    let cache = directory.join("cache");
    for path in [&bin, &runner_temp, &cache] {
        fs::create_dir(path).unwrap();
    }

    let curl = bin.join("curl");
    fs::write(&curl, "#!/bin/sh\nprintf called > \"$CURL_CALLED\"\nexit 99\n").unwrap();
    fs::set_permissions(&curl, fs::Permissions::from_mode(0o700)).unwrap();

    let curl_called = directory.join("curl-called");
    let github_path = directory.join("github-path");
    fs::write(&github_path, "").unwrap();
    let path =
        std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())))
            .unwrap();
    let (runner_os, runner_arch, target) = if cfg!(target_os = "macos") {
        ("macOS", "ARM64", "aarch64-apple-darwin")
    } else if cfg!(target_arch = "aarch64") {
        ("Linux", "ARM64", "aarch64-unknown-linux-gnu")
    } else {
        ("Linux", "X64", "x86_64-unknown-linux-gnu")
    };
    let version = env!("CARGO_PKG_VERSION");
    let run = |mode: &str| {
        Command::new("bash")
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/bootstrap.sh"))
            .args(["self-check", "--expect-version", version, "--expect-target", target])
            .env("PATH", &path)
            .env("RUNNER_OS", runner_os)
            .env("RUNNER_ARCH", runner_arch)
            .env("RUNNER_TEMP", &runner_temp)
            .env("RUNNER_TOOL_CACHE", &cache)
            .env("CURL_CALLED", &curl_called)
            .env("GITHUB_PATH", &github_path)
            .env("CARGO_RAIL_ACTION_RUNTIME_SOURCE", mode)
            .output()
            .unwrap()
    };

    let output = run("source");
    assert!(output.status.success(), "{output:?}");
    assert!(!curl_called.exists());
    let command_directory = fs::read_to_string(&github_path).unwrap();
    let launcher = PathBuf::from(command_directory.trim()).join("cargo-rail-action");
    assert!(launcher.is_file());

    let invalid = run("checkout");
    assert!(!invalid.status.success(), "{invalid:?}");
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("runtime-source must be release or source"));
    assert!(!curl_called.exists());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn runtime_packaging_requires_the_complete_qualified_set_and_exact_license() {
    let directory = temporary_directory();
    let mut assets = Vec::new();
    for name in [
        "cargo-rail-action-aarch64-apple-darwin",
        "cargo-rail-action-aarch64-unknown-linux-gnu",
        "cargo-rail-action-x86_64-pc-windows-msvc.exe",
        "cargo-rail-action-x86_64-unknown-linux-gnu",
        "LICENSE",
    ] {
        let path = directory.join(name);
        fs::write(
            &path,
            if name == "LICENSE" {
                include_bytes!("../LICENSE").as_slice()
            } else {
                b"qualified runtime fixture"
            },
        )
        .unwrap();
        assets.push(path);
    }
    let manifest = directory.join("cargo-rail-action-runtime-v1.tsv");
    let invoke = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-rail-action"));
        command.args(["release", "package", "--output"]).arg(&manifest);
        for asset in &assets {
            command.arg("--asset").arg(asset);
        }
        command.output().unwrap()
    };
    fs::write(directory.join("LICENSE"), "wrong source license").unwrap();
    let rejected = invoke();
    assert_eq!(rejected.status.code(), Some(2), "{rejected:?}");
    assert!(!manifest.exists());
    fs::write(directory.join("LICENSE"), include_bytes!("../LICENSE")).unwrap();
    let completed = invoke();
    assert!(completed.status.success(), "{completed:?}");
    let contents = fs::read_to_string(&manifest).unwrap();
    assert_eq!(contents.lines().count(), 5);
    assert!(contents.starts_with(&format!(
        "cargo-rail-action-runtime-v1\t{}\n",
        env!("CARGO_PKG_VERSION")
    )));
    for (row, path) in contents.lines().skip(1).zip(&assets) {
        let fields = row.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 4);
        assert_eq!(fields[1], path.file_name().unwrap().to_str().unwrap());
        assert_eq!(fields[2], "25");
    }
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
#[ignore = "requires the current Cargo-Rail source binary"]
fn source_release_runner_releases_through_the_engine_and_publishes_its_outputs() {
    let core = PathBuf::from(std::env::var_os("CARGO_RAIL_TEST_BINARY").expect("source core"));
    let version = String::from_utf8(Command::new(&core).arg("--version").output().unwrap().stdout).unwrap();
    let version = version
        .trim()
        .strip_prefix("cargo-rail ")
        .expect("core version")
        .to_owned();
    let directory = temporary_directory();
    let workspace = directory.join("workspace");
    fs::create_dir_all(workspace.join(".config")).unwrap();
    fs::create_dir(workspace.join(".changes")).unwrap();
    fs::write(
        workspace.join("Cargo.toml"),
        "[package]\nname='runner-fixture'\nversion='0.1.0'\nedition='2024'\n[lib]\npath='lib.rs'\n",
    )
    .unwrap();
    fs::write(workspace.join("lib.rs"), "pub fn value() -> u8 { 1 }\n").unwrap();
    fs::write(workspace.join(".gitignore"), "target/\n").unwrap();
    fs::write(
        workspace.join(".config/rail.toml"),
        "[release]\npublish = 'none'\nsemver_check = 'off'\n",
    )
    .unwrap();
    let cargo = Command::new("cargo")
        .current_dir(&workspace)
        .args(["generate-lockfile", "--offline"])
        .output()
        .unwrap();
    assert!(cargo.status.success(), "{cargo:?}");
    let git = |root: &std::path::Path, args: &[&str]| {
        let result = Command::new("git")
            .current_dir(root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid"])
            .args(args)
            .output()
            .unwrap();
        assert!(result.status.success(), "{args:?}: {result:?}");
        String::from_utf8(result.stdout).unwrap().trim().to_owned()
    };
    let remote = directory.join("origin.git");
    git(
        &directory,
        &[
            "init",
            "--quiet",
            "--bare",
            "--initial-branch=main",
            remote.to_str().unwrap(),
        ],
    );
    git(&workspace, &["init", "--quiet", "--initial-branch=main"]);
    git(&workspace, &["add", "."]);
    git(&workspace, &["commit", "--quiet", "-m", "Initial workspace"]);
    git(
        &workspace,
        &["tag", "-a", "runner-fixture-v0.1.0", "-m", "Initial release"],
    );
    git(&workspace, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(
        &workspace,
        &["push", "--quiet", "--set-upstream", "origin", "main", "--tags"],
    );
    fs::write(
        workspace.join(".changes/fix.md"),
        "---\nrunner-fixture = 'patch'\n---\n\nFix the value.\n",
    )
    .unwrap();
    fs::write(workspace.join("lib.rs"), "pub fn value() -> u8 { 2 }\n").unwrap();
    git(&workspace, &["add", "."]);
    git(&workspace, &["commit", "--quiet", "-m", "Fix the value"]);
    git(&workspace, &["push", "--quiet"]);

    let run = |dry_run: bool| {
        let output_file = directory.join(format!("output-{dry_run}"));
        let summary = directory.join(format!("summary-{dry_run}"));
        let path_file = directory.join(format!("path-{dry_run}"));
        for file in [&output_file, &summary, &path_file] {
            fs::write(file, "").unwrap();
        }
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-rail-action"));
        command
            .args(["release", "execute", "--cargo-rail"])
            .arg(&core)
            .args(["--version", &version, "--bump", "auto"])
            .current_dir(&workspace)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GITHUB_ACTIONS", "true")
            .env("GITHUB_WORKSPACE", &workspace)
            .env("GITHUB_SERVER_URL", "https://github.com")
            .env("GITHUB_OUTPUT", &output_file)
            .env("GITHUB_STEP_SUMMARY", &summary)
            .env("GITHUB_PATH", &path_file)
            .env("GH_TOKEN", "fixture-token");
        if dry_run {
            command.arg("--dry-run");
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "{output:?}");
        (
            fs::read_to_string(output_file).unwrap(),
            fs::read_to_string(summary).unwrap(),
        )
    };

    let (planned, _) = run(true);
    assert!(planned.contains("state=planned"), "{planned}");
    assert!(planned.contains("tags=[\"runner-fixture-v0.1.1\"]"), "{planned}");
    assert!(
        git(&remote, &["tag", "--list"])
            .lines()
            .all(|tag| tag != "runner-fixture-v0.1.1")
    );

    let (released, summary) = run(false);
    let head = git(&workspace, &["rev-parse", "HEAD"]);
    assert!(released.contains("state=released"), "{released}");
    assert!(released.contains(&format!("commit={head}")), "{released}");
    assert!(
        summary.contains("| runner-fixture | 0.1.1 | runner-fixture-v0.1.1 | skipped | skipped |"),
        "{summary}"
    );
    assert_eq!(git(&remote, &["rev-parse", "runner-fixture-v0.1.1^{commit}"]), head);
    // The commit identity came from the runner, not from global configuration.
    assert_eq!(git(&workspace, &["log", "-1", "--format=%an"]), "github-actions[bot]");

    let (again, _) = run(false);
    assert!(again.contains("state=nothing_to_release"), "{again}");
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
#[ignore = "downloads the locked Cargo-Rail release"]
fn release_setup_installs_the_locked_version_without_a_rust_toolchain() {
    let directory = temporary_directory();
    let runner_temp = directory.join("runner");
    let cache = directory.join("cache");
    for path in [&runner_temp, &cache] {
        fs::create_dir(path).unwrap();
    }
    let github_path = directory.join("github-path");
    let github_output = directory.join("github-output");
    fs::write(&github_path, "").unwrap();
    fs::write(&github_output, "").unwrap();
    let (runner_os, runner_arch, target) = if cfg!(target_os = "macos") {
        ("macOS", "ARM64", "aarch64-apple-darwin")
    } else if cfg!(target_arch = "aarch64") {
        ("Linux", "ARM64", "aarch64-unknown-linux-gnu")
    } else {
        ("Linux", "X64", "x86_64-unknown-linux-gnu")
    };
    // Building the Action runtime from source is the only step that uses Rust.
    let bootstrap = Command::new("bash")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/bootstrap.sh"))
        .args([
            "self-check",
            "--expect-version",
            env!("CARGO_PKG_VERSION"),
            "--expect-target",
            target,
        ])
        .env("RUNNER_OS", runner_os)
        .env("RUNNER_ARCH", runner_arch)
        .env("RUNNER_TEMP", &runner_temp)
        .env("RUNNER_TOOL_CACHE", &cache)
        .env("GITHUB_PATH", &github_path)
        .env("CARGO_RAIL_ACTION_RUNTIME_SOURCE", "source")
        .output()
        .unwrap();
    assert!(bootstrap.status.success(), "{bootstrap:?}");
    let launcher = PathBuf::from(fs::read_to_string(&github_path).unwrap().trim()).join("cargo-rail-action");
    let lock = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/.github/cargo-rail.lock")).unwrap();
    let locked = lock.lines().find_map(|line| line.strip_prefix("version=")).unwrap();

    // Setup runs with system tools only: no cargo, rustc, or rustup is reachable.
    let system_path = "/usr/bin:/bin";
    for tool in ["cargo", "rustc", "rustup"] {
        let found = Command::new("sh")
            .args(["-c", &format!("command -v {tool}")])
            .env("PATH", system_path)
            .output()
            .unwrap();
        assert!(!found.status.success(), "{tool} is reachable on {system_path}");
    }
    fs::write(&github_path, "").unwrap();
    let setup = Command::new(&launcher)
        .args(["run", "setup"])
        .env_clear()
        .env("PATH", system_path)
        .env("HOME", &directory)
        .env("RUNNER_OS", runner_os)
        .env("RUNNER_ARCH", runner_arch)
        .env("RUNNER_TEMP", &runner_temp)
        .env("RUNNER_TOOL_CACHE", &cache)
        .env("GITHUB_PATH", &github_path)
        .env("GITHUB_OUTPUT", &github_output)
        .env("INPUT_VERSION", locked)
        .output()
        .unwrap();
    assert!(setup.status.success(), "{setup:?}");
    assert_eq!(
        String::from_utf8_lossy(&setup.stdout).trim(),
        format!("Cargo-Rail setup ready: {locked}")
    );
    assert_eq!(
        fs::read_to_string(&github_output).unwrap(),
        format!("version={locked}\n")
    );
    let installed = fs::read_to_string(&github_path)
        .unwrap()
        .lines()
        .map(|line| PathBuf::from(line).join("cargo-rail"))
        .find(|path| path.is_file())
        .expect("installed Cargo-Rail directory on GITHUB_PATH");
    let version = Command::new(&installed).arg("--version").env_clear().output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        format!("cargo-rail {locked}")
    );

    // The installed release plans a workspace pinned to an older toolchain than any Action build uses.
    let toolchain = std::env::var("CARGO_RAIL_TEST_OLD_TOOLCHAIN").unwrap_or_else(|_| "1.91.0".to_string());
    let workspace = directory.join("workspace");
    fs::create_dir_all(workspace.join("src")).unwrap();
    fs::write(
        workspace.join("Cargo.toml"),
        format!(
            "[package]\nname = 'old-toolchain'\nversion = '0.1.0'\nedition = '2021'\nrust-version = '{}'\n",
            toolchain
                .rsplit_once('.')
                .map_or(toolchain.as_str(), |(minor, _)| minor)
        ),
    )
    .unwrap();
    fs::write(
        workspace.join("rust-toolchain.toml"),
        format!("[toolchain]\nchannel = '{toolchain}'\n"),
    )
    .unwrap();
    fs::write(workspace.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n").unwrap();
    fs::write(workspace.join(".gitignore"), "target/\n").unwrap();
    let git = |arguments: &[&str]| {
        let output = Command::new("git")
            .current_dir(&workspace)
            .args([
                "-c",
                "user.name=Setup Test",
                "-c",
                "user.email=setup@invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(arguments)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    };
    // The fixture's own rust-toolchain.toml must select its toolchain, not the test runner's.
    let lockfile = Command::new("cargo")
        .current_dir(&workspace)
        .env_remove("RUSTUP_TOOLCHAIN")
        .args(["generate-lockfile", "--offline"])
        .output()
        .unwrap();
    assert!(lockfile.status.success(), "{lockfile:?}");
    git(&["init", "-q", "-b", "main"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "base"]);
    fs::write(workspace.join("src/lib.rs"), "pub fn value() -> u8 { 2 }\n").unwrap();
    git(&["commit", "-qam", "change"]);
    let rustc = Command::new("rustc")
        .current_dir(&workspace)
        .env_remove("RUSTUP_TOOLCHAIN")
        .arg("--version")
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&rustc.stdout).contains(&format!("rustc {toolchain}")),
        "{rustc:?}"
    );
    let plan = Command::new(&installed)
        .current_dir(&workspace)
        .env_remove("RUSTUP_TOOLCHAIN")
        .env_remove("CARGO")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .args(["rail", "plan", "--since", "HEAD~1", "--json"])
        .output()
        .unwrap();
    assert!(plan.status.success(), "{plan:?}");
    let plan: serde_json::Value = serde_json::from_slice(&plan.stdout).unwrap();
    assert_eq!(plan["work"]["cargo.test"]["state"], "required");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn planner_stops_on_a_legacy_output_consumer_before_installing() {
    let directory = temporary_directory();
    let workspace = directory.join("workspace");
    fs::create_dir_all(workspace.join(".github/workflows")).unwrap();
    fs::write(
        workspace.join("Cargo.toml"),
        "[package]\nname = 'audit'\nversion = '0.1.0'\nedition = '2021'\n",
    )
    .unwrap();
    fs::write(
        workspace.join(".github/workflows/ci.yml"),
        "jobs:\n  plan:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: loadingalias/cargo-rail-action@current\n        id: rail\n      - if: steps.rail.outputs.test == 'true'\n        run: cargo test\n"
            .replace("@current", &format!("@v{}", env!("CARGO_PKG_VERSION").split('.').next().unwrap())),
    )
    .unwrap();
    let git = Command::new("git")
        .current_dir(&workspace)
        .args(["init", "-q", "-b", "main"])
        .output()
        .unwrap();
    assert!(git.status.success(), "{git:?}");
    let runner_temp = directory.join("runner");
    fs::create_dir(&runner_temp).unwrap();
    // A reachable curl would record that installation started.
    let bin = directory.join("bin");
    fs::create_dir(&bin).unwrap();
    let curl_called = directory.join("curl-called");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let curl = bin.join("curl");
        fs::write(&curl, "#!/bin/sh\nprintf called > \"$CURL_CALLED\"\nexit 99\n").unwrap();
        fs::set_permissions(&curl, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let path =
        std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())))
            .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-rail-action"))
        .args(["run", "planner"])
        .env("PATH", path)
        .env("CURL_CALLED", &curl_called)
        .env("RUNNER_TEMP", &runner_temp)
        .env("RUNNER_TOOL_CACHE", &runner_temp)
        .env("GITHUB_WORKSPACE", &workspace)
        .env("INPUT_VERSION", "0.30.1")
        .env("INPUT_COMPONENTS", "core")
        .env("INPUT_SINCE", "")
        .env("INPUT_ALL", "true")
        .env("INPUT_EVIDENCE", "")
        .env("INPUT_WORKING_DIRECTORY", ".")
        .env("INPUT_REPOSITORY_TOKEN", "")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("::error file=.github/workflows/ci.yml,line=7::") && stderr.contains("has no output test"),
        "{stderr}"
    );
    assert!(stderr.contains("workflow audit found 1 error"), "{stderr}");
    assert!(!curl_called.exists(), "installation started before the audit failed");
    fs::remove_dir_all(directory).unwrap();
}
