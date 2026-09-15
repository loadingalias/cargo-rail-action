//! Independent invocation validation and execution of the Cargo-Rail release engine.

use crate::{
    ActionError, Result, env_string,
    github::{Publication, publish},
    install::{self, ComponentSet},
    plan::parse_unique_json,
    repository::{run_bounded, subprocess_failure},
};
use clap::Args;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Args)]
pub(crate) struct ExecuteArgs {
    /// Exact source-built core for coordinated bootstrap; normal actions install the authenticated release.
    #[arg(long)]
    cargo_rail: Option<PathBuf>,
    #[arg(long, default_value = "latest")]
    version: String,
    #[arg(long, default_value = "[]")]
    packages: String,
    #[arg(long, default_value = "auto")]
    bump: String,
    #[arg(long)]
    publish: bool,
    #[arg(long)]
    review: bool,
}

pub(crate) fn run_action() -> Result<()> {
    execute(&ExecuteArgs {
        cargo_rail: None,
        version: env_string("INPUT_VERSION")?,
        packages: env_string("INPUT_PACKAGES")?,
        bump: env_string("INPUT_BUMP")?,
        publish: boolean("INPUT_PUBLISH")?,
        review: boolean("INPUT_REVIEW")?,
    })
}

fn boolean(name: &str) -> Result<bool> {
    match env_string(name)?.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ActionError::rejected(format!("{name} must be true or false"))),
    }
}

