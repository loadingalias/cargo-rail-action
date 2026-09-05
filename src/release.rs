use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Args;
use rscrypto::Sha256;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::plan::parse_unique_json;
use crate::repository::{run_bounded, run_bounded_gh, subprocess_failure};
use crate::{ActionError, MAX_RUNTIME_BYTES, QUALIFIED_TARGETS, Result, VERSION, env_string};

const INTENT_SCHEMA_VERSION: u32 = 1;
const MAX_INTENT_BYTES: u64 = 64 * 1024;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_RELEASE_RUNTIME_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SUBPROCESS_BYTES: usize = 1024 * 1024;

#[derive(Debug, Args)]
pub(crate) struct IntentArgs {
    #[arg(long)]
    source_commit: String,
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long, required = true)]
    asset: Vec<PathBuf>,
    #[arg(long)]
    output: PathBuf,
}

#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
pub(crate) struct EffectArgs {
    #[arg(long, group = "effect")]
    check: bool,
    #[arg(long, group = "effect")]
    apply: bool,
    #[arg(long)]
    intent: PathBuf,
    #[arg(long, required = true)]
    asset: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct AssetRecord {
    name: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ReleaseIntent {
    schema_version: u32,
    identity: String,
    action_version: String,
    source_commit: String,
    cargo_rail_version: String,
    runtime_manifest_sha256: String,
    assets: Vec<AssetRecord>,
}

#[derive(Debug, Serialize)]
struct PortableIntent<'a> {
    schema_version: u32,
    action_version: &'a str,
    source_commit: &'a str,
    cargo_rail_version: &'a str,
    runtime_manifest_sha256: &'a str,
    assets: &'a [AssetRecord],
}

#[derive(Debug)]
struct RemoteRelease {
    draft: bool,
    prerelease: bool,
    target: String,
    body: String,
    assets: BTreeMap<String, (u64, String)>,
}

pub(crate) fn write_intent(arguments: &IntentArgs) -> Result<()> {
    validate_source_commit(&arguments.source_commit)?;
    let cargo_rail_version = preflight_cargo_rail_version()?;
    let runtime_assets = asset_records(&arguments.asset)?;
    let manifest = match std::fs::symlink_metadata(&arguments.manifest) {
        Ok(_) => read_bounded(&arguments.manifest, MAX_MANIFEST_BYTES, "runtime manifest")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let bytes = generate_runtime_manifest(&runtime_assets)?;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&arguments.manifest)
                .and_then(|mut output| output.write_all(&bytes).and_then(|()| output.flush()))
                .map_err(|error| {
                    ActionError::operational(format!(
                        "cannot create runtime manifest '{}': {error}",
                        arguments.manifest.display()
                    ))
                })?;
            bytes
        }
        Err(error) => {
            return Err(ActionError::operational(format!(
                "cannot inspect runtime manifest '{}': {error}",
                arguments.manifest.display()
            )));
        }
    };
    let manifest_rows = validate_runtime_manifest(&manifest)?;
    let mut asset_paths = arguments.asset.clone();
    asset_paths.push(arguments.manifest.clone());
    let assets = asset_records(&asset_paths)?;
    validate_assets_against_manifest(&assets, &manifest_rows, &arguments.manifest)?;
    let runtime_manifest_sha256 = digest_bytes(&manifest);
    let mut intent = ReleaseIntent {
        schema_version: INTENT_SCHEMA_VERSION,
        identity: String::new(),
        action_version: VERSION.to_string(),
        source_commit: arguments.source_commit.clone(),
        cargo_rail_version,
        runtime_manifest_sha256,
        assets,
    };
    intent.identity = intent_identity(&intent)?;
    validate_intent(&intent)?;
    let mut bytes = serde_json::to_vec(&intent)
        .map_err(|error| ActionError::operational(format!("cannot encode release intent: {error}")))?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_INTENT_BYTES {
        return Err(ActionError::rejected("release intent exceeds 64 KiB"));
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&arguments.output)
        .and_then(|mut output| output.write_all(&bytes).and_then(|()| output.flush()))
        .map_err(|error| {
            ActionError::operational(format!(
                "cannot create release intent '{}': {error}",
                arguments.output.display()
            ))
        })?;
    println!("Cargo-Rail Action release intent ready: {}", intent.identity);
    Ok(())
}

pub(crate) fn preflight() -> Result<()> {
    let version = preflight_cargo_rail_version()?;
    println!("Cargo-Rail Action release preflight ready: Cargo-Rail {version}");
    Ok(())
}

fn preflight_cargo_rail_version() -> Result<String> {
    let version = metadata_cargo_rail_version(Path::new("."))?;
    crate::install::validate_cargo_rail_version(&version)?;
    verify_cargo_rail_release_assets(&version)?;
    Ok(version)
}

