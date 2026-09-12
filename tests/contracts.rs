use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

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

    for (name, action, bootstrap) in [
        ("planner", planner, "$GITHUB_ACTION_PATH/scripts/bootstrap.sh"),
        ("cache", cache, "$GITHUB_ACTION_PATH/../scripts/bootstrap.sh"),
        ("setup", setup, "$GITHUB_ACTION_PATH/../scripts/bootstrap.sh"),
    ] {
        let runs = mapping(field(action, "runs"), "runs");
        assert_eq!(field(runs, "using").as_str(), Some("composite"), "{name}");
        let steps = field(runs, "steps").as_array().expect("steps sequence");
        assert_eq!(steps.len(), 1, "{name} must have one composite step");
        let run = field(mapping(&steps[0], "step"), "run").as_str().expect("run string");
        assert_eq!(run, format!("bash \"{bootstrap}\" run {name}"), "{name}");
    }

    let defaults = [planner, cache, setup].map(|action| {
        let inputs = mapping(field(action, "inputs"), "inputs");
        let version = mapping(field(inputs, "version"), "version input");
        field(version, "default").as_str().expect("version default")
    });
    assert_eq!(defaults, ["0.26.0"; 3]);
}

#[test]
fn public_schemas_reject_unknown_fields_and_bound_release_assets() {
    let cache: Value =
        serde_json::from_slice(&fs::read(root().join("schemas/cache-status-v1.schema.json")).expect("cache schema"))
            .expect("parse cache schema");
    assert_eq!(cache["additionalProperties"], false);

    let release: Value = serde_json::from_slice(
        &fs::read(root().join("schemas/release-intent-v1.schema.json")).expect("release schema"),
    )
    .expect("parse release schema");
    assert_eq!(release["additionalProperties"], false);
    assert_eq!(release["$defs"]["asset"]["additionalProperties"], false);
    assert_eq!(release["properties"]["assets"]["minItems"], 4);
    assert_eq!(release["properties"]["assets"]["maxItems"], 4);
    assert_eq!(release["properties"]["assets"]["items"], false);
    for (asset, maximum) in [
        ("macos_runtime", 16 * 1024 * 1024),
        ("windows_runtime", 16 * 1024 * 1024),
        ("linux_runtime", 16 * 1024 * 1024),
        ("runtime_manifest", 64 * 1024),
    ] {
        assert_eq!(
            release["$defs"][asset]["allOf"][1]["properties"]["bytes"]["maximum"], maximum,
            "{asset}"
        );
    }
}

#[test]
fn ci_produces_every_runtime_required_by_release() {
    let ci = yaml(".github/workflows/ci.yml");
    let rows = ci["jobs"]["check"]["strategy"]["matrix"]["include"]
        .as_array()
        .expect("native CI matrix");
    let targets: BTreeSet<_> = rows.iter().map(|row| row["target"].as_str().unwrap()).collect();
    assert_eq!(
        targets,
        BTreeSet::from([
            "aarch64-apple-darwin",
            "x86_64-pc-windows-msvc",
            "x86_64-unknown-linux-gnu",
        ])
    );
    let steps = ci["jobs"]["check"]["steps"].as_array().unwrap();
    let upload = steps.last().unwrap();
    assert_eq!(upload["with"]["name"], "runtime-${{ matrix.target }}");
    assert_eq!(upload["with"]["if-no-files-found"], "error");
    let release = yaml(".github/workflows/release.yml");
    let commands = release["jobs"]["prepare"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|step| step["run"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for target in targets {
        assert!(commands.contains(&format!("cargo-rail-action-{target}")));
    }
}

#[test]
fn release_workflow_requires_ci_and_protects_publication() {
    let workflow = yaml(".github/workflows/release.yml");
    assert_eq!(
        keys(mapping(&workflow["on"], "release triggers")),
        BTreeSet::from(["workflow_dispatch".to_string()])
    );
    assert!(mapping(&workflow["permissions"], "default permissions").is_empty());
    assert_eq!(workflow["concurrency"]["cancel-in-progress"], false);
    let prepare = &workflow["jobs"]["prepare"];
    assert_eq!(prepare["if"], "github.ref == 'refs/heads/main'");
    assert_eq!(prepare["permissions"]["contents"], "read");
    let steps = prepare["steps"].as_array().unwrap();
    let gate = steps.iter().position(|step| step["id"] == "ci").unwrap();
    let download = steps
        .iter()
        .position(|step| step["with"].get("run-id").is_some())
        .unwrap();
    assert!(gate < download);
    assert!(
        steps[gate]["run"]
            .as_str()
            .unwrap()
            .contains("actions/workflows/ci.yml/runs")
    );
    assert_eq!(steps[download]["with"]["run-id"], "${{ steps.ci.outputs.run-id }}");
    let publish = &workflow["jobs"]["publish"];
    assert_eq!(publish["needs"], "prepare");
    assert_eq!(publish["environment"], "release");
    assert_eq!(publish["permissions"]["contents"], "write");
    let steps = publish["steps"].as_array().unwrap();
    let commands = steps.last().unwrap()["run"].as_str().unwrap();
    assert!(commands.find("release publish --check").unwrap() < commands.find("release publish --apply").unwrap());
    assert!(commands.find("release publish --apply").unwrap() < commands.find("release promote --check").unwrap());
    assert!(commands.find("release promote --check").unwrap() < commands.find("release promote --apply").unwrap());
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
