//! Run `cargo rail release` for a workflow and publish its result.
//!
//! Releases keep no records: Cargo-Rail reads progress from Git, the registry, and GitHub,
//! and a rerun finishes an interrupted release. This runner validates only what it hands
//! the engine and what it publishes: the inputs, the installed engine, and the result's shape.
//! Push credentials reach the engine process alone, through process-scoped Git configuration.

use crate::{
    ActionError, Result, env_string,
    github::{Publication, publish},
    install::{self, ComponentSet},
    optional_env,
};
use clap::Args;
use serde_json::Value;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const MAX_RESULT_BYTES: u64 = 4 * 1024 * 1024;
const STATES: [&str; 7] = [
    "nothing_to_release",
    "planned",
    "cancelled",
    "pull_request",
    "awaiting_checks",
    "awaiting_assets",
    "released",
];

#[derive(Debug, Args)]
pub(crate) struct ExecuteArgs {
    /// Exact source-built core for coordinated bootstrap; normal actions install the authenticated release.
    #[arg(long)]
    cargo_rail: Option<PathBuf>,
    #[arg(long, default_value = "latest")]
    version: String,
    /// Crates to release, separated by whitespace; empty selects every crate with release-worthy changes.
    #[arg(long, default_value = "")]
    crates: String,
    #[arg(long, default_value = "auto")]
    bump: String,
    #[arg(long)]
    pull_request: bool,
    #[arg(long)]
    dry_run: bool,
}

pub(crate) fn run_action() -> Result<()> {
    execute(&ExecuteArgs {
        cargo_rail: None,
        version: env_string("INPUT_VERSION")?,
        crates: optional_env("INPUT_CRATES")?,
        bump: env_string("INPUT_BUMP")?,
        pull_request: boolean("INPUT_PULL_REQUEST")?,
        dry_run: boolean("INPUT_DRY_RUN")?,
    })
}

fn boolean(name: &str) -> Result<bool> {
    match env_string(name)?.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ActionError::rejected(format!("{name} must be true or false"))),
    }
}