pub(crate) fn publish(arguments: &EffectArgs) -> Result<()> {
    let intent = load_intent(&arguments.intent)?;
    let local_assets = asset_records(&arguments.asset)?;
    if local_assets != intent.assets {
        return Err(ActionError::rejected(
            "release assets disagree with the durable release intent",
        ));
    }
    validate_local_release_authority(&intent)?;
    let repository = repository_name()?;
    let token = github_token()?;
    let tag = format!("v{}", intent.action_version);
    validate_exact_tag(&repository, &tag, &intent.source_commit, &token, true)?;
    let existing = inspect_release(&repository, &tag, &token)?;
    if let Some(release) = &existing {
        validate_remote_release(release, &intent, release.draft)?;
    }
    if arguments.check {
        println!(
            "Cargo-Rail Action release {} is {} for exact reconciliation",
            tag,
            if existing.is_some() {
                "consistent"
            } else {
                "absent and ready"
            }
        );
        return Ok(());
    }

    if existing.is_none() {
        create_immutable_tag(&repository, &tag, &intent.source_commit, &token)?;
        gh_success(
            &token,
            [
                "release",
                "create",
                tag.as_str(),
                "--repo",
                repository.as_str(),
                "--target",
                intent.source_commit.as_str(),
                "--draft",
                "--title",
                tag.as_str(),
                "--notes",
                &format!("Release intent: {}", intent.identity),
            ],
            "create draft Action release",
        )?;
    }
    let refreshed = inspect_release(&repository, &tag, &token)?
        .ok_or_else(|| ActionError::operational("draft release was not observable after creation"))?;
    validate_remote_release(&refreshed, &intent, true)?;
    for path in &arguments.asset {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| ActionError::rejected("release asset name is not UTF-8"))?;
        if !refreshed.assets.contains_key(name) {
            gh_success(
                &token,
                [
                    "release",
                    "upload",
                    tag.as_str(),
                    path_text(path)?,
                    "--repo",
                    repository.as_str(),
                ],
                "upload immutable Action release asset",
            )?;
        }
    }
    let complete = inspect_release(&repository, &tag, &token)?
        .ok_or_else(|| ActionError::operational("draft release disappeared during reconciliation"))?;
    validate_remote_release(&complete, &intent, false)?;
    if complete.draft {
        gh_success(
            &token,
            [
                "release",
                "edit",
                tag.as_str(),
                "--repo",
                repository.as_str(),
                "--draft=false",
            ],
            "publish immutable Action release",
        )?;
    }
    let published = inspect_release(&repository, &tag, &token)?
        .ok_or_else(|| ActionError::operational("Action release disappeared after publication"))?;
    validate_remote_release(&published, &intent, false)?;
    if published.draft || published.prerelease {
        return Err(ActionError::operational(
            "Action release did not reach the published non-prerelease state",
        ));
    }
    println!(
        "Cargo-Rail Action release {tag} published with intent {}",
        intent.identity
    );
    Ok(())
}

pub(crate) fn promote(arguments: &EffectArgs) -> Result<()> {
    let intent = load_intent(&arguments.intent)?;
    let local_assets = asset_records(&arguments.asset)?;
    if local_assets != intent.assets {
        return Err(ActionError::rejected(
            "release assets disagree with the durable release intent",
        ));
    }
    validate_local_release_authority(&intent)?;
    let repository = repository_name()?;
    let token = github_token()?;
    let immutable_tag = format!("v{}", intent.action_version);
    validate_exact_tag(&repository, &immutable_tag, &intent.source_commit, &token, false)?;
    let release = inspect_release(&repository, &immutable_tag, &token)?
        .ok_or_else(|| ActionError::rejected("immutable Action release is absent"))?;
    validate_remote_release(&release, &intent, false)?;
    if release.draft || release.prerelease {
        return Err(ActionError::rejected(
            "only a published, non-prerelease Action release can be promoted",
        ));
    }
    let current = inspect_ref(&repository, "v9", &token)?;
    if arguments.check {
        println!(
            "Cargo-Rail Action v9 promotion is {}",
            if current.as_deref() == Some(intent.source_commit.as_str()) {
                "already exact"
            } else {
                "ready"
            }
        );
        return Ok(());
    }
    if current.as_deref() != Some(intent.source_commit.as_str()) {
        if current.is_some() {
            gh_success(
                &token,
                [
                    "api",
                    "--method",
                    "PATCH",
                    &format!("repos/{repository}/git/refs/tags/v9"),
                    "-f",
                    &format!("sha={}", intent.source_commit),
                    "-F",
                    "force=true",
                ],
                "move mutable v9 tag",
            )?;
        } else {
            create_immutable_tag(&repository, "v9", &intent.source_commit, &token)?;
        }
    }
    validate_exact_tag(&repository, "v9", &intent.source_commit, &token, false)?;
    println!("Cargo-Rail Action v9 now selects {immutable_tag}");
    Ok(())
}

