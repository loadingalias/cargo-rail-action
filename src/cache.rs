use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::github::{Publication, publish};
use crate::install::{self, ComponentSet};
use crate::plan::parse_unique_json;
use crate::repository::run_bounded;
use crate::{ActionError, Result, env_string, optional_env};

const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;
const MAX_PATH_BYTES: usize = 4 * 1024;
const MAX_URL_BYTES: usize = 4 * 1024;

#[derive(Debug)]
struct CacheInputs {
    remote: String,
    mode: String,
    max_size: String,
    max_bytes: u64,
    local_dir: Option<String>,
    root_portability: String,
    verify_remote: bool,
    version: String,
    workspace: PathBuf,
}

#[derive(Debug)]
struct RemoteState {
    provider: String,
    authority: String,
    mode: String,
    activation: String,
}

#[derive(Debug, Serialize)]
struct CacheStatusProjection<'a> {
    schema_version: u32,
    cargo_rail: &'a str,
    provider: &'a str,
    mode: &'a str,
    max_bytes: u64,
    root_portability: &'a str,
    remote_verification: &'a str,
}

pub(crate) fn run_action() -> Result<()> {
    let inputs = CacheInputs::load()?;
    let installed = install::install_cargo_rail(&inputs.version, ComponentSet::Cache)?;

    let mut setup = Command::new(installed.binary());
    setup
        .current_dir(&inputs.workspace)
        .args(["rail", "cache", "setup", "--remote"])
        .arg(&inputs.remote)
        .args([
            "--remote-mode",
            &inputs.mode,
            "--max-size",
            &inputs.max_size,
            "--root-portability",
            &inputs.root_portability,
        ]);
    if let Some(local_dir) = &inputs.local_dir {
        setup.arg("--local-dir").arg(local_dir);
    }
    setup.args(["-f", "json"]);
    let setup_output = run_bounded(&mut setup, MAX_DOCUMENT_BYTES, MAX_DOCUMENT_BYTES)?;
    if !setup_output.status.success() {
        return Err(cache_subprocess_failure("Cargo-Rail cache setup", &setup_output));
    }
    let setup_value = parse_unique_json(&setup_output.stdout, "Cargo-Rail cache setup")
        .map_err(|error| after_setup(error, "cache setup response validation"))?;
    let (setup_remote, setup_max_bytes) =
        validate_setup(&setup_value, &inputs).map_err(|error| after_setup(error, "cache setup response validation"))?;

    let mut status_command = Command::new(installed.binary());
    status_command
        .current_dir(&inputs.workspace)
        .args(["rail", "cache", "status", "--scope", "local", "-f", "json"]);
    let status_output = run_bounded(&mut status_command, MAX_DOCUMENT_BYTES, MAX_DOCUMENT_BYTES)
        .map_err(|error| after_setup(error, "cache status"))?;
    if !status_output.status.success() {
        return Err(after_setup(
            cache_subprocess_failure("Cargo-Rail cache status", &status_output),
            "cache status",
        ));
    }
    let status_value = parse_unique_json(&status_output.stdout, "Cargo-Rail cache status")
        .map_err(|error| after_setup(error, "cache status validation"))?;
    let (status_remote, max_bytes) =
        validate_status(&status_value, &inputs).map_err(|error| after_setup(error, "cache status validation"))?;
    require_remote_match(&setup_remote, &status_remote, "cache setup and status")
        .map_err(|error| after_setup(error, "cache authority correlation"))?;
    require(
        setup_max_bytes == max_bytes,
        "cache setup and status disagree on the local byte bound",
    )
    .map_err(|error| after_setup(error, "cache byte-bound correlation"))?;

    let remote_verification = if inputs.verify_remote {
        let mut probe_command = Command::new(installed.binary());
        probe_command
            .current_dir(&inputs.workspace)
            .args(["rail", "cache", "probe", "-f", "json"]);
        let probe_output = run_bounded(&mut probe_command, MAX_DOCUMENT_BYTES, MAX_DOCUMENT_BYTES)
            .map_err(|error| after_setup(error, "remote cache verification"))?;
        if !probe_output.status.success() {
            return Err(after_setup(
                cache_subprocess_failure("Cargo-Rail cache probe", &probe_output),
                "remote cache verification",
            ));
        }
        let probe_value = parse_unique_json(&probe_output.stdout, "Cargo-Rail cache probe")
            .map_err(|error| after_setup(error, "remote cache verification"))?;
        let probe_remote =
            validate_probe(&probe_value).map_err(|error| after_setup(error, "remote cache verification"))?;
        require_remote_match(&status_remote, &probe_remote, "cache status and probe")
            .map_err(|error| after_setup(error, "remote cache verification"))?;
        "verified"
    } else {
        "not_requested"
    };

    let projection = CacheStatusProjection {
        schema_version: 1,
        cargo_rail: &inputs.version,
        provider: &status_remote.provider,
        mode: &status_remote.mode,
        max_bytes,
        root_portability: &inputs.root_portability,
        remote_verification,
    };
    let compact = serde_json::to_string(&projection)
        .map_err(|error| ActionError::operational(format!("cannot encode cache action status: {error}")))
        .map_err(|error| after_setup(error, "cache result preparation"))?;
    crate::report::begin(installed.binary(), &inputs.workspace, &compact)
        .map_err(|error| after_setup(error, "cache recording initialization"))?;
    let runtime_directory =
        install::runtime_directory().map_err(|error| after_setup(error, "cache result preparation"))?;
    publish(Publication {
        summary: None,
        paths: vec![runtime_directory, installed.directory().to_path_buf()],
        outputs: vec![
            ("version".to_string(), inputs.version.clone()),
            ("status".to_string(), compact),
        ],
    })
    .map_err(|error| after_setup(error, "cache result publication"))?;
    println!(
        "Cargo-Rail cache ready: {}, {}, {} local, remote {}",
        provider_display(&status_remote.provider),
        if status_remote.mode == "read" {
            "read-only"
        } else {
            "read-write"
        },
        human_bytes(max_bytes),
        if remote_verification == "verified" {
            "verified"
        } else {
            "not checked"
        }
    );
    Ok(())
}

