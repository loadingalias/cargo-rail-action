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
    assert_eq!(examples.len(), 2, "both documented Cargo selector examples");
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
#[ignore = "requires the current Cargo-Rail source binary"]
fn source_release_record_is_independently_validated() {
    let binary = PathBuf::from(std::env::var_os("CARGO_RAIL_TEST_BINARY").expect("source binary"));
    let directory = temporary_directory();
    let workspace = directory.join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(workspace.join(".config")).unwrap();
    fs::create_dir(workspace.join(".changes")).unwrap();
    fs::write(
        workspace.join("Cargo.toml"),
        "[package]\nname = 'release-contract'\nversion = '0.1.0'\nedition = '2024'\n[lib]\npath = 'lib.rs'\n",
    )
    .unwrap();
    fs::write(workspace.join("lib.rs"), "pub fn value() -> u8 { 7 }\n").unwrap();
    fs::write(workspace.join(".gitignore"), "target/\n").unwrap();
    fs::write(
        workspace.join(".config/rail.toml"),
        "[release]\nremote_effects = 'push'\nsemver_check = 'off'\nsign_tags = false\n",
    )
    .unwrap();
    fs::write(
        workspace.join(".changes/release.md"),
        "---\nrelease-contract = 'patch'\n---\n\nPreserve release authority between runners.\n",
    )
    .unwrap();
    let lock = Command::new("cargo")
        .current_dir(&workspace)
        .args(["generate-lockfile", "--offline"])
        .output()
        .unwrap();
    assert!(lock.status.success(), "{lock:?}");
    for arguments in [
        vec!["init", "--initial-branch=main"],
        vec!["config", "user.name", "Release Contract"],
        vec!["config", "user.email", "contract@example.invalid"],
        vec!["config", "commit.gpgsign", "false"],
        vec![
            "remote",
            "add",
            "origin",
            "https://github.com/example/release-contract.git",
        ],
        vec!["add", "."],
        vec!["commit", "-m", "Review release intent"],
    ] {
        let output = Command::new("git")
            .current_dir(&workspace)
            .args(arguments)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    let prepared = Command::new(&binary)
        .current_dir(&workspace)
        .env_remove("CARGO_TARGET_DIR")
        .args([
            "rail",
            "release",
            "run",
            "--all",
            "--bump",
            "patch",
            "--skip-tag",
            "--prepare",
            "--yes",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(prepared.status.success(), "{prepared:?}");
    let summary: serde_json::Value = serde_json::from_slice(&prepared.stdout).unwrap();
    let record_path = workspace
        .join("target/cargo-rail/releases")
        .join(format!("{}.json", summary["transaction_id"].as_str().unwrap()));
    let original = fs::read(&record_path).unwrap();
    let record: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let intent = record["intent"]["identity"].as_str().unwrap();
    let source = record["preparation"]["commit"].as_str().unwrap();
    let validate = |expected_intent: &str, expected_source: &str, repository: &str| {
        Command::new(env!("CARGO_BIN_EXE_cargo-rail-action"))
            .args(["release", "validate-record"])
            .arg(&record_path)
            .args([
                "--intent",
                expected_intent,
                "--source",
                expected_source,
                "--repository",
                repository,
            ])
            .output()
            .unwrap()
    };
    let accepted = validate(intent, source, "github.com/example/release-contract");
    assert!(accepted.status.success(), "{accepted:?}");
    let selected: serde_json::Value = serde_json::from_slice(&accepted.stdout).unwrap();
    assert_eq!(selected["transaction_id"], record["transaction_id"]);
    assert_eq!(selected["source"], source);
    assert_eq!(selected["state"], "active");
    assert_eq!(selected["phase"], "prepared");
    for rejected in [
        validate("sha256:wrong", source, "github.com/example/release-contract"),
        validate(intent, &"a".repeat(40), "github.com/example/release-contract"),
        validate(intent, source, "github.com/example/another"),
    ] {
        assert_eq!(rejected.status.code(), Some(2), "{rejected:?}");
        assert!(rejected.stdout.is_empty());
    }
    for (pointer, replacement) in [
        ("/schema_version", serde_json::json!(9)),
        ("/intent/plan/summary/total_crates", serde_json::json!(2)),
        (
            "/intent/plan/crates/0/manifest_path",
            serde_json::json!("../Cargo.toml"),
        ),
        ("/intent/release_config/sign_tags", serde_json::Value::Null),
        ("/intent/plan/crates/0/new_version", serde_json::json!("invalid")),
        ("/crates/0/publication/status", serde_json::json!("in_progress")),
        ("/status", serde_json::json!("complete")),
    ] {
        let mut changed = record.clone();
        *changed.pointer_mut(pointer).unwrap() = replacement;
        let mut unsigned = changed["intent"].clone();
        unsigned.as_object_mut().unwrap().remove("identity");
        let mut framed = b"cargo-rail-release-intent-v1\0".to_vec();
        framed.extend(
            serde_json::to_vec(&serde_json::json!({
                "intent": unsigned, "transaction_id": changed["transaction_id"]
            }))
            .unwrap(),
        );
        let identity = format!(
            "sha256:{}",
            rscrypto::Sha256::digest(&framed)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        changed["intent"]["identity"] = serde_json::json!(identity);
        fs::write(&record_path, serde_json::to_vec(&changed).unwrap()).unwrap();
        let rejected = validate(&identity, source, "github.com/example/release-contract");
        assert_eq!(rejected.status.code(), Some(2), "{pointer}: {rejected:?}");
        assert!(rejected.stdout.is_empty());
    }
    for changed in [
        String::from_utf8(original.clone()).unwrap().replacen(
            "\"schema_version\": 10",
            "\"schema_version\": 10, \"schema_version\": 10",
            1,
        ),
        String::from_utf8(original.clone())
            .unwrap()
            .replacen("\"schema_version\": 10", "\"schema_version\": 10.0", 1),
    ] {
        assert_ne!(changed.as_bytes(), original);
        fs::write(&record_path, changed).unwrap();
        let rejected = validate(intent, source, "github.com/example/release-contract");
        assert_eq!(rejected.status.code(), Some(2), "{rejected:?}");
        assert!(rejected.stdout.is_empty());
    }
    // The independent reader checks producer/asset bindings, including a valid native record.
    let mut native = record.clone();
    native["intent"]["skip_tag"] = serde_json::json!(false);
    native["tag_push"] = serde_json::json!({"status":"pending"});
    for package in native["crates"].as_array_mut().unwrap() {
        for field in ["tag", "forge_draft", "forge_publication"] {
            package[field] = serde_json::json!({"status":"pending"});
        }
    }
    native["intent"]["release_config"]["remote_effects"] = serde_json::json!("github");
    native["intent"]["release_config"]["validation"] = serde_json::json!({".github/workflows/ci.yml":["package"]});
    native["intent"]["release_config"]["artifacts"] = serde_json::json!({"release-contract":{
        "workflow":".github/workflows/ci.yml", "files":{"{crate}-{version}.zip":{"target":"x86_64-unknown-linux-gnu","source":null}}}});
    native["intent"]["plan"]["artifacts"] = serde_json::json!([{"package":"release-contract","workflow":".github/workflows/ci.yml",
        "files":[{"name":"release-contract-0.1.1.zip","target":"x86_64-unknown-linux-gnu","source":null}]}]);
    native["phase"] = serde_json::json!("awaiting_checks");
    native["commit_push"] = serde_json::json!({"status":"complete","object":source});
    native["readiness"] = serde_json::json!({"status":"complete","object":"exact workflow attempt verified"});
    native["validation"] = serde_json::json!([{"workflow":".github/workflows/ci.yml","workflow_id":7,"run_id":42,"attempt":3,"jobs":[{"name":"package","id":17}]}]);
    native["artifacts"] = serde_json::json!([{"package":"release-contract","run_id":42,"attempt":3,"artifact_id":91,"bytes":200,
        "sha256":"a".repeat(64),"expires_at":4070908800_u64,"files":[{"name":"release-contract-0.1.1.zip","bytes":123,"sha256":"b".repeat(64)}]}]);
    let check_native = |mut candidate: serde_json::Value| {
        let mut unsigned = candidate["intent"].clone();
        unsigned.as_object_mut().unwrap().remove("identity");
        let mut framed = b"cargo-rail-release-intent-v1\0".to_vec();
        framed.extend(
            serde_json::to_vec(&serde_json::json!({"transaction_id":candidate["transaction_id"],"intent":unsigned}))
                .unwrap(),
        );
        let identity = format!(
            "sha256:{}",
            rscrypto::Sha256::digest(&framed)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        candidate["intent"]["identity"] = serde_json::json!(identity);
        fs::write(&record_path, serde_json::to_vec(&candidate).unwrap()).unwrap();
        validate(&identity, source, "github.com/example/release-contract")
    };
    let accepted = check_native(native.clone());
    assert!(accepted.status.success(), "{accepted:?}");
    for (pointer, replacement) in [
        ("/artifacts/0/attempt", serde_json::json!(4)),
        ("/artifacts/0/files/0/name", serde_json::json!("another.zip")),
        ("/artifacts/0/sha256", serde_json::json!("invalid")),
        ("/readiness/status", serde_json::json!("pending")),
        ("/intent/skip_tag", serde_json::json!(true)),
        (
            "/intent/plan/artifacts/0/files/0/target",
            serde_json::json!("aarch64-unknown-linux-gnu"),
        ),
        (
            "/intent/plan/artifacts/0/files/0/source",
            serde_json::json!("../LICENSE"),
        ),
        (
            "/intent/release_config/artifacts/release-contract/workflow",
            serde_json::json!(".github/workflows/other.yml"),
        ),
    ] {
        let mut candidate = native.clone();
        *candidate.pointer_mut(pointer).unwrap() = replacement;
        let rejected = check_native(candidate);
        assert_eq!(rejected.status.code(), Some(2), "{pointer}: {rejected:?}");
        assert!(rejected.stdout.is_empty());
    }
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
fn source_release_adapter_executes_and_independently_rejects_changed_invocation_authority() {
    source_release_adapter(false);
}

#[cfg(unix)]
#[test]
#[ignore = "requires the current Cargo-Rail source binary"]
fn source_release_adapter_recovers_an_unrecorded_merge_and_rejects_a_changed_review_tree() {
    source_release_adapter(true);
}

#[cfg(unix)]
fn source_release_adapter(review: bool) {
    use std::os::unix::fs::PermissionsExt;
    let core = PathBuf::from(std::env::var_os("CARGO_RAIL_TEST_BINARY").expect("source core"));
    let directory = temporary_directory();
    let workspace = directory.join("workspace");
    fs::create_dir_all(workspace.join(".config")).unwrap();
    fs::create_dir(workspace.join(".changes")).unwrap();
    fs::write(
        workspace.join("Cargo.toml"),
        "[package]\nname='adapter-fixture'\nversion='0.1.0'\nedition='2024'\n[lib]\npath='lib.rs'\n",
    )
    .unwrap();
    fs::write(workspace.join("lib.rs"), "pub fn value() -> u8 { 1 }\n").unwrap();
    fs::write(workspace.join(".gitignore"), "target/\n").unwrap();
    fs::write(
        workspace.join(".changes/release.md"),
        "---\nadapter-fixture='patch'\n---\n\nKeep one release authority.\n",
    )
    .unwrap();
    fs::write(workspace.join(".config/rail.toml"),"[release]\nremote_effects='push'\nsemver_check='off'\nhosted_workflow='.github/workflows/release.yml'\nvalidation={'.github/workflows/ci.yml'=['tests']}\n").unwrap();
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
            .args(args)
            .output()
            .unwrap();
        assert!(result.status.success(), "{args:?}: {result:?}");
        String::from_utf8(result.stdout).unwrap().trim().to_owned()
    };
    let remote = directory.join("origin.git");
    git(
        &directory,
        &["init", "--bare", "--initial-branch=main", remote.to_str().unwrap()],
    );
    let ssh = directory.join("ssh");
    fs::write(&ssh,format!("#!/bin/sh\ncase \"$*\" in *git-receive-pack*) exec git-receive-pack '{}' ;; *git-upload-pack*) exec git-upload-pack '{}' ;; esac\nexit 1\n",remote.display(),remote.display())).unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
    for args in [
        vec!["init", "--initial-branch=main"],
        vec!["config", "user.name", "Release adapter"],
        vec!["config", "user.email", "adapter@example.invalid"],
        vec!["config", "core.sshCommand", ssh.to_str().unwrap()],
        vec!["remote", "add", "origin", "git@github.com:org/repo.git"],
        vec!["add", "."],
        vec!["commit", "-m", "Review release"],
        vec!["push", "-u", "origin", "main"],
    ] {
        git(&workspace, &args);
    }
    let initial = git(&workspace, &["rev-parse", "HEAD"]);
    let gh = directory.join("gh");
    fs::write(&gh,r#"#!/usr/bin/env python3
import json,pathlib,subprocess,sys
if sys.argv[1]!='api':sys.exit(0)
a=sys.argv[1:];endpoint=next(x for x in a if x.startswith('repos/'))
sha=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
run={'id':42,'run_attempt':3,'workflow_id':7,'head_sha':sha,'path':'.github/workflows/ci.yml','repository':{'full_name':'org/repo'},'head_repository':{'full_name':'org/repo'},'event':'workflow_dispatch','status':'completed','conclusion':'success'}
p=pathlib.Path(__file__).parent/'pull.json'
if '/pulls' in endpoint:
    if '--method' in a:
        body=json.loads(pathlib.Path(a[a.index('--input')+1]).read_text())
        p.write_text(json.dumps({'number':7,'head':{'sha':sha,'ref':body['head'],'repo':{'full_name':'org/repo'}},'base':{'ref':'main','repo':{'full_name':'org/repo'}},'state':'open','merged':False}))
    print(json.dumps(([json.loads(p.read_text())] if p.exists() else []) if '/pulls?' in endpoint else json.loads(p.read_text())))
    sys.exit(0)
if endpoint.endswith('/ci.yml'):result={'id':7,'path':'.github/workflows/ci.yml','state':'active'}
elif endpoint.endswith('/dispatches'):result={'workflow_run_id':42}
elif '/runs?' in endpoint:result={'total_count':0,'workflow_runs':[]}
elif '/jobs?' in endpoint:result={'total_count':1,'jobs':[{'id':17,'name':'tests','run_id':42,'run_attempt':3,'head_sha':sha,'status':'completed','conclusion':'success'}]}
else:result=run
print(json.dumps(result))
"#).unwrap();
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
    let event = directory.join("event.json");
    fs::write(&event, r#"{"repository":{"full_name":"org/repo"},"inputs":{}}"#).unwrap();
    let output_file = directory.join("output");
    let path_file = directory.join("path");
    fs::write(&output_file, "").unwrap();
    fs::write(&path_file, "").unwrap();
    let invoke = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-rail-action"));
        if review {
            command.arg("release").arg("execute").arg("--review");
        } else {
            command.arg("release").arg("execute");
        }
        command
            .current_dir(&workspace)
            .env_remove("CARGO_TARGET_DIR")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env(
                "PATH",
                format!("{}:{}", directory.display(), std::env::var("PATH").unwrap()),
            )
            .env("GITHUB_ACTIONS", "true")
            .env("GITHUB_REPOSITORY", "org/repo")
            .env("GITHUB_WORKSPACE", &workspace)
            .env("GITHUB_EVENT_NAME", "workflow_dispatch")
            .env("GITHUB_EVENT_PATH", &event)
            .env("GITHUB_RUN_ID", "88")
            .env(
                "GITHUB_WORKFLOW_REF",
                "org/repo/.github/workflows/release.yml@refs/heads/main",
            )
            .env("GITHUB_SHA", &initial)
            .env("GITHUB_OUTPUT", &output_file)
            .env("GITHUB_PATH", &path_file)
            .arg("--cargo-rail")
            .arg(&core)
            .arg("--version")
            .arg(std::env::var("CARGO_RAIL_TEST_RELEASE_VERSION").expect("source core version"))
            .output()
            .unwrap()
    };
    let completed = invoke();
    assert!(completed.status.success(), "{completed:?}");
    let record_path = fs::read_dir(workspace.join("target/cargo-rail/releases"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|extension| extension == "json"))
        .unwrap();
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    if review {
        assert_eq!(record["phase"], "awaiting_review");
        assert!(record["review"]["merge"].is_null());
        let branch = format!("rail/{}", record["transaction_id"].as_str().unwrap());
        git(&workspace, &["switch", "main"]);
        git(
            &workspace,
            &["merge", "--no-ff", &branch, "-m", "Merge reviewed release"],
        );
        let merged = git(&workspace, &["rev-parse", "HEAD"]);
        git(&workspace, &["push", "origin", "main"]);
        let pull_path = directory.join("pull.json");
        let mut pull: serde_json::Value = serde_json::from_slice(&fs::read(&pull_path).unwrap()).unwrap();
        pull["merged"] = true.into();
        pull["state"] = "closed".into();
        pull["merged_at"] = "2026-09-13T12:00:00Z".into();
        fs::write(workspace.join("unexpected.txt"), "changed reviewed tree").unwrap();
        git(&workspace, &["add", "unexpected.txt"]);
        git(&workspace, &["commit", "-m", "Change reviewed tree"]);
        let changed = git(&workspace, &["rev-parse", "HEAD"]);
        git(&remote, &["fetch", workspace.to_str().unwrap(), &changed]);
        pull["merge_commit_sha"] = changed.into();
        fs::write(&pull_path, serde_json::to_vec(&pull).unwrap()).unwrap();
        let payload = serde_json::json!({"repository":{"full_name":"org/repo"},"inputs": {
            "transaction": record["transaction_id"], "intent": record["intent"]["identity"], "source": initial
        }});
        fs::write(&event, serde_json::to_vec(&payload).unwrap()).unwrap();
        let before = fs::read(&output_file).unwrap();
        let rejected = invoke();
        assert_eq!(rejected.status.code(), Some(2), "{rejected:?}");
        assert!(
            String::from_utf8_lossy(&rejected.stderr).contains("reviewed merge changed the prepared release tree"),
            "{rejected:?}"
        );
        assert_eq!(fs::read(&output_file).unwrap(), before);
        pull["merge_commit_sha"] = merged.clone().into();
        fs::write(&pull_path, serde_json::to_vec(&pull).unwrap()).unwrap();
        git(&workspace, &["switch", &branch]);
        let completed = invoke();
        assert!(completed.status.success(), "{completed:?}");
        record = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
        assert_eq!(record["review"]["merge"]["commit"], merged);
    }
    assert_eq!(record["status"], "complete");
    let prepared = record["review"]["merge"]["commit"]
        .as_str()
        .or_else(|| record["preparation"]["commit"].as_str())
        .unwrap();
    assert_eq!(
        git(&remote, &["rev-parse", "refs/tags/adapter-fixture-v0.1.1^{commit}"],),
        prepared
    );
    assert!(
        fs::read_to_string(&output_file)
            .unwrap()
            .contains(&format!("release-sha={prepared}\n"))
    );
    let immutable_outputs = fs::read(&output_file).unwrap();
    let payload = serde_json::json!({"repository":{"full_name":"org/repo"},"inputs":{"transaction":record["transaction_id"],"intent":"sha256:wrong","source":prepared}});
    fs::write(&event, serde_json::to_vec(&payload).unwrap()).unwrap();
    let rejected = invoke();
    assert_eq!(rejected.status.code(), Some(2), "{rejected:?}");
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("does not authorize the original release intent"));
    assert_eq!(fs::read(&output_file).unwrap(), immutable_outputs);
    assert_eq!(
        git(&remote, &["rev-parse", "refs/tags/adapter-fixture-v0.1.1^{commit}"],),
        prepared
    );
    fs::remove_dir_all(directory).unwrap();
}