fn load_intent(path: &Path) -> Result<ReleaseIntent> {
    let bytes = read_bounded(path, MAX_INTENT_BYTES, "release intent")?;
    let value = parse_unique_json(&bytes, "release intent")?;
    let intent: ReleaseIntent = serde_json::from_value(value)
        .map_err(|error| ActionError::rejected(format!("release intent has an invalid shape: {error}")))?;
    validate_intent(&intent)?;
    Ok(intent)
}

fn validate_intent(intent: &ReleaseIntent) -> Result<()> {
    if intent.schema_version != INTENT_SCHEMA_VERSION
        || intent.action_version != VERSION
        || !valid_digest(&intent.runtime_manifest_sha256)
        || intent.identity != intent_identity(intent)?
    {
        return Err(ActionError::rejected(
            "release intent identity or version authority is invalid",
        ));
    }
    validate_source_commit(&intent.source_commit)?;
    crate::install::validate_cargo_rail_version(&intent.cargo_rail_version)?;
    if intent.assets.len() != 4 {
        return Err(ActionError::rejected(
            "release intent must bind exactly three runtimes and one manifest",
        ));
    }
    if !intent.assets.windows(2).all(|pair| pair[0].name < pair[1].name) {
        return Err(ActionError::rejected("release intent assets are not uniquely sorted"));
    }
    let expected_names = BTreeSet::from([
        "cargo-rail-action-aarch64-apple-darwin",
        "cargo-rail-action-runtime-v1.tsv",
        "cargo-rail-action-x86_64-pc-windows-msvc.exe",
        "cargo-rail-action-x86_64-unknown-linux-gnu",
    ]);
    let actual_names = intent
        .assets
        .iter()
        .map(|asset| asset.name.as_str())
        .collect::<BTreeSet<_>>();
    if actual_names != expected_names {
        return Err(ActionError::rejected(
            "release intent does not bind the exact v9.0 asset inventory",
        ));
    }
    for asset in &intent.assets {
        let maximum = if asset.name == "cargo-rail-action-runtime-v1.tsv" {
            MAX_MANIFEST_BYTES
        } else {
            MAX_RELEASE_RUNTIME_BYTES
        };
        if !valid_asset_name(&asset.name) || !valid_digest(&asset.sha256) || asset.bytes == 0 || asset.bytes > maximum {
            return Err(ActionError::rejected("release intent contains an invalid asset record"));
        }
        if asset.name == "cargo-rail-action-runtime-v1.tsv" && asset.sha256 != intent.runtime_manifest_sha256 {
            return Err(ActionError::rejected(
                "release intent manifest record disagrees with its manifest identity",
            ));
        }
    }
    Ok(())
}

fn intent_identity(intent: &ReleaseIntent) -> Result<String> {
    let portable = PortableIntent {
        schema_version: intent.schema_version,
        action_version: &intent.action_version,
        source_commit: &intent.source_commit,
        cargo_rail_version: &intent.cargo_rail_version,
        runtime_manifest_sha256: &intent.runtime_manifest_sha256,
        assets: &intent.assets,
    };
    let bytes = serde_json::to_vec(&portable)
        .map_err(|error| ActionError::operational(format!("cannot encode release intent identity: {error}")))?;
    Ok(format!(
        "cargo-rail-action-release-intent-v1:sha256:{}",
        digest_bytes(&bytes)
    ))
}

fn asset_records(paths: &[PathBuf]) -> Result<Vec<AssetRecord>> {
    let mut records = Vec::with_capacity(paths.len());
    for path in paths {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|name| valid_asset_name(name))
            .ok_or_else(|| ActionError::rejected("release asset must have one pathless UTF-8 name"))?;
        let maximum = if name == "cargo-rail-action-runtime-v1.tsv" {
            MAX_MANIFEST_BYTES
        } else {
            MAX_RUNTIME_BYTES
        };
        let bytes = read_bounded(path, maximum, "release asset")?;
        records.push(AssetRecord {
            name: name.to_string(),
            bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            sha256: digest_bytes(&bytes),
        });
    }
    records.sort_by(|left, right| left.name.cmp(&right.name));
    if records.windows(2).any(|pair| pair[0].name == pair[1].name) {
        return Err(ActionError::rejected("release asset names must be unique"));
    }
    Ok(records)
}

