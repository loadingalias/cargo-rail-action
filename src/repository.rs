use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use serde_json::Value;

use crate::github::{Publication, publish};
use crate::install::{self, ComponentSet};
use crate::plan::{MAX_PLAN_BYTES, ValidatedPlan, parse_unique_json};
use crate::{ActionError, Result, env_string, optional_env};

const MAX_EVENT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 4 * 1024;
const MAX_TOKEN_BYTES: usize = 4 * 1024;
const MAX_SUBPROCESS_BYTES: usize = 1024 * 1024;

static PRIVATE_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub(crate) struct BoundedOutput {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

#[derive(Debug)]
struct PlannerInputs {
    version: String,
    components: ComponentSet,
    since: String,
    force_all: bool,
    evidence: Option<PathBuf>,
    workspace: PathBuf,
    repository_token: String,
}

#[derive(Debug)]
enum Comparison {
    All,
    Since { reference: String, merge_base: bool },
}

pub(crate) fn run_planner() -> Result<()> {
    let inputs = PlannerInputs::load()?;
    let installed = install::install_cargo_rail(&inputs.version, inputs.components)?;
    let base = match select_comparison(&inputs.workspace, &inputs.since, inputs.force_all)? {
        Comparison::All => None,
        Comparison::Since { reference, merge_base } => Some(ensure_history(
            &inputs.workspace,
            &reference,
            merge_base,
            &inputs.repository_token,
        )?),
    };

    if inputs.components.needs_surface() {
        let mut command = Command::new(installed.binary());
        command
            .current_dir(&inputs.workspace)
            .args(["rail", "surface", "--prepare", "-f", "json"]);
        let output = run_bounded(&mut command, MAX_SUBPROCESS_BYTES, MAX_SUBPROCESS_BYTES)?;
        validate_machine_success(&output, "surface", "prepare", "Surface preparation")?;
    }

    let runner_temp = canonical_directory_from_env("RUNNER_TEMP")?;
    let plan_directory = create_private_directory(&runner_temp, "cargo-rail-plan")?;
    let plan_path = plan_directory.join("plan.json");
    let mut command = Command::new(installed.binary());
    command.current_dir(&inputs.workspace).args(["rail", "plan", "--json"]);
    if let Some(base) = base {
        command.arg("--since").arg(base);
    } else {
        command.arg("--all");
    }
    if let Some(evidence) = &inputs.evidence {
        command.arg("--evidence").arg(evidence);
    }
    let output = run_bounded(&mut command, MAX_PLAN_BYTES, MAX_SUBPROCESS_BYTES)?;
    if !output.status.success() {
        return Err(subprocess_failure("Cargo-Rail planning", &output));
    }
    let plan = ValidatedPlan::from_bytes(output.stdout)?;
    std::fs::write(&plan_path, plan.bytes()).map_err(|error| {
        ActionError::operational(format!("cannot write private plan '{}': {error}", plan_path.display()))
    })?;
    set_private_file(&plan_path)?;

    crate::plan::verify_checkout_with(plan.bytes(), installed.binary(), Some(&inputs.workspace))?;
    let required = serde_json::to_string(&plan.required_strings())
        .map_err(|error| ActionError::operational(format!("cannot encode required work: {error}")))?;
    publish(Publication {
        summary: Some(plan.render_summary()),
        paths: vec![install::runtime_directory()?, installed.directory().to_path_buf()],
        outputs: vec![
            ("version".to_string(), inputs.version.clone()),
            ("plan-file".to_string(), canonical_text(&plan_path)?),
            ("required-work".to_string(), required),
        ],
    })?;
    let required_count = plan.required().len();
    let skipped_count = plan.work_count().saturating_sub(required_count);
    let changed = plan.changed_files();
    println!(
        "Cargo-Rail plan ready: {required_count} required, {skipped_count} skipped, {changed} changed file{}",
        if changed == 1 { "" } else { "s" }
    );
    Ok(())
}

impl PlannerInputs {
    fn load() -> Result<Self> {
        let version = env_string("INPUT_VERSION")?;
        install::validate_cargo_rail_version(&version)?;
        let components = ComponentSet::planner(&env_string("INPUT_COMPONENTS")?)?;
        let since = optional_env("INPUT_SINCE")?;
        validate_ref_or_empty(&since, "since")?;
        let force_all = parse_bool(&env_string("INPUT_ALL")?, "all")?;
        let repository_token = optional_env("INPUT_REPOSITORY_TOKEN")?;
        if repository_token.len() > MAX_TOKEN_BYTES || repository_token.contains(['\r', '\n']) {
            return Err(ActionError::rejected(
                "repository-token must be one value within the 4 KiB bound",
            ));
        }

        let checkout = canonical_directory_from_env("GITHUB_WORKSPACE")?;
        let working_input = env_string("INPUT_WORKING_DIRECTORY")?;
        validate_path_input(&working_input, "working-directory")?;
        let workspace = canonical_contained_directory(&checkout, &checkout.join(&working_input), "working-directory")?;
        if !workspace.join("Cargo.toml").is_file() {
            return Err(ActionError::rejected("working-directory does not contain Cargo.toml"));
        }
        let evidence_input = optional_env("INPUT_EVIDENCE")?;
        let evidence = if evidence_input.is_empty() {
            None
        } else {
            validate_path_input(&evidence_input, "evidence")?;
            let candidate = workspace.join(evidence_input);
            let canonical = std::fs::canonicalize(&candidate).map_err(|error| {
                ActionError::rejected(format!(
                    "evidence file '{}' is unavailable: {error}",
                    candidate.display()
                ))
            })?;
            if !canonical.starts_with(&workspace) {
                return Err(ActionError::rejected("evidence file escapes working-directory"));
            }
            let metadata = std::fs::symlink_metadata(&canonical).map_err(|error| {
                ActionError::operational(format!(
                    "cannot inspect evidence file '{}': {error}",
                    canonical.display()
                ))
            })?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(ActionError::rejected(
                    "evidence must resolve to an in-workspace regular file",
                ));
            }
            Some(canonical)
        };
        Ok(Self {
            version,
            components,
            since,
            force_all,
            evidence,
            workspace,
            repository_token,
        })
    }
}