impl CacheInputs {
    fn load() -> Result<Self> {
        let remote = env_string("INPUT_REMOTE")?;
        if remote.is_empty()
            || remote.len() > MAX_URL_BYTES
            || remote.contains(['\r', '\n'])
            || remote.as_bytes().contains(&0)
        {
            return Err(ActionError::rejected(
                "remote must be one non-empty URL no longer than 4 KiB",
            ));
        }
        let mode = env_string("INPUT_MODE")?;
        if !matches!(mode.as_str(), "read" | "read-write") {
            return Err(ActionError::rejected(
                "mode must be explicitly set to read or read-write",
            ));
        }
        let max_size = env_string("INPUT_MAX_SIZE")?;
        let max_bytes = parse_cache_size(&max_size)?;
        let local_dir = optional_env("INPUT_LOCAL_DIR")?;
        if local_dir.len() > MAX_PATH_BYTES || local_dir.contains(['\r', '\n']) || local_dir.as_bytes().contains(&0) {
            return Err(ActionError::rejected("local-dir must be one path no longer than 4 KiB"));
        }
        let root_portability = env_string("INPUT_ROOT_PORTABILITY")?;
        if !matches!(root_portability.as_str(), "physical" | "remap") {
            return Err(ActionError::rejected("root-portability must be physical or remap"));
        }
        let verify_remote = match env_string("INPUT_VERIFY_REMOTE")?.as_str() {
            "true" => true,
            "false" => false,
            _ => return Err(ActionError::rejected("verify-remote must be true or false")),
        };
        let version = env_string("INPUT_VERSION")?;
        install::validate_cargo_rail_version(&version)?;
        let checkout = canonical_directory(&PathBuf::from(env_string("GITHUB_WORKSPACE")?), "GITHUB_WORKSPACE")?;
        let working = env_string("INPUT_WORKING_DIRECTORY")?;
        if working.is_empty() || working.len() > MAX_PATH_BYTES || working.contains(['\r', '\n']) {
            return Err(ActionError::rejected(
                "working-directory must be one path no longer than 4 KiB",
            ));
        }
        let workspace = canonical_directory(&checkout.join(working), "working-directory")?;
        if !workspace.starts_with(&checkout) || !workspace.join("Cargo.toml").is_file() {
            return Err(ActionError::rejected(
                "working-directory must remain in the canonical checkout and contain Cargo.toml",
            ));
        }
        Ok(Self {
            remote,
            mode,
            max_size,
            max_bytes,
            local_dir: (!local_dir.is_empty()).then_some(local_dir),
            root_portability,
            verify_remote,
            version,
            workspace,
        })
    }
}