fn generate_runtime_manifest(assets: &[AssetRecord]) -> Result<Vec<u8>> {
    let by_name = assets
        .iter()
        .map(|asset| (asset.name.as_str(), asset))
        .collect::<BTreeMap<_, _>>();
    let targets = QUALIFIED_TARGETS;
    if by_name.len() != targets.len()
        || targets.iter().any(|target| !by_name.contains_key(runtime_name(target)))
        || assets
            .iter()
            .any(|asset| asset.bytes == 0 || asset.bytes > MAX_RELEASE_RUNTIME_BYTES)
    {
        return Err(ActionError::rejected(
            "runtime manifest generation requires the exact three bounded v9.0 executables",
        ));
    }
    let mut manifest = format!("cargo-rail-action-runtime-v1\t{VERSION}\n");
    for target in targets {
        let asset = by_name[runtime_name(target)];
        manifest.push_str(&format!(
            "{target}\t{}\t{}\t{}\n",
            asset.name, asset.bytes, asset.sha256
        ));
    }
    let bytes = manifest.into_bytes();
    validate_runtime_manifest(&bytes)?;
    Ok(bytes)
}

fn validate_runtime_manifest(bytes: &[u8]) -> Result<BTreeMap<String, (u64, String)>> {
    let text = std::str::from_utf8(bytes).map_err(|_| ActionError::rejected("runtime manifest is not UTF-8"))?;
    if !text.is_ascii() || text.contains('\r') || !text.ends_with('\n') {
        return Err(ActionError::rejected(
            "runtime manifest must be canonical ASCII LF-only text",
        ));
    }
    let mut lines = text.lines();
    if lines.next() != Some(&format!("cargo-rail-action-runtime-v1\t{VERSION}")) {
        return Err(ActionError::rejected("runtime manifest version header is invalid"));
    }
    let expected_targets = QUALIFIED_TARGETS;
    let mut rows = BTreeMap::new();
    let mut observed_targets = Vec::new();
    for line in lines {
        let mut fields = line.split('\t');
        let (Some(target), Some(name), Some(bytes), Some(digest)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(ActionError::rejected("runtime manifest contains an invalid row"));
        };
        if fields.next().is_some() {
            return Err(ActionError::rejected("runtime manifest contains an invalid row"));
        }
        if !expected_targets.contains(&target)
            || name != runtime_name(target)
            || bytes.is_empty()
            || !bytes.bytes().all(|byte| byte.is_ascii_digit())
            || (bytes.len() > 1 && bytes.starts_with('0'))
            || !valid_digest(digest)
        {
            return Err(ActionError::rejected("runtime manifest contains invalid authority"));
        }
        let size = bytes
            .parse::<u64>()
            .map_err(|_| ActionError::rejected("runtime manifest byte length is invalid"))?;
        if size == 0
            || size > MAX_RUNTIME_BYTES
            || rows.insert(target.to_string(), (size, digest.to_string())).is_some()
        {
            return Err(ActionError::rejected(
                "runtime manifest contains duplicate or out-of-bounds authority",
            ));
        }
        observed_targets.push(target);
    }
    if observed_targets != expected_targets {
        return Err(ActionError::rejected(
            "runtime manifest does not advertise exactly the sorted v9.0 targets",
        ));
    }
    Ok(rows)
}

fn validate_assets_against_manifest(
    assets: &[AssetRecord],
    rows: &BTreeMap<String, (u64, String)>,
    manifest_path: &Path,
) -> Result<()> {
    let manifest_name = manifest_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if manifest_name != "cargo-rail-action-runtime-v1.tsv" {
        return Err(ActionError::rejected("runtime manifest asset name is invalid"));
    }
    let by_name = assets
        .iter()
        .map(|asset| (asset.name.as_str(), asset))
        .collect::<BTreeMap<_, _>>();
    if by_name.len() != 4 || !by_name.contains_key(manifest_name) {
        return Err(ActionError::rejected(
            "release assets must contain the runtime manifest and three executables",
        ));
    }
    for (target, (bytes, digest)) in rows {
        let name = runtime_name(target);
        let record = by_name
            .get(name)
            .ok_or_else(|| ActionError::rejected(format!("runtime manifest asset {name} is missing")))?;
        if record.bytes != *bytes || record.sha256 != *digest {
            return Err(ActionError::rejected(format!(
                "runtime manifest authority disagrees with {name}"
            )));
        }
    }
    Ok(())
}

