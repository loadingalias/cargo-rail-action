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
fn source_plan_selectors_validate_identity_and_reject_checkout_drift() {
    let binary = PathBuf::from(std::env::var_os("CARGO_RAIL_TEST_BINARY").expect("source binary"));
    let directory = temporary_directory();
    let workspace = directory.join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::write(
        workspace.join("Cargo.toml"),
        "[package]\nname = 'source-contract'\nversion = '0.1.0'\nedition = '2024'\n[lib]\npath = 'lib.rs'\n",
    )
    .unwrap();
    fs::write(workspace.join("lib.rs"), "pub fn value() -> u8 { 7 }\n").unwrap();
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
    fs::write(workspace.join("lib.rs"), "pub fn value() -> u8 { 8 }\n").unwrap();
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
    let path =
        std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())))
            .unwrap();
    let run = || {
        Command::new("bash")
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/bootstrap.sh"))
            .args(["run", "planner"])
            .env("PATH", &path)
            .env("RUNNER_OS", "macOS")
            .env("RUNNER_ARCH", "ARM64")
            .env("RUNNER_TEMP", &runner_temp)
            .env("RUNNER_TOOL_CACHE", &cache)
            .env("FIXTURES", &fixtures)
            .env("DOWNLOADS", &downloads)
            .env("INVOCATIONS", &invocations)
            .output()
            .unwrap()
    };
    let asset = "cargo-rail-action-aarch64-apple-darwin";
    let release_url = format!("https://github.com/loadingalias/cargo-rail-action/releases/download/v{version}");
    let first = run();
    assert!(first.status.success(), "{first:?}");
    assert_eq!(
        fs::read_to_string(&downloads).unwrap(),
        format!("{release_url}/{manifest_name}\n{release_url}/{asset}\n")
    );
    fs::write(&downloads, "").unwrap();
    let reused = run();
    assert!(reused.status.success(), "{reused:?}");
    assert_eq!(
        fs::read_to_string(&downloads).unwrap(),
        format!("{release_url}/{manifest_name}\n")
    );
    let calls = fs::read_to_string(&invocations).unwrap();
    let expected_calls =
        format!("self-check --expect-version {version} --expect-target aarch64-apple-darwin\nrun planner\n");
    assert_eq!(calls, expected_calls.repeat(2));

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
        let rejected = run();
        assert!(!rejected.status.success(), "{rejected:?}");
        assert!(String::from_utf8_lossy(&rejected.stderr).contains("immutable runtime destination is corrupt"));
        assert_eq!(fs::read_to_string(&invocations).unwrap(), calls);
        fs::write(&license, license_bytes).unwrap();
    }
    let mut corrupt = runtime.to_vec();
    corrupt[0] = b'x';
    fs::write(installed, corrupt).unwrap();
    fs::write(&downloads, "").unwrap();
    let rejected = run();
    assert!(!rejected.status.success(), "{rejected:?}");
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("immutable runtime destination is corrupt"));
    assert_eq!(
        fs::read_to_string(&downloads).unwrap(),
        format!("{release_url}/{manifest_name}\n")
    );
    assert_eq!(fs::read_to_string(&invocations).unwrap(), calls);
    fs::remove_dir_all(directory).unwrap();
}
