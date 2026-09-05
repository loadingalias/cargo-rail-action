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
    assert!(examples.len() >= 3);
    for example in examples {
        let script = format!(
            "cargo-rail-action() {{ \"$RUNTIME\" \"$@\"; }}\ncargo() {{ printf invoked > \"$EXECUTED\"; }}\n{example}"
        );
        let output = Command::new("bash")
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