fn validate_local_release_authority(intent: &ReleaseIntent) -> Result<()> {
    let root = git_output(["rev-parse", "--show-toplevel"], "locate repository")?;
    let root = PathBuf::from(root);
    if env_string("GITHUB_REF")? != "refs/heads/main" {
        return Err(ActionError::rejected(
            "Action releases must be dispatched from the main branch",
        ));
    }
    if env_string("GITHUB_SHA")? != intent.source_commit {
        return Err(ActionError::rejected(
            "workflow source commit disagrees with release intent",
        ));
    }
    if git_output(["rev-parse", "HEAD"], "inspect source commit")? != intent.source_commit {
        return Err(ActionError::rejected(
            "release intent source commit is not current HEAD",
        ));
    }
    let status = git_output(
        ["status", "--porcelain=v1", "--untracked-files=all"],
        "inspect worktree",
    )?;
    if !status.is_empty() {
        return Err(ActionError::rejected("Action release requires a clean worktree"));
    }
    let origin_main = git_output(
        ["rev-parse", "refs/remotes/origin/main^{commit}"],
        "inspect origin/main",
    )?;
    if origin_main != intent.source_commit {
        return Err(ActionError::rejected(
            "release commit must already be present on origin/main",
        ));
    }
    if package_manifest_version(&root.join("Cargo.toml"))? != intent.action_version {
        return Err(ActionError::rejected(
            "Cargo.toml package version disagrees with release intent",
        ));
    }
    let metadata_version = metadata_cargo_rail_version(&root)?;
    if metadata_version != intent.cargo_rail_version {
        return Err(ActionError::rejected(
            "Action metadata Cargo-Rail defaults disagree with release intent",
        ));
    }
    verify_cargo_rail_release_assets(&intent.cargo_rail_version)?;
    Ok(())
}

fn metadata_cargo_rail_version(root: &Path) -> Result<String> {
    let mut defaults = Vec::new();
    for relative in ["action.yaml", "cache/action.yaml", "setup/action.yaml"] {
        let text = std::fs::read_to_string(root.join(relative)).map_err(|error| {
            ActionError::operational(format!(
                "cannot read {relative} while binding release defaults: {error}"
            ))
        })?;
        let mut in_inputs = false;
        let mut in_version = false;
        let mut version_blocks = 0usize;
        let mut selected = Vec::new();
        for line in text.lines() {
            if !line.starts_with(char::is_whitespace) {
                in_inputs = line == "inputs:";
                in_version = false;
                continue;
            }
            if !in_inputs {
                continue;
            }
            if line == "  version:" {
                version_blocks += 1;
                in_version = true;
                continue;
            }
            if in_version && line.starts_with("  ") && !line.starts_with("    ") {
                in_version = false;
            }
            if in_version && let Some(value) = line.trim().strip_prefix("default: ") {
                selected.push(
                    value
                        .strip_prefix('"')
                        .and_then(|value| value.strip_suffix('"'))
                        .ok_or_else(|| {
                            ActionError::rejected(format!(
                                "{relative} version default must be one double-quoted scalar"
                            ))
                        })?
                        .to_string(),
                );
            }
        }
        if version_blocks != 1 || selected.len() != 1 {
            return Err(ActionError::rejected(format!(
                "{relative} must have one version input and one default"
            )));
        }
        defaults.push(selected.remove(0));
    }
    if !defaults.windows(2).all(|pair| pair[0] == pair[1]) {
        return Err(ActionError::rejected(
            "planner, cache, and setup Cargo-Rail defaults disagree",
        ));
    }
    Ok(defaults.remove(0))
}

fn package_manifest_version(path: &Path) -> Result<String> {
    let text = std::fs::read_to_string(path)?;
    let mut in_package = false;
    let mut versions = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if in_package && let Some(value) = trimmed.strip_prefix("version = ") {
            versions.push(
                value
                    .strip_prefix('"')
                    .and_then(|value| value.strip_suffix('"'))
                    .ok_or_else(|| ActionError::rejected("Cargo.toml package version must be a quoted string"))?
                    .to_string(),
            );
        }
    }
    if versions.len() != 1 {
        return Err(ActionError::rejected(
            "Cargo.toml must contain one package version authority",
        ));
    }
    Ok(versions.remove(0))
}

fn verify_cargo_rail_release_assets(version: &str) -> Result<()> {
    let assets =
        std::iter::once("SHA256SUMS".to_string()).chain(QUALIFIED_TARGETS.map(crate::install::cargo_rail_archive_name));
    for asset in assets {
        let url = format!("https://github.com/loadingalias/cargo-rail/releases/download/v{version}/{asset}");
        let mut command = Command::new("curl");
        command.args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--head",
            "--proto",
            "=https",
            "--tlsv1.2",
            url.as_str(),
        ]);
        let output = run_bounded(&mut command, MAX_SUBPROCESS_BYTES, MAX_SUBPROCESS_BYTES)?;
        if !output.status.success() {
            return Err(ActionError::rejected(format!(
                "Cargo-Rail v{version} release asset {asset} is unavailable"
            )));
        }
    }
    Ok(())
}

