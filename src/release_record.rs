use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read as _;
use std::path::PathBuf;

use clap::Args;
use rscrypto::Sha256;
use serde_json::Value;

use crate::{ActionError, Result, plan::parse_unique_json, write_stdout};

const MAX_RECORD_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Args)]
pub(crate) struct ValidateArgs {
    record: PathBuf,
    #[arg(long)]
    intent: String,
    #[arg(long)]
    source: String,
    /// Normalized hosted repository, including its host.
    #[arg(long)]
    repository: String,
}

pub(crate) fn validate(arguments: &ValidateArgs) -> Result<()> {
    let metadata = std::fs::symlink_metadata(&arguments.record)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_RECORD_BYTES {
        return Err(ActionError::rejected("release record must be a bounded regular file"));
    }
    let mut bytes = Vec::new();
    File::open(&arguments.record)?
        .take(MAX_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)?;
    let value = read(&bytes, &arguments.intent, &arguments.source, &arguments.repository)?;
    write_stdout(
        format!(
            "{}\n",
            serde_json::json!({
                "transaction_id": value["transaction_id"],
                "intent": value["intent"]["identity"],
                "source": arguments.source,
                "state": value["status"],
                "phase": value["phase"],
            })
        )
        .as_bytes(),
    )
}

pub(crate) fn read(bytes: &[u8], expected_intent: &str, expected_source: &str, repository: &str) -> Result<Value> {
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(ActionError::rejected("release record exceeds 16 MiB"));
    }
    let value = parse_unique_json(bytes, "release record")?;
    require_integers(&value)?;
    let schema: Value = serde_json::from_str(include_str!("../schemas/release-record-v10.schema.json"))
        .map_err(|error| ActionError::operational(format!("invalid built-in release schema: {error}")))?;
    let validator = jsonschema::validator_for(&schema)
        .map_err(|error| ActionError::operational(format!("invalid built-in release schema: {error}")))?;
    validator
        .validate(&value)
        .map_err(|error| ActionError::rejected(format!("invalid release record: {error}")))?;
    let intent = &value["intent"];
    let mut unsigned = intent.clone();
    unsigned
        .as_object_mut()
        .expect("schema-validated intent")
        .remove("identity");
    let mut framed = b"cargo-rail-release-intent-v1\0".to_vec();
    framed.extend(
        serde_json::to_vec(&serde_json::json!({
            "transaction_id": value["transaction_id"], "intent": unsigned
        }))
        .map_err(|error| ActionError::rejected(error.to_string()))?,
    );
    let identity = format!("sha256:{}", digest(&framed));
    let source = value["review"]["merge"]
        .get("commit")
        .or_else(|| value["preparation"].get("commit"))
        .unwrap_or(&intent["initial_head"]);
    let remote = &intent["remote_repository"];
    if identity != expected_intent
        || intent["identity"] != identity
        || source != expected_source
        || remote["host"]
            .as_str()
            .zip(remote["path"].as_str())
            .is_none_or(|(host, path)| format!("{host}/{path}") != repository)
    {
        return Err(ActionError::rejected(
            "release record does not match the authorized intent, source, and repository",
        ));
    }
    let plan = &intent["plan"];
    let packages = plan["crates"].as_array().expect("schema-validated crates");
    let names = packages
        .iter()
        .map(|package| package["name"].as_str().expect("schema-validated name"))
        .collect::<Vec<_>>();
    let tags = packages
        .iter()
        .map(|package| package["tag_name"].as_str().expect("schema-validated tag"))
        .collect::<BTreeSet<_>>();
    let progress = value["crates"].as_array().expect("schema-validated progress");
    if names.iter().copied().collect::<BTreeSet<_>>().len() != names.len()
        || tags.len() != names.len()
        || plan["canonical_crate_order"] != serde_json::json!(names)
        || plan["summary"]["total_crates"] != packages.len()
        || plan["summary"]["crates_to_tag"] != packages.len()
        || plan["summary"]["crates_to_publish"] != packages.iter().filter(|package| package["publish"] == true).count()
        || plan["source"] != intent["release_config"]["source"]
        || progress.len() != names.len()
        || progress
            .iter()
            .zip(&names)
            .any(|(package, name)| package["name"] != *name)
        || (intent["skip_publish"] == true) != intent.get("publish_registry").is_none()
        || intent
            .get("publish_registry")
            .is_some_and(|registry| registry != &intent["release_config"]["registry_publication"])
    {
        return Err(ActionError::rejected(
            "release record contains inconsistent selection or publication authority",
        ));
    }
    for package in packages {
        for field in ["current_version", "new_version"] {
            if package[field]
                .as_str()
                .is_none_or(|version| semver::Version::parse(version).is_err())
            {
                return Err(ActionError::rejected(
                    "release record contains an invalid package version",
                ));
            }
        }
        for dependency in package["dependency_updates"]
            .as_array()
            .expect("schema-validated dependency updates")
        {
            if dependency["version"]
                .as_str()
                .is_none_or(|version| semver::Version::parse(version).is_err())
            {
                return Err(ActionError::rejected(
                    "release record contains an invalid dependency version",
                ));
            }
        }
        let api = &package["api_evidence"];
        if (api["required"] == true) != (intent["release_config"]["semver_check"] == "deny") {
            return Err(ActionError::rejected(
                "release API evidence does not match required policy",
            ));
        }
        if api["outcome"] == "fail" || api["required"] == true && api["outcome"] == "unavailable" {
            return Err(ActionError::rejected("release record has blocking API evidence"));
        }
        let write = &package["presentation"]["changelog"];
        if (package["generate_changelog"] == true) == write.is_null()
            || !write.is_null() && (write["path"] != package["changelog_path"] || !matches_content(write))
        {
            return Err(ActionError::rejected("release record has inconsistent changelog bytes"));
        }
    }
    for write in plan["auxiliary_lockfiles"]
        .as_array()
        .expect("schema-validated lockfiles")
    {
        if write["content"]
            .as_str()
            .is_none_or(|content| write["after_digest"] != format!("sha256:{}", digest(content.as_bytes())))
        {
            return Err(ActionError::rejected("release record has inconsistent lockfile bytes"));
        }
    }
    let seal = &value["package_seal"];
    if !seal.is_null() {
        let selected = packages
            .iter()
            .filter(|package| package["publish"] == true)
            .collect::<Vec<_>>();
        let archives = seal["packages"].as_array().expect("schema-validated archives");
        if value["preparation"]["status"] != "complete"
            || seal["source_commit"] != *source
            || archives.len() != selected.len()
            || archives.iter().zip(selected).any(|(archive, package)| {
                archive["name"] != package["name"] || archive["version"] != package["new_version"]
            })
        {
            return Err(ActionError::rejected(
                "release package seal does not match the prepared selection",
            ));
        }
    }
    for (progress, package) in progress.iter().zip(packages) {
        let tag = &progress["tag_object"];
        if !tag.is_null() {
            let prefix = format!(
                "object {}\ntype commit\ntag {}\ntagger ",
                source.as_str().expect("schema-validated source"),
                package["tag_name"].as_str().expect("schema-validated tag")
            );
            if intent["skip_tag"] == true
                || progress["tag"]["status"] != "complete"
                || progress["tag"]["object"] != tag["id"]
                || !tag["content"].as_str().is_some_and(|content| {
                    content.len() <= 65536
                        && content.starts_with(&prefix)
                        && content.split_once("\n\n").is_some_and(|(_, body)| {
                            body.starts_with(&format!(
                                "Release {} v{}\n",
                                package["name"].as_str().expect("schema-validated name"),
                                package["new_version"].as_str().expect("schema-validated version")
                            ))
                        })
                })
            {
                return Err(ActionError::rejected(
                    "release tag object does not match its prepared identity",
                ));
            }
        } else if intent["skip_tag"] != true && progress["tag"]["status"] == "complete" {
            return Err(ActionError::rejected("completed release tag has no retained object"));
        }
        let publication = &progress["publication"];
        let attempt = &progress["publication_attempt"];
        if publication["status"] == "pending" && !attempt.is_null()
            || publication["status"] == "in_progress" && attempt.is_null()
        {
            return Err(ActionError::rejected(
                "release publication has inconsistent upload attempt identity",
            ));
        }
        if intent["skip_publish"] != true && package["publish"] == true && publication["status"] != "pending" {
            if !matches!(value["phase"].as_str(), Some("publishing" | "released"))
                || value["readiness"]["status"] != "complete"
                || value["commit_push"]["status"] != "complete"
                || intent["skip_tag"] != true
                    && value["crates"]
                        .as_array()
                        .expect("schema-validated progress")
                        .iter()
                        .any(|package| package["tag"]["status"] != "complete")
            {
                return Err(ActionError::rejected(
                    "release upload precedes required preparation and validation",
                ));
            }
            let archive = seal["packages"]
                .as_array()
                .and_then(|archives| archives.iter().find(|archive| archive["name"] == package["name"]));
            if archive.is_none_or(|archive| publication["object"] != archive["sha256"]) {
                return Err(ActionError::rejected(
                    "release publication progress has no matching sealed archive",
                ));
            }
        }
    }
    let required = intent["release_config"]["validation"]
        .as_object()
        .expect("schema-validated validation policy");
    let runs = value["validation"]
        .as_array()
        .expect("schema-validated validation evidence");
    if !runs.is_empty()
        && (runs.len() != required.len()
            || runs.iter().zip(required).any(|(run, (workflow, jobs))| {
                run["workflow"] != *workflow
                    || run["jobs"]
                        .as_array()
                        .expect("schema-validated jobs")
                        .iter()
                        .map(|job| job["id"].as_u64().expect("schema-validated job ID"))
                        .collect::<BTreeSet<_>>()
                        .len()
                        != jobs.as_array().expect("schema-validated required jobs").len()
                    || run["jobs"]
                        .as_array()
                        .expect("schema-validated jobs")
                        .iter()
                        .map(|job| &job["name"])
                        .ne(jobs.as_array().expect("schema-validated required jobs").iter())
            })
            || runs
                .iter()
                .map(|run| run["run_id"].as_u64().expect("schema-validated run ID"))
                .collect::<BTreeSet<_>>()
                .len()
                != runs.len())
    {
        return Err(ActionError::rejected(
            "release workflow evidence does not cover the required validation",
        ));
    }
    let remote_effects = intent["release_config"]["remote_effects"]
        .as_str()
        .expect("schema-validated remote effects");
    let github_validation_required = remote_effects != "none"
        && (intent["skip_tag"] != true || intent["skip_publish"] != true)
        && (remote_effects == "github" || remote["host"] == "github.com");
    if github_validation_required
        && (required.is_empty() || value["readiness"]["status"] == "complete" && runs.is_empty())
    {
        return Err(ActionError::rejected(
            "release readiness has no required workflow evidence",
        ));
    }
    let review = &value["review"];
    if (intent["review"] == true) != !review.is_null()
        || !review.is_null()
            && (value["remote_storage"] != true
                || remote_effects == "none"
                || remote["host"] != "github.com"
                || review["pushed"]["status"] != "pending"
                    && (value["preparation"]["status"] != "complete"
                        || review["pushed"]["object"] != value["preparation"]["commit"])
                || !review["request"].is_null() && review["pushed"]["status"] != "complete"
                || !review["merge"].is_null() && review["request"].is_null()
                || review["merge"].is_null()
                    && (!value["package_seal"].is_null()
                        || !runs.is_empty()
                        || matches!(value["phase"].as_str(), Some("ready" | "publishing" | "released"))))
    {
        return Err(ActionError::rejected(
            "reviewed release contradicts its authorized source or effect ordering",
        ));
    }
    let dispatches = value["validation_dispatches"]
        .as_object()
        .expect("schema-validated dispatches");
    if intent["hosted"] == true
        && (value["remote_storage"] != true
            || intent["release_config"]["hosted_workflow"].is_null()
            || remote["host"] != "github.com")
        || !value["executor"].is_null() && intent["hosted"] != true
        || dispatches.iter().any(|(workflow, run)| {
            intent["hosted"] != true || !required.contains_key(workflow) || run == &value["executor"]
        })
        || runs.iter().any(|run| {
            dispatches
                .get(run["workflow"].as_str().unwrap_or(""))
                .is_some_and(|dispatch| dispatch != &run["run_id"])
        })
    {
        return Err(ActionError::rejected(
            "hosted release has inconsistent workflow authority",
        ));
    }
    let aliases = intent["release_config"]["aliases"]
        .as_object()
        .expect("schema-validated aliases");
    let previous = intent["alias_previous"]
        .as_object()
        .expect("schema-validated alias authority");
    let selected_aliases = aliases
        .keys()
        .filter(|package| names.contains(&package.as_str()))
        .collect::<BTreeSet<_>>();
    if aliases.values().any(|value| {
        value.as_str().is_none_or(|alias| {
            alias.is_empty()
                || alias.len() > 128
                || alias.contains("..")
                || alias.ends_with('.')
                || alias.ends_with(".lock")
                || !alias.as_bytes()[0].is_ascii_alphanumeric()
                || !alias
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        })
    }) || aliases
        .values()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>()
        .len()
        != aliases.len()
    {
        return Err(ActionError::rejected("release aliases must be unique bounded Git tags"));
    }
    if previous.keys().collect::<BTreeSet<_>>() != selected_aliases
        || !previous.is_empty()
            && (intent["skip_tag"] == true || !matches!(remote_effects, "auto" | "github" | "gitlab"))
        || progress.iter().any(|package| {
            let selected = previous.contains_key(package["name"].as_str().unwrap_or(""));
            package["alias"]["status"] == "pending" && !package["alias"]["object"].is_null()
                || package["alias"]["status"] != "pending"
                    && (!selected
                        || package["forge_publication"]["status"] != "complete"
                        || package["alias"]["object"] != package["tag_object"]["id"])
                || value["status"] == "complete" && selected && package["alias"]["status"] != "complete"
        })
    {
        return Err(ActionError::rejected(
            "release alias promotion precedes its verified immutable release",
        ));
    }
    validate_artifacts(&value)?;
    let complete = |step: &Value| step["status"] == "complete";
    if value["phase"] != "planned" && value["preparation"]["status"] != "complete"
        || value["phase"] == "released" && value["status"] != "complete"
        || value["status"] == "aborted" && !complete(&value["abort"])
        || value["status"] == "complete"
            && (value["phase"] != "released"
                || ["commit_push", "readiness", "tag_push"]
                    .iter()
                    .any(|field| !complete(&value[field]))
                || progress.iter().any(|package| {
                    !complete(&package["tag"])
                        || !complete(&package["publication"])
                        || matches!(remote_effects, "auto" | "github" | "gitlab")
                            && intent["skip_tag"] != true
                            && (!complete(&package["forge_draft"]) || !complete(&package["forge_publication"]))
                }))
    {
        return Err(ActionError::rejected(
            "release terminal state contradicts its retained effects",
        ));
    }
    Ok(value)
}