fn validate_setup(value: &Value, inputs: &CacheInputs) -> Result<(RemoteState, u64)> {
    let object = machine_envelope(value, "cache", "setup", "success", "cache setup")?;
    exact_keys(
        object,
        &[
            "schema_version",
            "command",
            "mode",
            "result",
            "exit_code",
            "changed",
            "config_path",
            "config_field",
            "config_action",
            "wrapper_path",
            "receipt_path",
            "private_state_action",
            "profile_id",
            "cache_base",
            "max_bytes",
            "remote",
            "root_portability",
            "distributed",
            "distributed_policy",
            "pending",
        ],
        &[],
        "cache setup envelope",
    )?;
    require(
        object["pending"] == false,
        "cache setup reported pending changes after apply",
    )?;
    require(object["changed"].is_boolean(), "cache setup changed flag is invalid")?;
    for field in [
        "config_path",
        "config_action",
        "wrapper_path",
        "receipt_path",
        "private_state_action",
        "profile_id",
        "cache_base",
    ] {
        string_field(object, field, "cache setup")?;
    }
    require(
        object["config_field"] == "build.rustc-wrapper",
        "cache setup config field is invalid",
    )?;
    require_optional_enum(
        object.get("distributed"),
        &["local_process_qualification_v1", "mutual_tls_direct_v1"],
        "cache setup distributed mode",
    )?;
    require_optional_enum(
        object.get("distributed_policy"),
        &["automatic", "qualification"],
        "cache setup distributed policy",
    )?;
    require(
        object["root_portability"] == inputs.root_portability,
        "cache setup root portability disagrees with input",
    )?;
    let remote = remote_state(object.get("remote"), "cache setup remote")?;
    require(
        remote.mode == inputs.mode,
        "cache setup remote mode disagrees with input",
    )?;
    require(
        remote.activation == "direct_transport_selected",
        "Cargo-Rail remote cache transport is not active",
    )?;
    let max_bytes = object["max_bytes"]
        .as_u64()
        .filter(|value| *value > 0)
        .ok_or_else(|| ActionError::rejected("cache setup max_bytes is missing or invalid"))?;
    require(
        max_bytes == inputs.max_bytes,
        "cache setup byte bound disagrees with input",
    )?;
    Ok((remote, max_bytes))
}

fn validate_status(value: &Value, inputs: &CacheInputs) -> Result<(RemoteState, u64)> {
    validate_status_contract(value, &inputs.mode, &inputs.root_portability, inputs.max_bytes)
}

pub(crate) fn validate_report_status(value: &Value, mode: &str, root_portability: &str, max_bytes: u64) -> Result<()> {
    validate_status_contract(value, mode, root_portability, max_bytes).map(|_| ())
}

fn validate_status_contract(
    value: &Value,
    mode: &str,
    root_portability: &str,
    expected_max_bytes: u64,
) -> Result<(RemoteState, u64)> {
    let object = machine_envelope(value, "cache", "status", "success", "cache status")?;
    exact_keys(
        object,
        &[
            "schema_version",
            "command",
            "mode",
            "result",
            "exit_code",
            "scope",
            "status",
        ],
        &[],
        "cache status envelope",
    )?;
    require(object["scope"] == "local", "cache status scope is not local")?;
    let status = object_field(object, "status", "cache status")?;
    exact_keys(
        status,
        &["schema_version", "installation", "local", "remote"],
        &[],
        "cache status",
    )?;
    require(status["schema_version"] == 16, "cache status schema_version must be 16")?;
    let installation = object_field(status, "installation", "cache status")?;
    exact_keys(
        installation,
        &[
            "state",
            "healthy",
            "cargo_home",
            "config_path",
            "selection_source",
            "cargo_l0",
            "usage",
            "issues",
        ],
        &[
            "wrapper_path",
            "profile_id",
            "bound_workspace_root",
            "trust_domain",
            "cache_base",
            "max_bytes",
            "root_portability",
            "distributed",
            "distributed_policy",
            "distributed_placement_history",
        ],
        "cache status installation",
    )?;
    require(
        installation["state"] == "installed",
        "Cargo-Rail cache integration is not installed",
    )?;
    require(
        installation["healthy"] == true,
        "Cargo-Rail cache integration is unhealthy",
    )?;
    for field in [
        "cargo_home",
        "config_path",
        "wrapper_path",
        "profile_id",
        "bound_workspace_root",
        "trust_domain",
        "selection_source",
        "cache_base",
        "cargo_l0",
    ] {
        string_field(installation, field, "cache status installation")?;
    }
    require(
        installation.get("root_portability").and_then(Value::as_str) == Some(root_portability),
        "cache status root portability disagrees with input",
    )?;
    let max_bytes = installation
        .get("max_bytes")
        .and_then(Value::as_u64)
        .filter(|bytes| *bytes > 0)
        .ok_or_else(|| ActionError::rejected("cache status max_bytes is missing or invalid"))?;
    validate_usage(object_field(installation, "usage", "cache status installation")?)?;
    require(
        installation["issues"]
            .as_array()
            .is_some_and(|issues| issues.iter().all(Value::is_string)),
        "cache status issues must be an array of strings",
    )?;
    require_optional_enum(
        installation.get("distributed"),
        &["local_process_qualification_v1", "mutual_tls_direct_v1"],
        "cache status installation.distributed",
    )?;
    require_optional_enum(
        installation.get("distributed_policy"),
        &["automatic", "qualification"],
        "cache status installation.distributed_policy",
    )?;
    require_optional_object(
        installation.get("distributed_placement_history"),
        validate_placement_history,
        "cache status distributed placement history",
    )?;
    validate_local(object_field(status, "local", "cache status")?)?;
    let remote = remote_state(status.get("remote"), "cache status remote")?;
    require(remote.mode == mode, "cache status remote mode disagrees with input")?;
    require(
        remote.activation == "direct_transport_selected",
        "Cargo-Rail remote cache transport is not active",
    )?;
    require(
        max_bytes == expected_max_bytes,
        "cache status byte bound disagrees with input",
    )?;
    Ok((remote, max_bytes))
}