fn select_comparison(workspace: &Path, explicit: &str, force_all: bool) -> Result<Comparison> {
    if force_all {
        return Ok(Comparison::All);
    }
    if !explicit.is_empty() {
        if is_zero_sha(explicit) {
            return Ok(Comparison::All);
        }
        return Ok(Comparison::Since {
            reference: explicit.to_string(),
            merge_base: false,
        });
    }
    if optional_env("GITHUB_EVENT_NAME")? == "push" {
        let before = push_before()?;
        if is_zero_sha(&before) {
            return Ok(Comparison::All);
        }
        return Ok(Comparison::Since {
            reference: before,
            merge_base: false,
        });
    }
    let pull_request_base = optional_env("GITHUB_BASE_REF")?;
    if !pull_request_base.is_empty() {
        validate_ref(&pull_request_base, "pull-request base")?;
        return Ok(Comparison::Since {
            reference: format!("origin/{pull_request_base}"),
            merge_base: true,
        });
    }
    if let Some(remote_default) = git_optional(
        workspace,
        ["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"],
    )? {
        validate_ref(&remote_default, "origin default branch")?;
        return Ok(Comparison::Since {
            reference: remote_default,
            merge_base: true,
        });
    }
    for candidate in ["origin/main", "origin/master"] {
        if git_success(workspace, ["rev-parse", "--verify", &format!("{candidate}^{{commit}}")])? {
            return Ok(Comparison::Since {
                reference: candidate.to_string(),
                merge_base: true,
            });
        }
    }
    Ok(Comparison::Since {
        reference: "HEAD~1".to_string(),
        merge_base: false,
    })
}

