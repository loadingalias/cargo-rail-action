use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

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
fn action_metadata_matches_the_v10_surface() {
    let planner = yaml("action.yaml");
    let cache = yaml("cache/action.yaml");
    let collect = yaml("cache/collect/action.yaml");
    let report = yaml("cache/report/action.yaml");
    let setup = yaml("setup/action.yaml");
    let release = yaml("release/action.yaml");

    let planner = mapping(&planner, "planner");
    let cache = mapping(&cache, "cache");
    let collect = mapping(&collect, "collect");
    let report = mapping(&report, "report");
    let setup = mapping(&setup, "setup");
    let release = mapping(&release, "release");
    assert_eq!(
        keys(mapping(field(planner, "inputs"), "planner inputs")),
        BTreeSet::from([
            "all".to_string(),
            "components".to_string(),
            "evidence".to_string(),
            "repository-token".to_string(),
            "runtime-source".to_string(),
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
            "runtime-source".to_string(),
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
        BTreeSet::from(["runtime-source".to_string(), "version".to_string()])
    );
    assert_eq!(
        keys(mapping(field(setup, "outputs"), "setup outputs")),
        BTreeSet::from(["version".to_string()])
    );

    for (name, action, bootstrap) in [
        ("planner", planner, "$GITHUB_ACTION_PATH/scripts/bootstrap.sh"),
        ("cache", cache, "$GITHUB_ACTION_PATH/../scripts/bootstrap.sh"),
        ("setup", setup, "$GITHUB_ACTION_PATH/../scripts/bootstrap.sh"),
        (
            "cache-collect",
            collect,
            "$GITHUB_ACTION_PATH/../../scripts/bootstrap.sh",
        ),
        ("cache-report", report, "$GITHUB_ACTION_PATH/../../scripts/bootstrap.sh"),
        ("release", release, "$GITHUB_ACTION_PATH/../scripts/bootstrap.sh"),
    ] {
        let runs = mapping(field(action, "runs"), "runs");
        assert_eq!(field(runs, "using").as_str(), Some("composite"), "{name}");
        let steps = field(runs, "steps").as_array().expect("steps sequence");
        assert_eq!(steps.len(), 1, "{name} must have one composite step");
        let run = field(mapping(&steps[0], "step"), "run").as_str().expect("run string");
        assert_eq!(run, format!("bash \"{bootstrap}\" run {name}"), "{name}");
        let env = mapping(field(mapping(&steps[0], "step"), "env"), "step environment");
        assert_eq!(
            field(env, "CARGO_RAIL_ACTION_RUNTIME_SOURCE").as_str(),
            Some("${{ inputs.runtime-source }}"),
            "{name}"
        );
    }

    for (name, action) in [("collect", collect), ("report", report), ("release", release)] {
        assert!(
            mapping(field(action, "inputs"), "inputs").contains_key("runtime-source"),
            "{name}"
        );
    }

    let defaults = [planner, cache, setup, release].map(|action| {
        let inputs = mapping(field(action, "inputs"), "inputs");
        let version = mapping(field(inputs, "version"), "version input");
        field(version, "default").as_str().expect("version default")
    });
    let lock = fs::read_to_string(root().join(".github/cargo-rail.lock")).expect("read Cargo-Rail lock");
    let locked_version = lock
        .lines()
        .find_map(|line| line.strip_prefix("version="))
        .expect("Cargo-Rail lock version");
    assert_eq!(defaults, [locked_version; 4]);
}

#[test]
fn public_schemas_reject_unknown_fields_and_bound_release_assets() {
    let cache: Value =
        serde_json::from_slice(&fs::read(root().join("schemas/cache-status-v1.schema.json")).expect("cache schema"))
            .expect("parse cache schema");
    assert_eq!(cache["additionalProperties"], false);
    let cache_validator = jsonschema::validator_for(&cache).expect("compile cache schema");
    let status = |cargo_rail| {
        serde_json::json!({
            "schema_version": 1,
            "cargo_rail": cargo_rail,
            "provider": "cloudflare-r2",
            "mode": "read-write",
            "max_bytes": 10_737_418_240_u64,
            "root_portability": "physical",
            "remote_verification": "verified"
        })
    };
    for accepted in ["0.26.0", "0.27.19", "1.0.0", "12.34.56"] {
        assert!(
            cache_validator.is_valid(&status(accepted)),
            "cache schema rejected {accepted}"
        );
    }
    for rejected in ["0.27.0-rc.1", "00.27.1", "latest"] {
        assert!(
            !cache_validator.is_valid(&status(rejected)),
            "cache schema accepted {rejected}"
        );
    }

    let release: Value =
        serde_json::from_slice(&fs::read(root().join("schemas/release-record-v9.schema.json")).unwrap()).unwrap();
    assert_eq!(release["additionalProperties"], false);
    assert_eq!(release["$defs"]["artifact_evidence"]["additionalProperties"], false);
}

#[test]
fn cargo_rail_lock_separates_release_and_tooling_authority() {
    let bash = std::env::var_os("CARGO_RAIL_TEST_BASH").unwrap_or_else(|| "bash".into());
    let output = Command::new(bash)
        .arg(root().join("scripts/read-cargo-rail-lock.sh"))
        .output()
        .expect("read Cargo-Rail lock");
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(output.stdout, fs::read(root().join(".github/cargo-rail.lock")).unwrap());

    let values = String::from_utf8(output.stdout).unwrap();
    let fields = values.lines().collect::<Vec<_>>();
    assert_eq!(fields.len(), 3);
    assert!(fields[0].strip_prefix("version=").is_some_and(|version| {
        semver::Version::parse(version).is_ok_and(|version| version.pre.is_empty() && version.build.is_empty())
    }));
    assert!(fields[1].strip_prefix("commit=").is_some_and(|commit| {
        commit.len() == 40
            && commit
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    }));
    assert!(fields[2].strip_prefix("tooling=").is_some_and(|commit| {
        commit.len() == 40
            && commit
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    }));
}

#[test]
fn ci_reuses_the_package_contract_with_bounded_permissions() {
    let ci = yaml(".github/workflows/ci.yml");
    assert_eq!(
        keys(mapping(&ci["on"], "CI triggers")),
        BTreeSet::from(["pull_request".into(), "push".into()])
    );
    assert_eq!(ci["permissions"]["contents"], "read");
    assert_eq!(ci["jobs"]["validate"]["uses"], "./.github/workflows/package.yml");
    assert_eq!(
        keys(mapping(
            &ci["jobs"]["validate"]["permissions"],
            "reusable package permissions"
        )),
        BTreeSet::from([
            "artifact-metadata".into(),
            "attestations".into(),
            "contents".into(),
            "id-token".into(),
        ])
    );
    assert_eq!(ci["jobs"]["validate"]["permissions"]["contents"], "read");
    assert_eq!(ci["jobs"]["validate"]["permissions"]["id-token"], "write");
    assert_eq!(ci["jobs"]["validate"]["permissions"]["attestations"], "write");
    assert_eq!(ci["jobs"]["validate"]["permissions"]["artifact-metadata"], "write");
}

#[test]
fn package_workflow_produces_every_authenticated_runtime() {
    let package = yaml(".github/workflows/package.yml");
    assert_eq!(
        keys(mapping(&package["on"], "package triggers")),
        BTreeSet::from(["workflow_call".into(), "workflow_dispatch".into()])
    );
    let check = &package["jobs"]["check"];
    let rows = check["strategy"]["matrix"]["include"]
        .as_array()
        .expect("native package matrix");
    let targets: BTreeSet<_> = rows.iter().map(|row| row["target"].as_str().unwrap()).collect();
    assert_eq!(
        targets,
        BTreeSet::from([
            "aarch64-apple-darwin",
            "aarch64-unknown-linux-gnu",
            "x86_64-pc-windows-msvc",
            "x86_64-unknown-linux-gnu",
        ])
    );
    let runners: BTreeSet<_> = rows.iter().map(|row| row["runner"].as_str().unwrap()).collect();
    assert_eq!(
        runners,
        BTreeSet::from(["macos-15", "ubuntu-24.04", "ubuntu-24.04-arm", "windows-2025"])
    );
    // The check job inherits this read-only grant, so its installer token cannot write.
    assert_eq!(package["permissions"], serde_json::json!({"contents": "read"}));
    assert_eq!(check["permissions"], Value::Null);
    let steps = check["steps"].as_array().unwrap();
    let lock = steps
        .iter()
        .find(|step| step["name"] == "Read the Cargo-Rail lock")
        .expect("lock reader");
    assert_eq!(
        lock["run"],
        "bash scripts/read-cargo-rail-lock.sh >> \"$GITHUB_OUTPUT\""
    );
    let tooling = steps
        .iter()
        .find(|step| step["name"] == "Checkout locked Cargo-Rail tooling")
        .expect("locked tooling checkout");
    assert_eq!(tooling["with"]["ref"], "${{ steps.cargo-rail.outputs.tooling }}");
    assert_eq!(tooling["with"]["path"], ".ci-tooling");
    let source = steps
        .iter()
        .find(|step| step["name"] == "Checkout locked Cargo-Rail release source")
        .expect("locked release source checkout");
    assert_eq!(source["if"], "github.event_name != 'workflow_dispatch'");
    assert_eq!(source["with"]["ref"], "${{ steps.cargo-rail.outputs.commit }}");
    assert_eq!(source["with"]["path"], ".cargo-rail-source");
    for (name, command) in [
        (
            "Install Linux tooling",
            "../.ci-tooling/scripts/tooling/\"$PLATFORM\".sh ci",
        ),
        (
            "Install Windows tooling",
            "../.ci-tooling/scripts/tooling/x86_64-win.ps1 -Operation ci",
        ),
        (
            "Install macOS tooling",
            "../.ci-tooling/scripts/tooling/package-unix.sh aarch64-apple-darwin ci",
        ),
    ] {
        let step = steps
            .iter()
            .find(|step| step["name"] == name)
            .expect("native tooling step");
        assert_eq!(step["run"], command, "tooling requires an explicit operation");
        if name == "Install Linux tooling" {
            assert_eq!(step["env"]["PLATFORM"], "${{ matrix.tooling }}");
        }
        if name == "Install macOS tooling" {
            // Hosted runners have no ambient GitHub credentials for cargo-binstall to discover.
            assert_eq!(
                step["env"]["GITHUB_TOKEN"], "${{ github.token }}",
                "macOS binary resolution must authenticate to the GitHub API"
            );
        }
    }
    let upload = steps.last().unwrap();
    assert_eq!(upload["with"]["name"], "runtime-${{ matrix.target }}");
    assert_eq!(upload["with"]["if-no-files-found"], "error");
    assert_eq!(upload["if"], "github.event_name == 'workflow_dispatch'");
    let contract = steps
        .iter()
        .find(|step| step["name"] == "Test the authenticated Cargo-Rail contract")
        .expect("Cargo-Rail contract step");
    let stable_command = steps
        .iter()
        .find(|step| step["name"] == "Test the stable Action command")
        .expect("stable Action command step");
    assert!(
        stable_command["run"]
            .as_str()
            .expect("stable Action command script")
            .contains("cargo-rail-action self-check")
    );
    assert!(
        contract["run"]
            .as_str()
            .expect("Cargo-Rail contract script")
            .contains("https://github.com/loadingalias/cargo-rail/releases/download/v$CARGO_RAIL_VERSION")
    );
    assert!(!contract["run"].as_str().unwrap().contains("/releases/latest/"));
    assert!(
        contract["run"]
            .as_str()
            .unwrap()
            .contains("$GITHUB_WORKSPACE/.cargo-rail-source")
    );
    assert_eq!(
        contract["env"]["CARGO_RAIL_VERSION"],
        "${{ steps.cargo-rail.outputs.version }}"
    );
    let collect = &package["jobs"]["collect"];
    assert_eq!(collect["needs"], "check");
    assert_eq!(collect["if"], "github.event_name == 'workflow_dispatch'");
    assert_eq!(collect["permissions"]["contents"], "read");
    assert_eq!(collect["permissions"]["id-token"], "write");
    assert_eq!(collect["permissions"]["attestations"], "write");
    assert_eq!(collect["permissions"]["artifact-metadata"], "write");
    let steps = collect["steps"].as_array().unwrap();
    let attestation = steps
        .iter()
        .find(|step| step["name"] == "Attest release assets")
        .expect("release asset attestation step");
    assert_eq!(
        attestation["uses"],
        "actions/attest@1e69f48acb82d1966a394da916b4c1698aa569d6"
    );
    assert_eq!(attestation["with"]["subject-path"], "${{ runner.temp }}/assets/*");
    let upload = steps.last().unwrap();
    assert_eq!(
        upload["with"]["name"],
        "release-cargo-rail-action-${{ github.run_id }}-${{ github.run_attempt }}"
    );

    let release_config = fs::read_to_string(root().join(".config/rail.toml")).unwrap();
    assert!(release_config.contains("workflow = \".github/workflows/package.yml\""));
    assert!(!release_config.contains("workflow = \".github/workflows/ci.yml\""));
}

#[test]
fn release_workflow_is_manual_exact_and_recoverable() {
    let workflow = yaml(".github/workflows/release.yml");
    assert_eq!(
        keys(mapping(&workflow["on"], "release triggers")),
        BTreeSet::from(["workflow_dispatch".into()])
    );
    assert_eq!(
        keys(mapping(
            &workflow["on"]["workflow_dispatch"]["inputs"],
            "release recovery inputs"
        )),
        BTreeSet::from(["intent".into(), "source".into(), "transaction".into()])
    );
    assert!(mapping(&workflow["permissions"], "permissions").is_empty());
    assert_eq!(workflow["concurrency"]["cancel-in-progress"], false);
    let job = &workflow["jobs"]["release"];
    assert_eq!(job["if"], "github.ref == 'refs/heads/main'");
    assert_eq!(job["environment"], "release");
    assert_eq!(job["permissions"]["contents"], "write");
    assert_eq!(job["permissions"]["actions"], "write");
    assert!(job["permissions"].get("pull-requests").is_none());
    let steps = job["steps"].as_array().unwrap();
    assert_eq!(steps[0]["with"]["persist-credentials"], false);
    let release = steps.last().unwrap();
    assert_eq!(
        release["env"]["INPUT_VERSION"],
        "${{ steps.cargo-rail.outputs.version }}"
    );
    assert_eq!(release["env"]["INPUT_BUMP"], "auto");
    assert_eq!(release["env"]["INPUT_PUBLISH"], "false");
    assert_eq!(release["env"]["INPUT_REVIEW"], "false");
    let command = release["run"].as_str().unwrap();
    assert!(command.contains("gh auth setup-git"));
    assert!(command.contains("cargo-rail-action run release"));
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