fn validate_probe(value: &Value) -> Result<RemoteState> {
    let object = machine_envelope(value, "cache", "probe", "ready", "cache probe")?;
    exact_keys(
        object,
        &[
            "schema_version",
            "command",
            "mode",
            "result",
            "exit_code",
            "ready",
            "protocol_marker",
            "remote",
        ],
        &[],
        "cache probe envelope",
    )?;
    require(object["ready"] == true, "cache probe is not ready")?;
    require(
        matches!(object["protocol_marker"].as_str(), Some("existing" | "initialized")),
        "cache probe protocol marker is invalid",
    )?;
    remote_state(object.get("remote"), "cache probe remote")
}

fn machine_envelope<'a>(
    value: &'a Value,
    command: &str,
    mode: &str,
    result: &str,
    subject: &str,
) -> Result<&'a Map<String, Value>> {
    let object = value
        .as_object()
        .ok_or_else(|| ActionError::rejected(format!("{subject} must be an object")))?;
    for (field, expected) in [
        ("schema_version", Value::from(1)),
        ("command", Value::from(command)),
        ("mode", Value::from(mode)),
        ("result", Value::from(result)),
        ("exit_code", Value::from(0)),
    ] {
        require(
            object.get(field) == Some(&expected),
            format!("{subject}.{field} is invalid"),
        )?;
    }
    Ok(object)
}

fn remote_state(value: Option<&Value>, subject: &str) -> Result<RemoteState> {
    let remote = value
        .and_then(Value::as_object)
        .ok_or_else(|| ActionError::rejected(format!("{subject} is missing or invalid")))?;
    exact_keys(
        remote,
        &[
            "provider",
            "protocol",
            "authority",
            "mode",
            "shared_environment_names",
            "activation",
            "selection_source",
        ],
        &[],
        subject,
    )?;
    let provider = string_field(remote, "provider", subject)?;
    require(
        matches!(provider, "aws-s3" | "azure-blob" | "cloudflare-r2" | "s3-compatible"),
        format!("{subject}.provider is unsupported"),
    )?;
    let authority = string_field(remote, "authority", subject)?;
    require(valid_authority(authority), format!("{subject}.authority is malformed"))?;
    let mode = string_field(remote, "mode", subject)?;
    require(
        matches!(mode, "read" | "read-write"),
        format!("{subject}.mode is invalid"),
    )?;
    let activation = string_field(remote, "activation", subject)?;
    require(
        remote["shared_environment_names"].as_u64().is_some(),
        "remote shared_environment_names is invalid",
    )?;
    require(
        string_field(remote, "protocol", subject)? == "native-v6",
        format!("{subject}.protocol is incompatible"),
    )?;
    string_field(remote, "selection_source", subject)?;
    Ok(RemoteState {
        provider: provider.to_string(),
        authority: authority.to_string(),
        mode: mode.to_string(),
        activation: activation.to_string(),
    })
}

fn require_remote_match(left: &RemoteState, right: &RemoteState, subject: &str) -> Result<()> {
    require(
        left.provider == right.provider
            && left.authority == right.authority
            && left.mode == right.mode
            && left.activation == right.activation,
        format!("{subject} disagree on provider, authority, mode, or activation"),
    )
}

