use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn yaml(path: &str) -> Value {
    serde_saphyr::from_slice(&fs::read(root().join(path)).expect("read metadata")).expect("parse metadata")
}

fn mapping<'a>(value: &'a Value, subject: &str) -> &'a Map<String, Value> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("{subject} must be a mapping"))
}

fn field<'a>(object: &'a Map<String, Value>, name: &str) -> &'a Value {
    object.get(name).unwrap_or_else(|| panic!("missing {name}"))
}

fn keys(object: &Map<String, Value>) -> BTreeSet<String> {
    object.keys().cloned().collect()
}

#[test]
fn action_metadata_matches_the_v9_surface() {
    let planner = yaml("action.yaml");
    let cache = yaml("cache/action.yaml");
    let setup = yaml("setup/action.yaml");

    let planner = mapping(&planner, "planner");
    let cache = mapping(&cache, "cache");
    let setup = mapping(&setup, "setup");
    assert_eq!(
        keys(mapping(field(planner, "inputs"), "planner inputs")),
        BTreeSet::from([
            "all".to_string(),
            "components".to_string(),
            "evidence".to_string(),
            "repository-token".to_string(),
            "since".to_string(),
            "version".to_string(),
            "working-directory".to_string(),
        ])
    );
    assert_eq!(
        keys(mapping(field(planner, "outputs"), "planner outputs")),
        BTreeSet::from([
            "plan-file".to_string(),
            "required-work".to_string(),
            "version".to_string()
        ])
    );
    assert_eq!(
        keys(mapping(field(cache, "inputs"), "cache inputs")),
        BTreeSet::from([
            "local-dir".to_string(),
            "max-size".to_string(),
            "mode".to_string(),
            "remote".to_string(),
            "root-portability".to_string(),
            "verify-remote".to_string(),
            "version".to_string(),
            "working-directory".to_string(),
        ])
    );
    assert_eq!(
        keys(mapping(field(cache, "outputs"), "cache outputs")),
        BTreeSet::from(["status".to_string(), "version".to_string()])
    );
    assert_eq!(
        keys(mapping(field(setup, "inputs"), "setup inputs")),
        BTreeSet::from(["version".to_string()])
    );
    assert_eq!(
        keys(mapping(field(setup, "outputs"), "setup outputs")),
        BTreeSet::from(["version".to_string()])
    );

    for (name, action) in [("planner", planner), ("cache", cache), ("setup", setup)] {
        let runs = mapping(field(action, "runs"), "runs");
        assert_eq!(field(runs, "using").as_str(), Some("composite"), "{name}");
        let steps = field(runs, "steps").as_array().expect("steps sequence");
        assert_eq!(steps.len(), 1, "{name} must have one composite step");
        let run = field(mapping(&steps[0], "step"), "run").as_str().expect("run string");
        assert!(run.contains("scripts/bootstrap.sh"), "{name}");
        assert!(!run.contains("python") && !run.contains("cargo install") && !run.contains("cargo build"));
    }

    let defaults = [planner, cache, setup].map(|action| {
        let inputs = mapping(field(action, "inputs"), "inputs");
        let version = mapping(field(inputs, "version"), "version input");
        field(version, "default").as_str().expect("version default")
    });
    assert_eq!(defaults, ["0.26.0"; 3]);
}

#[test]
fn schemas_are_closed_and_match_public_examples() {
    let cache_schema: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("schemas/cache-status-v1.schema.json")).expect("cache schema"))
            .expect("parse cache schema");
    assert_eq!(cache_schema["additionalProperties"], false);
    let required = cache_schema["required"].as_array().expect("required fields");
    let example = serde_json::json!({
        "schema_version": 1,
        "cargo_rail": "0.26.0",
        "provider": "aws-s3",
        "mode": "read",
        "max_bytes": 10_737_418_240_u64,
        "root_portability": "physical",
        "remote_verification": "not_requested"
    });
    assert_eq!(
        required
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<BTreeSet<_>>(),
        example
            .as_object()
            .expect("example object")
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
    );

    let release_schema: serde_json::Value = serde_json::from_slice(
        &fs::read(root().join("schemas/release-intent-v1.schema.json")).expect("release schema"),
    )
    .expect("parse release schema");
    assert_eq!(release_schema["additionalProperties"], false);
    assert_eq!(release_schema["properties"]["assets"]["maxItems"], 4);
    assert_eq!(
        release_schema["$defs"]["macos_runtime"]["allOf"][1]["properties"]["bytes"]["maximum"],
        16 * 1024 * 1024
    );
    assert_eq!(
        release_schema["$defs"]["runtime_manifest"]["allOf"][1]["properties"]["bytes"]["maximum"],
        64 * 1024
    );
}