fn push_before() -> Result<String> {
    let event_path = PathBuf::from(env_string("GITHUB_EVENT_PATH")?);
    let metadata = std::fs::symlink_metadata(&event_path)
        .map_err(|error| ActionError::operational(format!("cannot inspect GitHub push event: {error}")))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_EVENT_BYTES {
        return Err(ActionError::rejected(
            "GitHub push event must be a regular file within the 4 MiB input bound",
        ));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    File::open(&event_path)
        .and_then(|file| file.take(MAX_EVENT_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| ActionError::operational(format!("cannot read GitHub push event: {error}")))?;
    if bytes.len() as u64 > MAX_EVENT_BYTES {
        return Err(ActionError::rejected("GitHub push event exceeds the 4 MiB input bound"));
    }
    let event = parse_unique_json(&bytes, "GitHub push event")?;
    let before = event
        .as_object()
        .and_then(|value| value.get("before"))
        .and_then(Value::as_str)
        .ok_or_else(|| ActionError::rejected("GitHub push event has no previous commit SHA"))?;
    if !is_sha(before) {
        return Err(ActionError::rejected(
            "GitHub push event has no valid previous commit SHA",
        ));
    }
    Ok(before.to_string())
}

fn ensure_history(workspace: &Path, reference: &str, merge_base: bool, token: &str) -> Result<String> {
    validate_ref(reference, "comparison ref")?;
    if reference.starts_with("HEAD~") && head_relative_depth(reference).is_none() {
        return Err(ActionError::rejected(
            "comparison ref has an invalid or excessive HEAD-relative depth",
        ));
    }
    let mut resolved = reference.to_string();
    if let Some(depth) = head_relative_depth(reference) {
        if !has_commit(workspace, reference)? {
            fetch(
                workspace,
                token,
                ["--no-tags", &format!("--depth={}", depth + 1), "origin", "HEAD"],
            )?;
        }
    } else if is_sha(reference) {
        if !has_commit(workspace, reference)? {
            let first = fetch(workspace, token, ["--no-tags", "--depth=1", "origin", reference]);
            if first.is_err() || !has_commit(workspace, reference)? {
                unshallow(workspace, token)?;
                fetch(workspace, token, ["--no-tags", "origin", reference])?;
            }
        }
    } else if !has_commit(workspace, reference)? {
        if let Some(branch) = reference.strip_prefix("origin/") {
            resolved = format!("refs/remotes/origin/{branch}");
            let spec = format!("refs/heads/{branch}:{resolved}");
            fetch(workspace, token, ["--no-tags", "--depth=1", "origin", &spec])?;
        } else if let Some(branch) = reference.strip_prefix("refs/heads/") {
            resolved = format!("refs/remotes/origin/{branch}");
            let spec = format!("refs/heads/{branch}:{resolved}");
            fetch(workspace, token, ["--no-tags", "--depth=1", "origin", &spec])?;
        } else if reference.starts_with("refs/tags/") {
            let spec = format!("{reference}:{reference}");
            fetch(workspace, token, ["--depth=1", "origin", &spec])?;
        } else {
            resolved = format!("refs/remotes/origin/{reference}");
            let branch_spec = format!("refs/heads/{reference}:{resolved}");
            if fetch(workspace, token, ["--no-tags", "--depth=1", "origin", &branch_spec]).is_err() {
                resolved = format!("refs/tags/{reference}");
                let tag_spec = format!("{resolved}:{resolved}");
                fetch(workspace, token, ["--depth=1", "origin", &tag_spec])?;
            }
        }
    }
    if !has_commit(workspace, &resolved)? {
        return Err(ActionError::rejected(format!(
            "cannot resolve comparison ref {reference} after bounded history acquisition"
        )));
    }
    if merge_base && git_optional(workspace, ["merge-base", "HEAD", &resolved])?.is_none() {
        unshallow(workspace, token)?;
        if let Some(branch) = resolved.strip_prefix("refs/remotes/origin/") {
            let spec = format!("refs/heads/{branch}:{resolved}");
            let _ = fetch(workspace, token, ["--no-tags", "origin", &spec]);
        }
    }
    let arguments = if merge_base {
        vec!["merge-base".to_string(), "HEAD".to_string(), resolved]
    } else {
        vec![
            "rev-parse".to_string(),
            "--verify".to_string(),
            format!("{resolved}^{{commit}}"),
        ]
    };
    git_required(
        workspace,
        arguments.iter().map(String::as_str),
        "cannot resolve comparison base",
    )
}

fn unshallow(workspace: &Path, token: &str) -> Result<()> {
    if git_optional(workspace, ["rev-parse", "--is-shallow-repository"])? == Some("true".to_string()) {
        fetch(workspace, token, ["--no-tags", "--unshallow", "origin"])?;
    }
    Ok(())
}

fn fetch<I, S>(workspace: &Path, token: &str, arguments: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new("git");
    command.current_dir(workspace).arg("fetch").args(arguments);
    command.env("GIT_TERMINAL_PROMPT", "0");
    command
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_CONFIG_PARAMETERS");
    for index in 0..64 {
        command
            .env_remove(format!("GIT_CONFIG_KEY_{index}"))
            .env_remove(format!("GIT_CONFIG_VALUE_{index}"));
    }
    if !token.is_empty() {
        let (server, repository) = github_repository_authority()?;
        let origin = git_required(workspace, ["remote", "get-url", "origin"], "cannot inspect origin URL")?;
        let expected = format!("{server}/{repository}");
        if origin != expected && origin != format!("{expected}.git") {
            return Err(ActionError::rejected(
                "origin disagrees with the GitHub repository context; repository-token was not transmitted",
            ));
        }
        command
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", format!("http.{expected}/.extraHeader"))
            .env(
                "GIT_CONFIG_VALUE_0",
                format!(
                    "AUTHORIZATION: basic {}",
                    base64(format!("x-access-token:{token}").as_bytes())
                ),
            );
    }
    let output = run_bounded(&mut command, MAX_SUBPROCESS_BYTES, MAX_SUBPROCESS_BYTES)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(ActionError::operational(format!(
            "Git history fetch failed with exit code {}; verify the same-repository history authority and retry",
            output.status.code().unwrap_or(1)
        )))
    }
}

fn github_repository_authority() -> Result<(String, String)> {
    let server = env_string("GITHUB_SERVER_URL")?;
    let repository = env_string("GITHUB_REPOSITORY")?;
    if !server.starts_with("https://")
        || server.ends_with('/')
        || server.contains(['\r', '\n'])
        || repository.starts_with('/')
        || repository.ends_with('/')
        || repository.split('/').count() != 2
        || repository.contains(['\r', '\n'])
    {
        return Err(ActionError::rejected(
            "GitHub server or repository context is malformed",
        ));
    }
    Ok((server, repository))
}

fn has_commit(workspace: &Path, reference: &str) -> Result<bool> {
    git_success(workspace, ["rev-parse", "--verify", &format!("{reference}^{{commit}}")])
}

fn git_success<I, S>(workspace: &Path, arguments: I) -> Result<bool>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new("git");
    command.current_dir(workspace).args(arguments);
    Ok(run_bounded(&mut command, MAX_SUBPROCESS_BYTES, MAX_SUBPROCESS_BYTES)?
        .status
        .success())
}