fn validate_usage(usage: &Map<String, Value>) -> Result<()> {
    exact_keys(
        usage,
        &[
            "recorded_events",
            "hits",
            "misses",
            "bypasses",
            "failures",
            "ledger_full",
            "early_bypasses",
            "early_bypass_reasons",
            "early_bypass_ledger_full",
            "early_bypass_incomplete_tail",
            "failure_reason_counts_available",
            "failure_reasons",
        ],
        &[],
        "cache status usage",
    )?;
    for field in [
        "recorded_events",
        "hits",
        "misses",
        "bypasses",
        "failures",
        "early_bypasses",
    ] {
        require(
            usage[field].as_u64().is_some(),
            format!("cache status usage.{field} is invalid"),
        )?;
    }
    for field in [
        "ledger_full",
        "early_bypass_ledger_full",
        "early_bypass_incomplete_tail",
        "failure_reason_counts_available",
    ] {
        require(
            usage[field].is_boolean(),
            format!("cache status usage.{field} is invalid"),
        )?;
    }
    for field in ["early_bypass_reasons", "failure_reasons"] {
        let counters = usage[field]
            .as_object()
            .ok_or_else(|| ActionError::rejected(format!("cache status usage.{field} is invalid")))?;
        require(
            counters.values().all(|value| value.as_u64().is_some()),
            "cache status counter is invalid",
        )?;
    }
    Ok(())
}

fn validate_local(local: &Map<String, Value>) -> Result<()> {
    exact_keys(local, &["present", "profile_scoped"], &["cache"], "cache status local")?;
    require(
        local["present"].is_boolean() && local["profile_scoped"].is_boolean(),
        "cache status local flags are invalid",
    )?;
    if let Some(cache) = local.get("cache") {
        let cache = cache
            .as_object()
            .ok_or_else(|| ActionError::rejected("cache status local.cache is invalid"))?;
        exact_keys(
            cache,
            &[
                "root",
                "trust_domain",
                "bytes",
                "max_bytes",
                "committed_result_bytes",
                "results",
                "pins",
                "native_actions",
                "native_unique",
                "native_conflicted",
                "native_quarantined",
                "native_local_origins",
                "native_remote_origins",
                "native_ledger_bytes",
                "native_ledger_max_bytes",
                "native_ledger_disabled",
                "objects",
                "active_leases",
                "stale_leases",
                "native_restore_lock_files",
                "staging_entries",
                "staging_bytes",
                "index_files",
                "reclaimable_bytes",
            ],
            &["oldest_used_unix_ms", "newest_used_unix_ms"],
            "cache status local CAS",
        )?;
        for field in ["root", "trust_domain"] {
            string_field(cache, field, "cache status local CAS")?;
        }
        for field in [
            "bytes",
            "max_bytes",
            "committed_result_bytes",
            "results",
            "pins",
            "native_actions",
            "native_unique",
            "native_conflicted",
            "native_quarantined",
            "native_local_origins",
            "native_remote_origins",
            "native_ledger_bytes",
            "native_ledger_max_bytes",
            "objects",
            "active_leases",
            "stale_leases",
            "native_restore_lock_files",
            "staging_entries",
            "staging_bytes",
            "index_files",
            "reclaimable_bytes",
        ] {
            require(
                cache[field].as_u64().is_some(),
                format!("cache status local CAS.{field} is invalid"),
            )?;
        }
        require(
            cache["native_ledger_disabled"].is_boolean(),
            "cache status local CAS.native_ledger_disabled is invalid",
        )?;
        for field in ["oldest_used_unix_ms", "newest_used_unix_ms"] {
            require_optional_u64(cache.get(field), &format!("cache status local CAS.{field}"))?;
        }
    }
    Ok(())
}

fn validate_placement_history(value: &Map<String, Value>) -> Result<()> {
    exact_keys(
        value,
        &[
            "state",
            "classes",
            "local_classes",
            "local_observations",
            "remote_classes",
            "remote_observations",
            "remote_failures",
            "active_backoffs",
        ],
        &["newest_observation_unix_secs"],
        "cache status distributed placement history",
    )?;
    require(
        matches!(value["state"].as_str(), Some("empty" | "ready")),
        "cache status placement history state is invalid",
    )?;
    for field in [
        "classes",
        "local_classes",
        "local_observations",
        "remote_classes",
        "remote_observations",
        "remote_failures",
        "active_backoffs",
    ] {
        require(
            value[field].as_u64().is_some(),
            format!("cache status placement history.{field} is invalid"),
        )?;
    }
    require_optional_u64(
        value.get("newest_observation_unix_secs"),
        "cache status placement history.newest_observation_unix_secs",
    )
}