pub(crate) fn execute(args: &ExecuteArgs) -> Result<()> {
    let root = std::fs::canonicalize(env_string("GITHUB_WORKSPACE")?)?;
    let repository = env_string("GITHUB_REPOSITORY")?;
    let event_name = env_string("GITHUB_EVENT_NAME")?;
    let workflow = env_string("GITHUB_WORKFLOW_REF")?;
    if env_string("GITHUB_ACTIONS")? != "true"
        || !matches!(event_name.as_str(), "workflow_dispatch" | "pull_request_target")
        || repository.split('/').count() != 2
        || !workflow.starts_with(&format!("{repository}/.github/workflows/"))
        || !workflow.contains("@refs/heads/")
    {
        return Err(ActionError::rejected(
            "release requires a same-repository trusted workflow invocation",
        ));
    }
    let event_path = PathBuf::from(env_string("GITHUB_EVENT_PATH")?);
    if std::fs::metadata(&event_path)?.len() > 16 * 1024 * 1024 {
        return Err(ActionError::rejected("release event exceeds its bound"));
    }
    let event = parse_unique_json(&std::fs::read(event_path)?, "release event")?;
    if event["repository"]["full_name"] != repository {
        return Err(ActionError::rejected(
            "release event repository differs from invocation",
        ));
    }
    install::validate_cargo_rail_selection(&args.version)?;
    let installed = if args.cargo_rail.is_none() {
        Some(install::install_cargo_rail(&args.version, ComponentSet::Core)?)
    } else {
        install::validate_cargo_rail_version(&args.version)
            .map_err(|_| ActionError::rejected("--cargo-rail requires an exact stable --version"))?;
        None
    };
    let binary = args
        .cargo_rail
        .as_deref()
        .or_else(|| installed.as_ref().map(|installed| installed.binary()))
        .ok_or_else(|| ActionError::operational("release core is unavailable"))?;
    let selected_version = installed
        .as_ref()
        .map_or(args.version.as_str(), |installed| installed.version());
    let version = run_bounded(Command::new(binary).arg("--version"), 1024, 1024)?;
    if !version.status.success()
        || String::from_utf8_lossy(&version.stdout).trim() != format!("cargo-rail {selected_version}")
    {
        return Err(ActionError::rejected(
            "release core version does not match the selected component",
        ));
    }
    let transaction = event["inputs"]["transaction"].as_str().unwrap_or("");
    let continuing = !transaction.is_empty() || event_name == "pull_request_target";
    let transaction = if continuing {
        let mut fetch = vec!["rail", "release", "record", "fetch"];
        if !transaction.is_empty() {
            fetch.push(transaction);
        }
        let fetched = core_json(binary, &root, &fetch)?;
        let transaction = fetched["transaction_id"]
            .as_str()
            .ok_or_else(|| ActionError::rejected("record fetch returned no transaction"))?;
        let record = core_json(binary, &root, &["rail", "release", "record", "inspect", transaction])?;
        authorize_record(&root, &record, &event, &event_name, &repository, &workflow)?;
        execute_core(binary, &root, &["rail", "release", "resume", transaction, "--executor"])?;
        transaction.to_owned()
    } else {
        if event_name != "workflow_dispatch" || event["inputs"]["intent"].as_str().is_some_and(|s| !s.is_empty()) {
            return Err(ActionError::rejected(
                "a new release request requires a manual dispatch without retained authority",
            ));
        }
        if String::from_utf8_lossy(&git(&root, &["rev-parse", "HEAD"])?).trim() != env_string("GITHUB_SHA")? {
            return Err(ActionError::rejected(
                "new release checkout differs from its dispatch source",
            ));
        }
        let packages: Vec<String> =
            serde_json::from_str(&args.packages).map_err(|_| ActionError::rejected("packages must be a JSON array"))?;
        if packages.len() > 256
            || packages.iter().any(|p| {
                p.is_empty()
                    || p.len() > 64
                    || !p.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            })
        {
            return Err(ActionError::rejected(
                "packages must contain bounded Cargo package names",
            ));
        }
        let mut command = vec!["rail", "release", "run", "--executor", "--yes", "--bump", &args.bump];
        if packages.is_empty() {
            command.push("--all");
        } else {
            command.extend(packages.iter().map(String::as_str));
        }
        if args.publish {
            command.push("--publish");
        }
        if args.review {
            command.push("--pr");
        }
        execute_core(binary, &root, &command)?;
        let fetched = core_json(binary, &root, &["rail", "release", "record", "fetch"])?;
        fetched["transaction_id"]
            .as_str()
            .ok_or_else(|| ActionError::rejected("release returned no transaction"))?
            .to_owned()
    };
    let record = core_json(binary, &root, &["rail", "release", "record", "inspect", &transaction])?;
    let source = source(&record)?;
    crate::release_record::read(
        &serde_json::to_vec(&record).map_err(|error| ActionError::rejected(error.to_string()))?,
        record["intent"]["identity"].as_str().unwrap_or(""),
        source,
        &format!("github.com/{repository}"),
    )?;
    publish(Publication {
        summary: None,
        paths: vec![],
        outputs: vec![
            ("transaction-id".into(), transaction),
            ("release-sha".into(), source.into()),
            ("state".into(), record["status"].as_str().unwrap_or("").into()),
            (
                "run-url".into(),
                format!(
                    "https://github.com/{repository}/actions/runs/{}",
                    env_string("GITHUB_RUN_ID")?
                ),
            ),
        ],
    })
}

fn source(record: &Value) -> Result<&str> {
    record["review"]["merge"]["commit"]
        .as_str()
        .or_else(|| record["preparation"]["commit"].as_str())
        .or_else(|| record["intent"]["initial_head"].as_str())
        .ok_or_else(|| ActionError::rejected("release record has no source"))
}