fn git_optional<I, S>(workspace: &Path, arguments: I) -> Result<Option<String>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new("git");
    command.current_dir(workspace).args(arguments);
    let output = run_bounded(&mut command, MAX_SUBPROCESS_BYTES, MAX_SUBPROCESS_BYTES)?;
    if !output.status.success() {
        return Ok(None);
    }
    let value = String::from_utf8(output.stdout)
        .map_err(|_| ActionError::rejected("Git returned non-UTF-8 output"))?
        .trim()
        .to_string();
    Ok(Some(value))
}

fn git_required<I, S>(workspace: &Path, arguments: I, subject: &str) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new("git");
    command.current_dir(workspace).args(arguments);
    let output = run_bounded(&mut command, MAX_SUBPROCESS_BYTES, MAX_SUBPROCESS_BYTES)?;
    if !output.status.success() {
        return Err(subprocess_failure(subject, &output));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|_| ActionError::rejected(format!("{subject}: Git returned non-UTF-8 output")))
}

pub(crate) fn run_bounded(command: &mut Command, max_stdout: usize, max_stderr: usize) -> Result<BoundedOutput> {
    command
        .env_remove("INPUT_REPOSITORY_TOKEN")
        .env_remove("GH_TOKEN")
        .env_remove("INPUT_REMOTE")
        .env_remove("INPUT_LOCAL_DIR");
    run_bounded_inner(command, None, max_stdout, max_stderr)
}

pub(crate) fn run_bounded_with_input(
    command: &mut Command,
    input: &[u8],
    max_stdout: usize,
    max_stderr: usize,
) -> Result<BoundedOutput> {
    command
        .env_remove("INPUT_REPOSITORY_TOKEN")
        .env_remove("GH_TOKEN")
        .env_remove("INPUT_REMOTE")
        .env_remove("INPUT_LOCAL_DIR")
        .stdin(Stdio::piped());
    run_bounded_inner(command, Some(input), max_stdout, max_stderr)
}