fn inspect_release(repository: &str, tag: &str, token: &str) -> Result<Option<RemoteRelease>> {
    let endpoint = format!("repos/{repository}/releases/tags/{tag}");
    let mut command = Command::new("gh");
    command.args(["api", endpoint.as_str()]);
    let output = run_bounded_gh(&mut command, token, MAX_SUBPROCESS_BYTES, MAX_SUBPROCESS_BYTES)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("HTTP 404") || stderr.contains("Not Found") {
            return Ok(None);
        }
        return Err(subprocess_failure("inspect Action release", &output));
    }
    let value = parse_unique_json(&output.stdout, "GitHub release response")?;
    let object = value
        .as_object()
        .ok_or_else(|| ActionError::rejected("GitHub release response is not an object"))?;
    let draft = object
        .get("draft")
        .and_then(Value::as_bool)
        .ok_or_else(|| ActionError::rejected("release draft flag is absent"))?;
    let prerelease = object
        .get("prerelease")
        .and_then(Value::as_bool)
        .ok_or_else(|| ActionError::rejected("release prerelease flag is absent"))?;
    let target = object
        .get("target_commitish")
        .and_then(Value::as_str)
        .ok_or_else(|| ActionError::rejected("release target is absent"))?
        .to_string();
    let body = object
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let remote_assets = object
        .get("assets")
        .and_then(Value::as_array)
        .ok_or_else(|| ActionError::rejected("release assets are absent"))?;
    let mut assets = BTreeMap::new();
    for asset in remote_assets {
        let asset = asset
            .as_object()
            .ok_or_else(|| ActionError::rejected("release asset response is malformed"))?;
        let name = asset
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| ActionError::rejected("release asset name is absent"))?;
        let size = asset
            .get("size")
            .and_then(Value::as_u64)
            .ok_or_else(|| ActionError::rejected("release asset size is absent"))?;
        let digest = asset
            .get("digest")
            .and_then(Value::as_str)
            .and_then(|value| value.strip_prefix("sha256:"))
            .unwrap_or_default();
        if assets.insert(name.to_string(), (size, digest.to_string())).is_some() {
            return Err(ActionError::rejected("GitHub release contains duplicate asset names"));
        }
    }
    Ok(Some(RemoteRelease {
        draft,
        prerelease,
        target,
        body,
        assets,
    }))
}

fn validate_remote_release(release: &RemoteRelease, intent: &ReleaseIntent, allow_missing: bool) -> Result<()> {
    if release.prerelease
        || release.target != intent.source_commit
        || release.body.trim() != format!("Release intent: {}", intent.identity)
    {
        return Err(ActionError::rejected(
            "existing release metadata disagrees with release intent",
        ));
    }
    let expected = intent
        .assets
        .iter()
        .map(|asset| (asset.name.as_str(), (asset.bytes, asset.sha256.as_str())))
        .collect::<BTreeMap<_, _>>();
    for (name, (bytes, digest)) in &release.assets {
        let Some((expected_bytes, expected_digest)) = expected.get(name.as_str()) else {
            return Err(ActionError::rejected(format!(
                "existing release has unexpected asset {name}"
            )));
        };
        if bytes != expected_bytes || digest != expected_digest {
            return Err(ActionError::rejected(format!(
                "existing release asset {name} disagrees with release intent"
            )));
        }
    }
    if !allow_missing && release.assets.len() != expected.len() {
        return Err(ActionError::rejected("existing release is missing required assets"));
    }
    Ok(())
}

fn create_immutable_tag(repository: &str, tag: &str, commit: &str, token: &str) -> Result<()> {
    if let Some(existing) = inspect_ref(repository, tag, token)? {
        if existing == commit {
            return Ok(());
        }
        return Err(ActionError::rejected(format!(
            "tag {tag} already points to another commit"
        )));
    }
    gh_success(
        token,
        [
            "api",
            "--method",
            "POST",
            &format!("repos/{repository}/git/refs"),
            "-f",
            &format!("ref=refs/tags/{tag}"),
            "-f",
            &format!("sha={commit}"),
        ],
        "create Action release tag",
    )
}

fn validate_exact_tag(repository: &str, tag: &str, expected: &str, token: &str, allow_absent: bool) -> Result<()> {
    match inspect_ref(repository, tag, token)? {
        Some(actual) if actual == expected => Ok(()),
        Some(_) => Err(ActionError::rejected(format!("tag {tag} points to another commit"))),
        None if allow_absent => Ok(()),
        None => Err(ActionError::rejected(format!("tag {tag} is absent"))),
    }
}