fn require_optional_enum(value: Option<&Value>, allowed: &[&str], subject: &str) -> Result<()> {
    require(
        value.is_none_or(|value| value.is_null() || value.as_str().is_some_and(|value| allowed.contains(&value))),
        format!("{subject} is invalid"),
    )
}

fn require_optional_u64(value: Option<&Value>, subject: &str) -> Result<()> {
    require(
        value.is_none_or(|value| value.is_null() || value.as_u64().is_some()),
        format!("{subject} is invalid"),
    )
}

fn require_optional_object(
    value: Option<&Value>,
    validate: fn(&Map<String, Value>) -> Result<()>,
    subject: &str,
) -> Result<()> {
    match value {
        None | Some(Value::Null) => Ok(()),
        Some(Value::Object(value)) => validate(value),
        Some(_) => Err(ActionError::rejected(format!("{subject} is invalid"))),
    }
}

fn provider_display(provider: &str) -> &'static str {
    match provider {
        "aws-s3" => "AWS S3",
        "azure-blob" => "Azure Blob Storage",
        "cloudflare-r2" => "Cloudflare R2",
        "s3-compatible" => "S3-compatible",
        _ => "Unsupported provider",
    }
}

fn human_bytes(value: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    const TIB: u64 = GIB * 1024;
    for (unit, bytes) in [("TiB", TIB), ("GiB", GIB), ("MiB", MIB), ("KiB", KIB)] {
        if value >= bytes && value.is_multiple_of(bytes) {
            return format!("{} {unit}", value / bytes);
        }
    }
    format!("{value} B")
}

fn exact_keys(object: &Map<String, Value>, required: &[&str], optional: &[&str], subject: &str) -> Result<()> {
    let required = required.iter().copied().collect::<BTreeSet<_>>();
    let optional = optional.iter().copied().collect::<BTreeSet<_>>();
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let missing = required.difference(&actual).copied().collect::<Vec<_>>();
    let unknown = actual
        .difference(&required)
        .filter(|field| !optional.contains(**field))
        .copied()
        .collect::<Vec<_>>();
    require(missing.is_empty(), format!("{subject} is missing {missing:?}"))?;
    require(unknown.is_empty(), format!("{subject} has unknown fields {unknown:?}"))
}

fn object_field<'a>(object: &'a Map<String, Value>, field: &str, subject: &str) -> Result<&'a Map<String, Value>> {
    object
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| ActionError::rejected(format!("{subject}.{field} is missing or invalid")))
}

fn string_field<'a>(object: &'a Map<String, Value>, field: &str, subject: &str) -> Result<&'a str> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ActionError::rejected(format!("{subject}.{field} is missing or invalid")))
}

fn valid_authority(value: &str) -> bool {
    value.strip_prefix("remote-authority-v1-sha256-").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

fn require(condition: bool, message: impl Into<String>) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(ActionError::rejected(message))
    }
}

fn canonical_directory(path: &Path, subject: &str) -> Result<PathBuf> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| ActionError::rejected(format!("cannot resolve {subject} '{}': {error}", path.display())))?;
    if !canonical.is_dir() {
        return Err(ActionError::rejected(format!(
            "{subject} must name an existing directory"
        )));
    }
    Ok(canonical)
}

fn parse_cache_size(value: &str) -> Result<u64> {
    if value.len() > 64 || value.contains(['\r', '\n']) {
        return Err(ActionError::rejected("max-size must be one bounded value"));
    }
    let split = value
        .find(|character: char| !character.is_ascii_digit())
        .ok_or_else(|| ActionError::rejected("max-size requires B, KiB, MiB, GiB, or TiB"))?;
    let (number, unit) = value.split_at(split);
    if number.is_empty() || number.starts_with('0') && number != "0" {
        return Err(ActionError::rejected(
            "max-size must contain a canonical positive integer",
        ));
    }
    let number = number
        .parse::<u64>()
        .map_err(|_| ActionError::rejected("max-size integer is invalid or overflowing"))?;
    let multiplier = match unit {
        "B" => 1_u64,
        "KiB" => 1024,
        "MiB" => 1024 * 1024,
        "GiB" => 1024 * 1024 * 1024,
        "TiB" => 1024 * 1024 * 1024 * 1024,
        _ => return Err(ActionError::rejected("max-size unit must be B, KiB, MiB, GiB, or TiB")),
    };
    number
        .checked_mul(multiplier)
        .filter(|bytes| *bytes > 0 && *bytes <= usize::MAX as u64)
        .ok_or_else(|| ActionError::rejected("max-size must be positive and fit this platform"))
}

fn after_setup(error: ActionError, operation: &str) -> ActionError {
    ActionError::operational(format!(
        "{operation} failed after cache setup: {error}; the verified local installation remains and rerunning the action is idempotent"
    ))
}