/// The release command line for these inputs, rejecting anything Cargo-Rail could read as an option.
fn release_arguments(args: &ExecuteArgs) -> Result<Vec<String>> {
    let crates = args.crates.split_whitespace().collect::<Vec<_>>();
    if crates.len() > 256
        || crates.iter().any(|name| {
            name.len() > 64
                || !name.starts_with(|first: char| first.is_ascii_alphanumeric())
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
    {
        return Err(ActionError::rejected(
            "crates must be Cargo package names separated by whitespace",
        ));
    }
    if args.bump.is_empty()
        || args.bump.len() > 64
        || args.bump.starts_with('-')
        || !args
            .bump
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
    {
        return Err(ActionError::rejected(
            "bump must be auto, major, minor, patch, prerelease, release, or a version",
        ));
    }
    let mut arguments = vec!["rail", "release", "--format", "json", "--bump", &args.bump];
    arguments.push(if args.dry_run { "--dry-run" } else { "--yes" });
    if args.pull_request {
        arguments.push("--pr");
    }
    arguments.push("--");
    arguments.extend(crates);
    Ok(arguments.into_iter().map(str::to_owned).collect())
}

/// Process-scoped Git configuration that pushes to `server` with `token` and nothing else.
fn push_credentials(server: &str, token: &str) -> Result<Vec<(String, String)>> {
    let server = server.trim_end_matches('/');
    if !server.starts_with("https://") || server.contains(['\r', '\n', ' ']) {
        return Err(ActionError::rejected("GITHUB_SERVER_URL must be an HTTPS URL"));
    }
    if token.is_empty() || token.contains(['\r', '\n']) {
        return Err(ActionError::rejected("token must be a single-line value"));
    }
    let header = format!(
        "AUTHORIZATION: basic {}",
        base64(format!("x-access-token:{token}").as_bytes())
    );
    Ok(vec![
        ("GIT_CONFIG_COUNT".into(), "1".into()),
        ("GIT_CONFIG_KEY_0".into(), format!("http.{server}/.extraheader")),
        ("GIT_CONFIG_VALUE_0".into(), header),
    ])
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let value = chunk.iter().enumerate().fold(0_u32, |value, (index, byte)| {
            value | (u32::from(*byte) << (16 - 8 * index))
        });
        for index in 0..4 {
            if index <= chunk.len() {
                encoded.push(char::from(ALPHABET[((value >> (18 - 6 * index)) & 63) as usize]));
            } else {
                encoded.push('=');
            }
        }
    }
    encoded
}

/// A commit identity for the release commit when the checkout configures none.
fn identity(root: &Path) -> Vec<(String, String)> {
    let configured = Command::new("git")
        .current_dir(root)
        .args(["config", "user.email"])
        .output()
        .is_ok_and(|output| output.status.success() && !output.stdout.is_empty());
    if configured {
        return Vec::new();
    }
    let (name, email) = (
        "github-actions[bot]",
        "41898282+github-actions[bot]@users.noreply.github.com",
    );
    ["AUTHOR", "COMMITTER"]
        .into_iter()
        .flat_map(|role| {
            [
                (format!("GIT_{role}_NAME"), name.to_owned()),
                (format!("GIT_{role}_EMAIL"), email.to_owned()),
            ]
        })
        .collect()
}

pub(crate) fn execute(args: &ExecuteArgs) -> Result<()> {
    let root = std::fs::canonicalize(env_string("GITHUB_WORKSPACE")?)?;
    if env_string("GITHUB_ACTIONS")? != "true" {
        return Err(ActionError::rejected("release runs inside GitHub Actions"));
    }
    let token = env_string("GH_TOKEN")?;
    let server = optional_env("GITHUB_SERVER_URL")?;
    let server = if server.is_empty() {
        "https://github.com".to_owned()
    } else {
        server
    };
    let arguments = release_arguments(args)?;
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
    let selected = installed
        .as_ref()
        .map_or(args.version.as_str(), |installed| installed.version());
    let version = Command::new(binary).arg("--version").output()?;
    if !version.status.success() || String::from_utf8_lossy(&version.stdout).trim() != format!("cargo-rail {selected}")
    {
        return Err(ActionError::rejected(
            "release core version does not match the selected component",
        ));
    }

    let mut child = Command::new(binary)
        .current_dir(&root)
        .args(&arguments)
        .envs(push_credentials(&server, &token)?)
        .envs(identity(&root))
        .env("GH_TOKEN", &token)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let mut stdout = Vec::new();
    child
        .stdout
        .take()
        .ok_or_else(|| ActionError::operational("release output is unavailable"))?
        .take(MAX_RESULT_BYTES + 1)
        .read_to_end(&mut stdout)?;
    let status = child.wait()?;
    if stdout.len() as u64 > MAX_RESULT_BYTES {
        return Err(ActionError::rejected("release output exceeds its bound"));
    }
    let result: Value = serde_json::from_slice(&stdout)
        .map_err(|_| ActionError::operational(format!("cargo rail release exited with {status} and no result")))?;
    if result["error"] == true {
        return Err(ActionError::operational(
            result["message"]
                .as_str()
                .unwrap_or("cargo rail release failed")
                .to_owned(),
        ));
    }
    let outputs = outputs(&result)?;
    let summary = summary(&result);
    publish(Publication {
        summary: Some(summary),
        paths: Vec::new(),
        outputs,
    })?;
    match result["state"].as_str() {
        Some("awaiting_checks") => Err(ActionError::operational(format!(
            "checks on the source commit have not finished: {}",
            result["pending_checks"]
                .as_array()
                .map(|checks| checks.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
                .unwrap_or_default()
        ))),
        _ if !status.success() => Err(ActionError::operational(format!(
            "cargo rail release exited with {status}"
        ))),
        _ => Ok(()),
    }
}

/// The step outputs, after checking every field they come from.
fn outputs(result: &Value) -> Result<Vec<(String, String)>> {
    let state = result["state"]
        .as_str()
        .filter(|state| STATES.contains(state))
        .ok_or_else(|| ActionError::rejected("release result has an unknown state"))?;
    if result["command"] != "release" {
        return Err(ActionError::rejected("release result names another command"));
    }
    let commit = match &result["commit"] {
        Value::Null => String::new(),
        Value::String(commit)
            if matches!(commit.len(), 40 | 64) && commit.bytes().all(|byte| byte.is_ascii_hexdigit()) =>
        {
            commit.clone()
        }
        _ => return Err(ActionError::rejected("release result has an invalid commit")),
    };
    let tags = result["crates"]
        .as_array()
        .ok_or_else(|| ActionError::rejected("release result has no crates"))?
        .iter()
        .map(|release| {
            release["tag"]
                .as_str()
                .filter(|tag| {
                    !tag.is_empty()
                        && tag.len() <= 128
                        && tag
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+'))
                })
                .map(str::to_owned)
                .ok_or_else(|| ActionError::rejected("release result has an invalid tag"))
        })
        .collect::<Result<Vec<_>>>()?;
    let pull_request = match &result["pull_request"] {
        Value::Null => String::new(),
        Value::String(url) if url.starts_with("http") && !url.contains(['\r', '\n']) => url.clone(),
        _ => return Err(ActionError::rejected("release result has an invalid pull request URL")),
    };
    Ok(vec![
        ("state".into(), state.to_owned()),
        ("commit".into(), commit),
        ("tags".into(), serde_json::to_string(&tags).unwrap_or_default()),
        ("pull-request".into(), pull_request),
    ])
}

fn summary(result: &Value) -> String {
    let mut summary = format!(
        "### Cargo-Rail release: {}\n\n",
        result["state"].as_str().unwrap_or("unknown").replace('_', " ")
    );
    let crates = result["crates"].as_array().cloned().unwrap_or_default();
    if !crates.is_empty() {
        summary.push_str("| Crate | Version | Tag | Registry | GitHub release |\n|---|---|---|---|---|\n");
        for release in crates {
            let cell = |value: &Value| value.as_str().unwrap_or("").replace(['|', '\n'], " ");
            summary.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                cell(&release["name"]),
                cell(&release["version"]),
                cell(&release["tag"]),
                cell(&release["registry"]),
                cell(&release["github_release"]),
            ));
        }
    }
    if let Some(next) = result["next"].as_str() {
        summary.push_str(&format!("\nNext: {}\n", next.replace('\n', " ")));
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(crates: &str, bump: &str) -> ExecuteArgs {
        ExecuteArgs {
            cargo_rail: None,
            version: "latest".into(),
            crates: crates.into(),
            bump: bump.into(),
            pull_request: false,
            dry_run: false,
        }
    }

    #[test]
    fn release_arguments_pass_names_after_a_separator_and_reject_options() {
        assert_eq!(
            release_arguments(&args("lib-a  app", "minor")).unwrap(),
            [
                "rail", "release", "--format", "json", "--bump", "minor", "--yes", "--", "lib-a", "app"
            ]
        );
        for (crates, bump) in [("--publish", "auto"), ("a;b", "auto"), ("", "-x"), ("", "")] {
            assert!(release_arguments(&args(crates, bump)).is_err(), "{crates} {bump}");
        }
    }

    #[test]
    fn push_credentials_are_process_scoped_basic_auth_for_the_server() {
        let credentials = push_credentials("https://github.com/", "token").unwrap();
        assert_eq!(credentials[1].1, "http.https://github.com/.extraheader");
        assert_eq!(credentials[2].1, "AUTHORIZATION: basic eC1hY2Nlc3MtdG9rZW46dG9rZW4=");
        assert!(push_credentials("http://github.com", "token").is_err());
        assert!(push_credentials("https://github.com", "a\nb").is_err());
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"a"), "YQ==");
    }

    #[test]
    fn outputs_come_only_from_a_valid_result() {
        let result = serde_json::json!({
            "command": "release",
            "state": "awaiting_assets",
            "commit": "0123456789abcdef0123456789abcdef01234567",
            "crates": [{ "name": "app", "version": "1.2.0", "tag": "v1.2.0", "registry": "pending", "github_release": "pending" }],
        });
        assert_eq!(
            outputs(&result).unwrap(),
            [
                ("state".into(), "awaiting_assets".into()),
                ("commit".into(), "0123456789abcdef0123456789abcdef01234567".into()),
                ("tags".into(), "[\"v1.2.0\"]".into()),
                ("pull-request".into(), String::new()),
            ]
        );
        for (field, value) in [
            ("state", serde_json::json!("done")),
            ("commit", serde_json::json!("main")),
            ("crates", serde_json::json!([{ "tag": "v1\nx" }])),
            ("command", serde_json::json!("plan")),
        ] {
            let mut invalid = result.clone();
            invalid[field] = value;
            assert!(outputs(&invalid).is_err(), "{field}");
        }
        assert!(summary(&result).contains("| app | 1.2.0 | v1.2.0 | pending | pending |"));
    }
}