fn inspect_ref(repository: &str, tag: &str, token: &str) -> Result<Option<String>> {
    let endpoint = format!("repos/{repository}/git/ref/tags/{tag}");
    let mut command = Command::new("gh");
    command.args(["api", endpoint.as_str()]);
    let output = run_bounded_gh(&mut command, token, MAX_SUBPROCESS_BYTES, MAX_SUBPROCESS_BYTES)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("HTTP 404") || stderr.contains("Not Found") {
            return Ok(None);
        }
        return Err(subprocess_failure("inspect Action tag", &output));
    }
    let response = parse_unique_json(&output.stdout, "GitHub tag response")?;
    let object = response
        .as_object()
        .ok_or_else(|| ActionError::rejected("GitHub tag response must be an object"))?;
    let target = object
        .get("object")
        .and_then(Value::as_object)
        .ok_or_else(|| ActionError::rejected("GitHub tag response has no target object"))?;
    if target.get("type").and_then(Value::as_str) != Some("commit") {
        return Err(ActionError::rejected(
            "GitHub tag response is not a lightweight commit tag",
        ));
    }
    let sha = target
        .get("sha")
        .and_then(Value::as_str)
        .ok_or_else(|| ActionError::rejected("GitHub tag response has no commit SHA"))?
        .to_string();
    validate_source_commit(&sha)?;
    Ok(Some(sha))
}

fn gh_success<I, S>(token: &str, arguments: I, subject: &str) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut command = Command::new("gh");
    command.args(arguments);
    let output = run_bounded_gh(&mut command, token, MAX_SUBPROCESS_BYTES, MAX_SUBPROCESS_BYTES)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(subprocess_failure(subject, &output))
    }
}

fn git_output<I, S>(arguments: I, subject: &str) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut command = Command::new("git");
    command.args(arguments);
    let output = run_bounded(&mut command, MAX_SUBPROCESS_BYTES, MAX_SUBPROCESS_BYTES)?;
    if !output.status.success() {
        return Err(subprocess_failure(subject, &output));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|_| ActionError::rejected(format!("{subject} returned non-UTF-8 output")))
}

fn github_token() -> Result<String> {
    let token = env_string("GH_TOKEN")?;
    if token.is_empty() || token.len() > 4096 || token.contains(['\r', '\n']) {
        return Err(ActionError::rejected("GH_TOKEN must be one bounded non-empty token"));
    }
    Ok(token)
}

fn repository_name() -> Result<String> {
    let repository = env_string("GITHUB_REPOSITORY")?;
    let parts = repository.split_once('/');
    if parts.is_none_or(|(owner, name)| {
        name.contains('/')
            || [owner, name].iter().any(|part| {
                part.is_empty()
                    || part.len() > 100
                    || !part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            })
    }) {
        return Err(ActionError::rejected("GITHUB_REPOSITORY is malformed"));
    }
    Ok(repository)
}

fn read_bounded(path: &Path, maximum: u64, subject: &str) -> Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| ActionError::operational(format!("cannot inspect {subject} '{}': {error}", path.display())))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > maximum {
        return Err(ActionError::rejected(format!(
            "{subject} must be a bounded regular non-symbolic file"
        )));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    File::open(path)?.take(maximum + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum {
        return Err(ActionError::rejected(format!("{subject} grew beyond its bound")));
    }
    Ok(bytes)
}

fn validate_source_commit(value: &str) -> Result<()> {
    if !(40..=64).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ActionError::rejected(
            "source commit must be one lowercase full Git object ID",
        ));
    }
    Ok(())
}

fn runtime_name(target: &str) -> &'static str {
    match target {
        "aarch64-apple-darwin" => "cargo-rail-action-aarch64-apple-darwin",
        "x86_64-pc-windows-msvc" => "cargo-rail-action-x86_64-pc-windows-msvc.exe",
        "x86_64-unknown-linux-gnu" => "cargo-rail-action-x86_64-unknown-linux-gnu",
        _ => "",
    }
}