pub(crate) fn run_bounded_gh(
    command: &mut Command,
    token: &str,
    max_stdout: usize,
    max_stderr: usize,
) -> Result<BoundedOutput> {
    command
        .env_remove("INPUT_REPOSITORY_TOKEN")
        .env_remove("INPUT_REMOTE")
        .env_remove("INPUT_LOCAL_DIR")
        .env("GH_TOKEN", token);
    run_bounded_inner(command, None, max_stdout, max_stderr)
}

fn run_bounded_inner(
    command: &mut Command,
    input: Option<&[u8]>,
    max_stdout: usize,
    max_stderr: usize,
) -> Result<BoundedOutput> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| ActionError::operational(format!("cannot execute {:?}: {error}", command.get_program())))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ActionError::operational("cannot capture child stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ActionError::operational("cannot capture child stderr"))?;
    let stdout_thread = thread::spawn(move || read_bounded(stdout, max_stdout));
    let stderr_thread = thread::spawn(move || read_bounded(stderr, max_stderr));
    let input_result = input.map(|bytes| {
        child
            .stdin
            .take()
            .ok_or_else(|| ActionError::operational("cannot open child stdin"))?
            .write_all(bytes)
            .map_err(|error| ActionError::operational(format!("cannot write child stdin: {error}")))
    });
    let status = child
        .wait()
        .map_err(|error| ActionError::operational(format!("cannot wait for child: {error}")))?;
    let stdout = stdout_thread
        .join()
        .map_err(|_| ActionError::operational("stdout capture thread failed"))??;
    let stderr = stderr_thread
        .join()
        .map_err(|_| ActionError::operational("stderr capture thread failed"))??;
    if let Some(result) = input_result {
        result?;
    }
    Ok(BoundedOutput { status, stdout, stderr })
}

fn read_bounded<R: Read>(mut reader: R, maximum: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(u64::try_from(maximum).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| ActionError::operational(format!("cannot read child output: {error}")))?;
    if bytes.len() > maximum {
        return Err(ActionError::rejected(format!(
            "subprocess output exceeds its {maximum}-byte bound"
        )));
    }
    Ok(bytes)
}

pub(crate) fn find_executable(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    find_executable_in(name, &path)
}

fn find_executable_in(name: &str, path: &OsStr) -> Option<PathBuf> {
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let filename = format!("{name}{suffix}");
    std::env::split_paths(path)
        .map(|directory| directory.join(&filename))
        .find(|candidate| is_executable_file(candidate))
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.permissions().mode() & 0o111 != 0
    })
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
}

pub(crate) fn validate_machine_success(
    output: &BoundedOutput,
    command_name: &str,
    mode: &str,
    subject: &str,
) -> Result<Value> {
    if !output.status.success() {
        return Err(subprocess_failure(subject, output));
    }
    let value = parse_unique_json(&output.stdout, subject)?;
    let object = value
        .as_object()
        .ok_or_else(|| ActionError::rejected(format!("{subject} did not return a machine envelope")))?;
    for (field, expected) in [
        ("schema_version", Value::from(1)),
        ("command", Value::from(command_name)),
        ("mode", Value::from(mode)),
        ("result", Value::from("success")),
        ("exit_code", Value::from(0)),
    ] {
        if object.get(field) != Some(&expected) {
            return Err(ActionError::rejected(format!("{subject} returned invalid {field}")));
        }
    }
    Ok(value)
}

pub(crate) fn subprocess_failure(subject: &str, output: &BoundedOutput) -> ActionError {
    let detail =
        machine_failure_detail(output).unwrap_or_else(|| String::from_utf8_lossy(&output.stderr).trim().to_string());
    let suffix = if detail.is_empty() {
        String::new()
    } else {
        format!(": {detail}")
    };
    ActionError::operational(format!(
        "{subject} failed with exit code {}{suffix}",
        output.status.code().unwrap_or(1)
    ))
}