fn matches_content(value: &Value) -> bool {
    value["content"]
        .as_str()
        .is_some_and(|content| value["after_digest"] == digest(content.as_bytes()))
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn require_integers(value: &Value) -> Result<()> {
    match value {
        Value::Number(number) if !number.is_i64() && !number.is_u64() => Err(ActionError::rejected(
            "release records cannot contain floating-point numbers",
        )),
        Value::Array(values) => values.iter().try_for_each(require_integers),
        Value::Object(values) => values.values().try_for_each(require_integers),
        _ => Ok(()),
    }
}

fn asset_filename(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && !name.ends_with('.')
        && !matches!(
            name.split('.').next().unwrap_or_default().to_ascii_uppercase().as_str(),
            "CON"
                | "PRN"
                | "AUX"
                | "NUL"
                | "COM1"
                | "COM2"
                | "COM3"
                | "COM4"
                | "COM5"
                | "COM6"
                | "COM7"
                | "COM8"
                | "COM9"
                | "LPT1"
                | "LPT2"
                | "LPT3"
                | "LPT4"
                | "LPT5"
                | "LPT6"
                | "LPT7"
                | "LPT8"
                | "LPT9"
        )
}

fn validate_artifacts(record: &Value) -> Result<()> {
    let intent = &record["intent"];
    let config = &intent["release_config"];
    let plan = &intent["plan"];
    let packages = plan["crates"].as_array().expect("validated packages");
    let mut expected = Vec::new();
    for (name, producer) in config["artifacts"].as_object().expect("validated artifact policy") {
        let workflow = producer["workflow"].as_str().expect("validated workflow");
        if !asset_filename(name) || name.len() > 64 || config["validation"].get(workflow).is_none() {
            return Err(ActionError::rejected(
                "native artifact has no valid package or authorized workflow",
            ));
        }
        let mut files = Vec::new();
        let mut names = BTreeSet::new();
        for (template, file) in producer["files"].as_object().expect("validated asset policy") {
            let rendered = template.replace("{crate}", name).replace("{version}", "0.0.0");
            if !asset_filename(&rendered)
                || !names.insert(rendered.to_ascii_lowercase())
                || file["target"]
                    .as_str()
                    .is_some_and(|target| !asset_filename(target) || !target.contains('-'))
            {
                return Err(ActionError::rejected(
                    "native artifact has an unsafe or conflicting filename or target",
                ));
            }
            if let Some(package) = packages.iter().find(|package| package["name"] == *name) {
                let rendered = template
                    .replace("{crate}", name)
                    .replace("{version}", package["new_version"].as_str().expect("validated version"));
                if !asset_filename(&rendered) {
                    return Err(ActionError::rejected("resolved native asset filename is unsafe"));
                }
                files.push(serde_json::json!({"name":rendered,"target":file["target"],"source":file["source"]}));
            }
        }
        if !files.is_empty() {
            files.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
            if files
                .iter()
                .map(|file| file["name"].as_str().expect("resolved name").to_ascii_lowercase())
                .collect::<BTreeSet<_>>()
                .len()
                != files.len()
            {
                return Err(ActionError::rejected("resolved native asset filenames collide"));
            }
            expected.push(serde_json::json!({"package":name,"workflow":workflow,"files":files}));
        }
    }
    if plan["artifacts"] != serde_json::json!(expected) {
        return Err(ActionError::rejected(
            "native asset requirements disagree with the captured configuration",
        ));
    }
    if !expected.is_empty()
        && (intent["skip_tag"] == true
            || !matches!(config["remote_effects"].as_str(), Some("auto" | "github"))
            || config["remote_effects"] != "github" && intent["remote_repository"]["host"] != "github.com")
    {
        return Err(ActionError::rejected(
            "native artifacts have no authorized GitHub release effects",
        ));
    }
    let artifacts = record["artifacts"].as_array().expect("validated artifact evidence");
    if artifacts.is_empty()
        && matches!(
            record["phase"].as_str(),
            Some("planned" | "prepared" | "awaiting_review" | "awaiting_checks")
        )
    {
        return Ok(());
    }
    if artifacts.len() != expected.len() || !artifacts.is_empty() && record["readiness"]["status"] != "complete" {
        return Err(ActionError::rejected(
            "release has no complete authorized native artifact inventory",
        ));
    }
    let mut ids = BTreeSet::new();
    for (artifact, required) in artifacts.iter().zip(&expected) {
        let run = record["validation"]
            .as_array()
            .expect("validated workflows")
            .iter()
            .find(|run| run["workflow"] == required["workflow"]);
        if artifact["package"] != required["package"]
            || !ids.insert(artifact["artifact_id"].as_u64().expect("validated artifact ID"))
            || run.is_none_or(|run| run["run_id"] != artifact["run_id"] || run["attempt"] != artifact["attempt"])
            || artifact["files"]
                .as_array()
                .expect("validated assets")
                .iter()
                .map(|file| &file["name"])
                .ne(required["files"]
                    .as_array()
                    .expect("resolved files")
                    .iter()
                    .map(|file| &file["name"]))
        {
            return Err(ActionError::rejected(
                "native artifact evidence has the wrong producer, package, or inventory",
            ));
        }
    }
    Ok(())
}