fn valid_asset_name(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('.')
        && !value.contains(['/', '\\', '\r', '\n'])
        && value.as_bytes().iter().all(|byte| byte.is_ascii_graphic())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn digest_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn path_text(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| ActionError::rejected("release asset path is not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_intent() -> ReleaseIntent {
        let digest = "a".repeat(64);
        let mut intent = ReleaseIntent {
            schema_version: 1,
            identity: String::new(),
            action_version: VERSION.to_string(),
            source_commit: "b".repeat(40),
            cargo_rail_version: "0.26.0".to_string(),
            runtime_manifest_sha256: digest.clone(),
            assets: vec![
                AssetRecord {
                    name: "cargo-rail-action-aarch64-apple-darwin".to_string(),
                    bytes: 1,
                    sha256: "c".repeat(64),
                },
                AssetRecord {
                    name: "cargo-rail-action-runtime-v1.tsv".to_string(),
                    bytes: 1,
                    sha256: digest,
                },
                AssetRecord {
                    name: "cargo-rail-action-x86_64-pc-windows-msvc.exe".to_string(),
                    bytes: 1,
                    sha256: "d".repeat(64),
                },
                AssetRecord {
                    name: "cargo-rail-action-x86_64-unknown-linux-gnu".to_string(),
                    bytes: 1,
                    sha256: "e".repeat(64),
                },
            ],
        };
        intent.identity = intent_identity(&intent).expect("intent identity");
        intent
    }

    #[test]
    fn runtime_manifest_has_one_exact_row_per_target() {
        let bytes = format!(
            "cargo-rail-action-runtime-v1\t{VERSION}\n\
aarch64-apple-darwin\tcargo-rail-action-aarch64-apple-darwin\t1\t{}\n\
x86_64-pc-windows-msvc\tcargo-rail-action-x86_64-pc-windows-msvc.exe\t2\t{}\n\
x86_64-unknown-linux-gnu\tcargo-rail-action-x86_64-unknown-linux-gnu\t3\t{}\n",
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64)
        );
        assert!(validate_runtime_manifest(bytes.as_bytes()).is_ok());

        let mut lines = bytes.lines().collect::<Vec<_>>();
        lines.swap(1, 2);
        let unsorted = format!("{}\n", lines.join("\n"));
        assert!(validate_runtime_manifest(unsorted.as_bytes()).is_err());
    }

    #[test]
    fn runtime_manifest_generation_is_canonical_and_root_independent() {
        let assets = vec![
            AssetRecord {
                name: runtime_name("x86_64-unknown-linux-gnu").to_string(),
                bytes: 3,
                sha256: "c".repeat(64),
            },
            AssetRecord {
                name: runtime_name("aarch64-apple-darwin").to_string(),
                bytes: 1,
                sha256: "a".repeat(64),
            },
            AssetRecord {
                name: runtime_name("x86_64-pc-windows-msvc").to_string(),
                bytes: 2,
                sha256: "b".repeat(64),
            },
        ];
        let generated = generate_runtime_manifest(&assets).expect("generate manifest");
        let rows = validate_runtime_manifest(&generated).expect("validate generated manifest");
        assert_eq!(rows.len(), 3);
        assert!(generated.starts_with(format!("cargo-rail-action-runtime-v1\t{VERSION}\n").as_bytes()));
    }

    #[test]
    fn source_commit_rejects_abbreviations_and_uppercase() {
        assert!(validate_source_commit(&"a".repeat(40)).is_ok());
        assert!(validate_source_commit(&"a".repeat(39)).is_err());
        assert!(validate_source_commit(&"A".repeat(40)).is_err());
    }

    #[test]
    fn current_release_authorities_have_one_parseable_owner() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(!metadata_cargo_rail_version(root).expect("metadata default").is_empty());
        assert_eq!(
            package_manifest_version(&root.join("Cargo.toml")).expect("package version"),
            VERSION
        );
    }

    #[test]
    fn release_intent_rejects_wrong_inventory_and_release_size() {
        let intent = valid_intent();
        validate_intent(&intent).expect("valid intent");

        let mut wrong_name = intent.clone();
        wrong_name.assets[0].name = "unexpected".to_string();
        wrong_name.assets.sort_by(|left, right| left.name.cmp(&right.name));
        wrong_name.identity = intent_identity(&wrong_name).expect("wrong-name identity");
        assert!(validate_intent(&wrong_name).is_err());

        let mut oversized = intent;
        oversized.assets[0].bytes = MAX_RELEASE_RUNTIME_BYTES + 1;
        oversized.identity = intent_identity(&oversized).expect("oversized identity");
        assert!(validate_intent(&oversized).is_err());
    }

    #[test]
    fn draft_reconciliation_accepts_only_exact_partial_assets() {
        let intent = valid_intent();
        let first = &intent.assets[0];
        let partial = RemoteRelease {
            draft: true,
            prerelease: false,
            target: intent.source_commit.clone(),
            body: format!("Release intent: {}", intent.identity),
            assets: BTreeMap::from([(first.name.clone(), (first.bytes, first.sha256.clone()))]),
        };
        validate_remote_release(&partial, &intent, true).expect("retryable partial draft");
        assert!(validate_remote_release(&partial, &intent, false).is_err());
    }
}