/// Extract only the diagnostic fields from Cargo-Rail's two failure envelopes.
fn machine_failure_detail(output: &BoundedOutput) -> Option<String> {
    let value = parse_unique_json(&output.stdout, "Cargo-Rail error").ok()?;
    let code = i64::from(output.status.code()?);
    let (failure, recovery) =
        if value.get("error") == Some(&Value::Bool(true)) && value.get("code").and_then(Value::as_i64) == Some(code) {
            (&value, "help")
        } else if value.get("schema_version").and_then(Value::as_u64) == Some(1)
            && value.get("exit_code").and_then(Value::as_i64) == Some(code)
            && value.get("command").and_then(Value::as_str) == Some("cache")
            && value.get("result").and_then(Value::as_str) == Some("probe_failed")
        {
            (value.get("failure")?, "retry")
        } else {
            return None;
        };
    let message = failure.get("message")?.as_str()?.trim();
    if message.is_empty() {
        return None;
    }
    let mut detail = message.to_string();
    if let Some(context) = failure
        .get("context")
        .and_then(Value::as_str)
        .filter(|context| !context.is_empty())
    {
        detail.push('\n');
        detail.push_str(context);
    }
    if let Some(help) = failure
        .get(recovery)
        .and_then(Value::as_str)
        .filter(|help| !help.is_empty())
    {
        detail.push_str("\nNext: ");
        detail.push_str(help);
    }
    Some(detail)
}

fn parse_bool(value: &str, name: &str) -> Result<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ActionError::rejected(format!("{name} must be true or false"))),
    }
}

fn validate_ref_or_empty(value: &str, subject: &str) -> Result<()> {
    if value.is_empty() {
        Ok(())
    } else {
        validate_ref(value, subject)
    }
}

fn validate_ref(value: &str, subject: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_PATH_BYTES
        || value.trim() != value
        || value.starts_with('-')
        || value.contains(['\r', '\n'])
        || value.as_bytes().contains(&0)
    {
        return Err(ActionError::rejected(format!(
            "{subject} must be one bounded, non-option Git ref without padding"
        )));
    }
    Ok(())
}

fn is_sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_zero_sha(value: &str) -> bool {
    is_sha(value) && value.bytes().all(|byte| byte == b'0')
}

fn head_relative_depth(value: &str) -> Option<u64> {
    let depth = value.strip_prefix("HEAD~")?.parse().ok()?;
    (depth <= 1_000_000).then_some(depth)
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        encoded.push(char::from(ALPHABET[usize::from(first >> 2)]));
        encoded.push(char::from(ALPHABET[usize::from((first & 0x03) << 4 | second >> 4)]));
        encoded.push(if chunk.len() > 1 {
            char::from(ALPHABET[usize::from((second & 0x0f) << 2 | third >> 6)])
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            char::from(ALPHABET[usize::from(third & 0x3f)])
        } else {
            '='
        });
    }
    encoded
}

fn validate_path_input(value: &str, subject: &str) -> Result<()> {
    if value.is_empty() || value.len() > MAX_PATH_BYTES || value.contains(['\r', '\n']) || value.as_bytes().contains(&0)
    {
        return Err(ActionError::rejected(format!(
            "{subject} must be one non-empty path no longer than 4 KiB"
        )));
    }
    Ok(())
}

fn canonical_directory_from_env(name: &str) -> Result<PathBuf> {
    let path = PathBuf::from(env_string(name)?);
    let canonical = std::fs::canonicalize(&path)
        .map_err(|error| ActionError::operational(format!("cannot resolve {name} '{}': {error}", path.display())))?;
    if !canonical.is_dir() {
        return Err(ActionError::rejected(format!("{name} must name an existing directory")));
    }
    Ok(canonical)
}

fn canonical_contained_directory(root: &Path, candidate: &Path, subject: &str) -> Result<PathBuf> {
    let canonical = std::fs::canonicalize(candidate).map_err(|error| {
        ActionError::rejected(format!("cannot resolve {subject} '{}': {error}", candidate.display()))
    })?;
    if !canonical.is_dir() || !canonical.starts_with(root) {
        return Err(ActionError::rejected(format!(
            "{subject} must remain inside the canonical checkout"
        )));
    }
    Ok(canonical)
}

pub(crate) fn create_private_directory(parent: &Path, prefix: &str) -> Result<PathBuf> {
    for _ in 0..128 {
        let counter = PRIVATE_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!("{prefix}.{}.{counter}", std::process::id()));
        match std::fs::create_dir(&candidate) {
            Ok(()) => {
                set_private_directory(&candidate)?;
                return Ok(candidate);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(ActionError::operational(format!(
                    "cannot create private directory '{}': {error}",
                    candidate.display()
                )));
            }
        }
    }
    Err(ActionError::operational("cannot allocate a unique private directory"))
}