#[test]
fn repository_has_no_interpreter_implementation_residue() {
    let repository = root();
    let mut files = Vec::new();
    collect_files(&repository, &repository, &mut files);
    for relative in &files {
        assert_ne!(
            relative.extension().and_then(|value| value.to_str()),
            Some("py"),
            "{}",
            relative.display()
        );
        assert_ne!(
            relative.extension().and_then(|value| value.to_str()),
            Some("rb"),
            "{}",
            relative.display()
        );
    }
    let scripts = files
        .iter()
        .filter(|path| path.starts_with("scripts"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(scripts, vec![PathBuf::from("scripts/bootstrap.sh")]);
    let bootstrap = fs::read_to_string(repository.join("scripts/bootstrap.sh")).expect("bootstrap");
    assert!(bootstrap.contains(&format!("RUNTIME_VERSION=\"{}\"", env!("CARGO_PKG_VERSION"))));
    assert!(bootstrap.contains(&format!("RUNTIME_RELEASE=\"v{}\"", env!("CARGO_PKG_VERSION"))));
    assert!(
        !bootstrap.contains("python") && !bootstrap.contains("cargo install") && !bootstrap.contains("cargo build")
    );
}

#[test]
fn release_workflow_has_one_explicit_protected_release_path() {
    let workflow = fs::read_to_string(root().join(".github/workflows/release.yaml")).expect("release workflow");
    serde_saphyr::from_str::<Value>(&workflow).expect("parse release workflow");
    assert!(workflow.contains("operation:"));
    assert!(workflow.contains("options: [verify, publish, promote-v9]"));
    assert!(workflow.contains("environment: release"));
    assert!(workflow.contains("permissions: {}"));
    assert!(workflow.contains("release publish --check"));
    assert!(workflow.contains("release publish --apply"));
    assert!(workflow.contains("release promote --check"));
    assert!(workflow.contains("release promote --apply"));
    assert!(workflow.contains("release preflight"));
    assert!(workflow.contains("github.ref == 'refs/heads/main'"));
    assert_eq!(
        workflow.matches("cargo install just --version 1.58.0 --locked").count(),
        2
    );
    assert!(!workflow.contains("cargo install just --locked"));
    assert!(workflow.contains("name: runtime-executable-${{ matrix.target }}"));
    assert_eq!(workflow.matches("pattern: runtime-executable-*").count(), 2);
    assert_eq!(
        workflow.matches("name: runtime-manifest-${{ inputs.version }}").count(),
        2
    );
    assert!(!workflow.contains("pattern: runtime-*"));
    assert!(workflow.contains("Verify requested runtime identity before smoke tests"));
    assert!(!workflow.contains("path: ${{ runner.temp }}/release-intent"));
    assert!(!workflow.contains("printf 'cargo-rail-action-runtime-v1"));
    assert!(!workflow.contains("for target in aarch64-apple-darwin"));
    assert!(!workflow.contains("pull_request_target"));
    assert!(!workflow.contains("--pr"));
}

fn collect_files(root: &Path, current: &Path, output: &mut Vec<PathBuf>) {
    let mut entries = fs::read_dir(current)
        .expect("read repository")
        .map(|entry| entry.expect("directory entry"))
        .collect::<Vec<_>>();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let relative = path.strip_prefix(root).expect("relative path");
        if entry.file_type().expect("file type").is_dir() {
            if matches!(relative.to_str(), Some(".git" | "target")) {
                continue;
            }
            collect_files(root, &path, output);
        } else {
            output.push(relative.to_path_buf());
        }
    }
}

#[test]
fn cache_collection_and_report_have_distinct_bounded_surfaces() {
    let collect = yaml("cache/collect/action.yaml");
    let report = yaml("cache/report/action.yaml");
    assert_eq!(collect["runs"]["steps"].as_array().unwrap().len(), 1);
    assert_eq!(report["runs"]["steps"].as_array().unwrap().len(), 1);
    assert_eq!(
        keys(collect["outputs"].as_object().unwrap()),
        BTreeSet::from(["record-file".into()])
    );
    assert!(report.get("outputs").is_none());
    assert!(
        collect["runs"]["steps"][0]["run"]
            .as_str()
            .unwrap()
            .ends_with("run cache-collect")
    );
    assert!(
        report["runs"]["steps"][0]["run"]
            .as_str()
            .unwrap()
            .ends_with("run cache-report")
    );
    let schema: Value =
        serde_json::from_slice(&fs::read(root().join("schemas/cache-job-v1.schema.json")).unwrap()).unwrap();
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["$defs"]["measurements"]["properties"]["bypass_reasons"]["maxProperties"],
        128
    );
}