fn authorize_record(
    root: &Path,
    record: &Value,
    event: &Value,
    event_name: &str,
    repository: &str,
    workflow: &str,
) -> Result<()> {
    let source = source(record)?;
    let intent = &record["intent"];
    crate::release_record::read(
        &serde_json::to_vec(record).map_err(|error| ActionError::rejected(error.to_string()))?,
        intent["identity"].as_str().unwrap_or(""),
        source,
        &format!("github.com/{repository}"),
    )?;
    if intent["hosted"] != true
        || workflow
            != format!(
                "{repository}/{}@refs/heads/{}",
                intent["release_config"]["hosted_workflow"].as_str().unwrap_or(""),
                intent["branch"].as_str().unwrap_or("")
            )
    {
        return Err(ActionError::rejected("release record does not authorize this workflow"));
    }
    let observed;
    let pull = if event_name == "pull_request_target" {
        if event["action"] != "closed"
            || event["pull_request"]["merged"] != true
            || event["number"] != record["review"]["request"]
        {
            return Err(ActionError::rejected(
                "merge event does not authorize this release review",
            ));
        }
        Some(&event["pull_request"])
    } else {
        if event["inputs"]["transaction"] != record["transaction_id"]
            || event["inputs"]["intent"] != intent["identity"]
            || (event["inputs"]["source"] != source && event["inputs"]["source"] != intent["initial_head"])
        {
            return Err(ActionError::rejected(
                "dispatch does not authorize the original release intent",
            ));
        }
        if let Some(number) = record["review"]["request"].as_u64() {
            let output = run_bounded(
                Command::new("gh").current_dir(root).args([
                    "api",
                    "--hostname",
                    "github.com",
                    &format!("repos/{repository}/pulls/{number}"),
                ]),
                16 * 1024 * 1024,
                1024 * 1024,
            )?;
            if !output.status.success() {
                return Err(subprocess_failure("release review observation", &output));
            }
            observed = parse_unique_json(&output.stdout, "release review")?;
            if observed["number"] != number {
                return Err(ActionError::rejected("release review changed its request identity"));
            }
            Some(&observed)
        } else {
            None
        }
    };
    if let Some(pull) = pull {
        if pull["head"]["sha"] != record["preparation"]["commit"]
            || pull["head"]["ref"] != format!("rail/{}", record["transaction_id"].as_str().unwrap_or(""))
            || pull["head"]["repo"]["full_name"] != repository
            || pull["base"]["repo"]["full_name"] != repository
            || pull["base"]["ref"] != intent["branch"]
        {
            return Err(ActionError::rejected(
                "release review changed its repository, branch, or prepared source",
            ));
        }
        if pull["merged"] == false && pull["state"] == "open" && record["review"]["merge"].is_null() {
            return Ok(());
        }
        if pull["merged"] != true || pull["state"] != "closed" || pull["merged_at"].as_str().is_none_or(str::is_empty) {
            return Err(ActionError::rejected("release review has no observable merge"));
        }
        let merged = pull["merge_commit_sha"]
            .as_str()
            .filter(|s| matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| ActionError::rejected("release review has no exact merged source"))?;
        if record["review"]["merge"]["commit"]
            .as_str()
            .is_some_and(|retained| retained != merged)
        {
            return Err(ActionError::rejected(
                "release review changed its retained merged commit",
            ));
        }
        git(root, &["fetch", "--no-tags", "--no-write-fetch-head", "origin", merged])?;
        if git(root, &["show", "-s", "--format=%T", merged])?
            != git(
                root,
                &[
                    "show",
                    "-s",
                    "--format=%T",
                    record["preparation"]["commit"].as_str().unwrap_or(""),
                ],
            )?
        {
            return Err(ActionError::rejected(
                "reviewed merge changed the prepared release tree",
            ));
        }
    }
    Ok(())
}

fn core_json(binary: &Path, root: &Path, args: &[&str]) -> Result<Value> {
    let output = run_bounded(
        Command::new(binary).current_dir(root).args(args),
        16 * 1024 * 1024,
        1024 * 1024,
    )?;
    if !output.status.success() {
        return Err(subprocess_failure("release record operation", &output));
    }
    parse_unique_json(&output.stdout, "release record output")
}

fn execute_core(binary: &Path, root: &Path, args: &[&str]) -> Result<()> {
    if !Command::new(binary).current_dir(root).args(args).status()?.success() {
        return Err(ActionError::operational(
            "release engine stopped; the original transaction remains recoverable",
        ));
    }
    Ok(())
}

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = run_bounded(
        Command::new("git").current_dir(root).args(args),
        1024 * 1024,
        1024 * 1024,
    )?;
    if !output.status.success() {
        return Err(subprocess_failure("release Git observation", &output));
    }
    Ok(output.stdout)
}