#[cfg(unix)]
fn set_private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(ActionError::from)
}

#[cfg(not(unix))]
fn set_private_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(ActionError::from)
}

#[cfg(not(unix))]
fn set_private_file(_path: &Path) -> Result<()> {
    Ok(())
}

fn canonical_text(path: &Path) -> Result<String> {
    std::fs::canonicalize(path)
        .map_err(|error| ActionError::operational(format!("cannot resolve '{}': {error}", path.display())))?
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| ActionError::rejected("published path is not valid UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt as _;

    #[test]
    fn machine_errors_preserve_cause_and_recovery() {
        let status = Command::new("git")
            .args(["rev-parse", "--verify", "refs/heads/__missing_error_fixture__"])
            .output()
            .expect("git fixture")
            .status;
        let code = status.code().expect("exit code");
        for value in [
            serde_json::json!({"error": true, "code": code, "message": "invalid planning configuration", "help": "correct plan.work.tests"}),
            serde_json::json!({"schema_version": 1, "command": "cache", "mode": "probe", "result": "probe_failed", "exit_code": code,
                "failure": {"message": "remote cache request timed out", "retry": "check provider availability and retry"}}),
        ] {
            let output = BoundedOutput {
                status,
                stdout: serde_json::to_vec(&value).expect("JSON"),
                stderr: Vec::new(),
            };
            let error = subprocess_failure("operation", &output).to_string();
            let failure = value.get("failure").unwrap_or(&value);
            assert!(error.contains(failure["message"].as_str().expect("message")), "{error}");
            assert!(error.contains("Next: "), "{error}");
        }
        let output = BoundedOutput {
            status,
            stdout: b"{not JSON".to_vec(),
            stderr: b"compiler unavailable".to_vec(),
        };
        assert!(
            subprocess_failure("operation", &output)
                .to_string()
                .contains("compiler unavailable")
        );
    }

    #[test]
    fn rejects_option_like_and_padded_refs() {
        assert!(validate_ref("main", "ref").is_ok());
        assert!(validate_ref("-main", "ref").is_err());
        assert!(validate_ref(" main", "ref").is_err());
        assert!(validate_ref("main\nother", "ref").is_err());
    }

    #[test]
    fn recognizes_only_bounded_shas() {
        assert!(is_sha(&"a".repeat(40)));
        for length in [39, 41, 63, 65] {
            assert!(!is_sha(&"a".repeat(length)), "accepted {length} digits");
            assert!(
                !is_zero_sha(&"0".repeat(length)),
                "accepted {length}-digit zero sentinel"
            );
        }
        assert!(is_zero_sha(&"0".repeat(64)));
    }

    #[test]
    fn bounds_head_relative_history_and_encodes_basic_authority() {
        assert_eq!(head_relative_depth("HEAD~12"), Some(12));
        assert_eq!(head_relative_depth("HEAD~1000001"), None);
        assert_eq!(head_relative_depth("HEAD~-1"), None);
        assert_eq!(base64(b"x-access-token:test"), "eC1hY2Nlc3MtdG9rZW46dGVzdA==");
    }

    #[cfg(unix)]
    #[test]
    fn executable_lookup_skips_unusable_path_entries() {
        let parent = std::env::temp_dir();
        let first = create_private_directory(&parent, "cargo-rail-action-path-first").expect("first directory");
        let second = create_private_directory(&parent, "cargo-rail-action-path-second").expect("second directory");
        let unusable = first.join("fixture-tool");
        let executable = second.join("fixture-tool");
        std::fs::write(&unusable, b"not executable").expect("unusable fixture");
        std::fs::write(&executable, b"executable").expect("executable fixture");
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).expect("executable permissions");
        let path = std::env::join_paths([&first, &second]).expect("fixture PATH");
        let found = find_executable_in("fixture-tool", &path);
        std::fs::remove_dir_all(first).expect("remove first directory");
        std::fs::remove_dir_all(second).expect("remove second directory");

        assert_eq!(found, Some(executable));
    }
}