fn cache_subprocess_failure(subject: &str, output: &crate::repository::BoundedOutput) -> ActionError {
    crate::repository::subprocess_failure(subject, output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(mode: &str, portability: &str) -> CacheInputs {
        CacheInputs {
            remote: "s3://example/cache".to_string(),
            mode: mode.to_string(),
            max_size: "10GiB".to_string(),
            max_bytes: 10 * 1024 * 1024 * 1024,
            local_dir: None,
            root_portability: portability.to_string(),
            verify_remote: false,
            version: "0.26.0".to_string(),
            workspace: PathBuf::from("/workspace"),
        }
    }

    #[test]
    #[ignore = "requires the current Cargo-Rail source binary and authenticated sibling components"]
    fn source_cache_setup_and_status_match_the_action_contract() {
        let binary = PathBuf::from(std::env::var_os("CARGO_RAIL_TEST_BINARY").expect("source binary"));
        let root = crate::repository::create_private_directory(&std::env::temp_dir(), "rail-source-cache")
            .expect("isolated fixture");
        let workspace = root.join("workspace");
        let cargo_home = root.join("cargo-home");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&cargo_home).unwrap();
        std::fs::write(
            workspace.join("Cargo.toml"),
            "[package]\nname = 'source-contract'\nversion = '0.1.0'\nedition = '2024'\n[lib]\npath = 'lib.rs'\n",
        )
        .unwrap();
        std::fs::write(workspace.join("lib.rs"), "pub fn value() -> u8 { 7 }\n").unwrap();
        let mut inputs = inputs("read", "physical");
        inputs.workspace = workspace.clone();
        inputs.remote = "s3://source-contract/cache?region=us-east-1&owner=123456789012".into();
        let run = |arguments: &[&str]| {
            let output = Command::new(&binary)
                .current_dir(&workspace)
                .env("CARGO_HOME", &cargo_home)
                .args(arguments)
                .output()
                .expect("source command");
            assert!(output.status.success(), "{output:?}");
            parse_unique_json(&output.stdout, "source output").expect("one JSON value")
        };
        let setup = run(&[
            "rail",
            "cache",
            "setup",
            "--remote",
            &inputs.remote,
            "--remote-mode",
            &inputs.mode,
            "--max-size",
            &inputs.max_size,
            "--root-portability",
            &inputs.root_portability,
            "-f",
            "json",
        ]);
        let (setup_remote, setup_bytes) = validate_setup(&setup, &inputs).expect("source setup contract");
        let status = run(&["rail", "cache", "status", "--scope", "local", "-f", "json"]);
        let (status_remote, status_bytes) = validate_status(&status, &inputs).expect("source status contract");
        require_remote_match(&setup_remote, &status_remote, "source setup/status").expect("same authority");
        assert_eq!(setup_bytes, inputs.max_bytes);
        assert_eq!(status_bytes, inputs.max_bytes);
        run(&["rail", "cache", "uninstall", "-f", "json"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn remote(provider: &str, mode: &str) -> Value {
        serde_json::json!({
            "provider": provider,
            "protocol": "native-v6",
            "authority": format!("remote-authority-v1-sha256-{}", "a".repeat(64)),
            "mode": mode,
            "shared_environment_names": 0,
            "activation": "direct_transport_selected",
            "selection_source": "installed_profile",
        })
    }

    fn setup(remote: Value, portability: &str) -> Value {
        serde_json::json!({
            "schema_version": 1,
            "command": "cache",
            "mode": "setup",
            "result": "success",
            "exit_code": 0,
            "changed": true,
            "config_path": "/cargo/config.toml",
            "config_field": "build.rustc-wrapper",
            "config_action": "create",
            "wrapper_path": "/cargo/wrapper",
            "receipt_path": "/cargo/receipt",
            "private_state_action": "install_or_repair",
            "profile_id": "profile",
            "cache_base": "/cargo",
            "max_bytes": 10_737_418_240_u64,
            "remote": remote,
            "root_portability": portability,
            "distributed": null,
            "distributed_policy": null,
            "pending": false,
        })
    }

    fn status(remote: Value, portability: &str) -> Value {
        serde_json::json!({
            "schema_version": 1,
            "command": "cache",
            "mode": "status",
            "result": "success",
            "exit_code": 0,
            "scope": "local",
            "status": {
                "schema_version": 16,
                "installation": {
                    "state": "installed",
                    "healthy": true,
                    "cargo_home": "/cargo",
                    "config_path": "/cargo/config.toml",
                    "wrapper_path": "/cargo/wrapper",
                    "profile_id": "profile",
                    "bound_workspace_root": "/workspace",
                    "trust_domain": "trust",
                    "selection_source": "installed_profile",
                    "cache_base": "/cargo",
                    "max_bytes": 10_737_418_240_u64,
                    "root_portability": portability,
                    "cargo_l0": "owned_by_cargo_not_observable_when_rustc_is_not_launched",
                    "usage": {
                        "recorded_events": 0,
                        "hits": 0,
                        "misses": 0,
                        "bypasses": 0,
                        "failures": 0,
                        "ledger_full": false,
                        "early_bypasses": 0,
                        "early_bypass_reasons": {},
                        "early_bypass_ledger_full": false,
                        "early_bypass_incomplete_tail": false,
                        "failure_reason_counts_available": true,
                        "failure_reasons": {},
                    },
                    "issues": [],
                },
                "local": {"present": false, "profile_scoped": true},
                "remote": remote,
            },
        })
    }

    fn probe(remote: Value, marker: &str) -> Value {
        serde_json::json!({
            "schema_version": 1,
            "command": "cache",
            "mode": "probe",
            "result": "ready",
            "exit_code": 0,
            "ready": true,
            "protocol_marker": marker,
            "remote": remote,
        })
    }

    #[test]
    fn public_status_is_flat_and_redacted() {
        let projection = CacheStatusProjection {
            schema_version: 1,
            cargo_rail: "0.26.0",
            provider: "aws-s3",
            mode: "read",
            max_bytes: 10 * 1024 * 1024 * 1024,
            root_portability: "physical",
            remote_verification: "not_requested",
        };
        let value = serde_json::to_value(&projection).expect("projection");
        assert_eq!(value.as_object().expect("object").len(), 7);
        assert!(value.get("authority").is_none());
        assert!(value.get("remote").is_none());
        assert!(value.get("local_dir").is_none());
    }

    #[test]
    fn human_sizes_are_exact() {
        assert_eq!(human_bytes(10 * 1024 * 1024 * 1024), "10 GiB");
        assert_eq!(human_bytes(12), "12 B");
    }

    #[test]
    fn cache_sizes_match_cargo_rail_contract() {
        assert_eq!(parse_cache_size("10GiB").expect("size"), 10 * 1024 * 1024 * 1024);
        for invalid in ["", "10", "01GiB", "0B", "1.5GiB", "1GB", "-1GiB"] {
            assert!(parse_cache_size(invalid).is_err(), "accepted {invalid}");
        }
    }

    #[test]
    fn setup_status_and_probe_bind_the_same_redacted_authority() {
        for provider in ["aws-s3", "azure-blob", "cloudflare-r2", "s3-compatible"] {
            let inputs = inputs("read", "remap");
            let setup_remote = remote(provider, "read");
            let status_remote = remote(provider, "read");
            let probe_remote = remote(provider, "read");
            let (from_setup, setup_bytes) =
                validate_setup(&setup(setup_remote, "remap"), &inputs).expect("setup contract");
            let (from_status, status_bytes) =
                validate_status(&status(status_remote, "remap"), &inputs).expect("status contract");
            let from_probe = validate_probe(&probe(probe_remote, "initialized")).expect("probe contract");
            require_remote_match(&from_setup, &from_status, "setup/status").expect("setup/status authority");
            require_remote_match(&from_status, &from_probe, "status/probe").expect("status/probe authority");
            assert_eq!(setup_bytes, status_bytes);
        }
    }

    #[test]
    fn cache_contract_rejects_schema_authority_and_probe_drift() {
        let inputs = inputs("read", "physical");
        let mut wrong_schema = status(remote("aws-s3", "read"), "physical");
        wrong_schema["status"]["schema_version"] = Value::from(15);
        assert!(validate_status(&wrong_schema, &inputs).is_err());

        let mut retired_field = status(remote("aws-s3", "read"), "physical");
        retired_field["status"]["installation"]["unbound_pre_profile_state"] = serde_json::json!({});
        assert!(validate_status(&retired_field, &inputs).is_err());

        let status_remote = remote_state(Some(&remote("aws-s3", "read")), "status remote").expect("status remote");
        let probe_remote = validate_probe(&probe(remote("azure-blob", "read"), "existing")).expect("probe contract");
        assert!(require_remote_match(&status_remote, &probe_remote, "status/probe").is_err());

        let mut leaked = remote("aws-s3", "read");
        leaked["url"] = Value::String("s3://secret".to_string());
        assert!(remote_state(Some(&leaked), "remote").is_err());
    }
}
