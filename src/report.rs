//! One bounded record per job and one workflow summary. Records never authorize execution.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::{ActionError, Result, env_string, github, repository};

const MAX_RECORD_BYTES: u64 = 32 * 1024;
const MAX_JOBS: usize = 512;
const CONTEXT_ENV: &str = "CARGO_RAIL_ACTION_CACHE_CONTEXT";
const RECORDING_ENV: &str = "CARGO_RAIL_CACHE_REPORT";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunBinding {
    repository: String,
    run_id: String,
    attempt: String,
}

impl RunBinding {
    fn current() -> Result<Self> {
        let binding = Self {
            repository: env_string("GITHUB_REPOSITORY")?,
            run_id: env_string("GITHUB_RUN_ID")?,
            attempt: env_string("GITHUB_RUN_ATTEMPT")?,
        };
        if binding.repository.len() > 256
            || binding.repository.split('/').count() != 2
            || !binding
                .repository
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"/_.-".contains(&byte))
            || ![&binding.run_id, &binding.attempt]
                .iter()
                .all(|value| !value.is_empty() && value.len() <= 20 && value.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return Err(ActionError::rejected(
                "cache report requires a valid workflow run binding",
            ));
        }
        Ok(binding)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    schema_version: u32,
    cargo_rail: String,
    provider: String,
    mode: String,
    max_bytes: u64,
    root_portability: String,
    remote_verification: String,
}

impl Configuration {
    fn validate(&self) -> Result<()> {
        crate::install::validate_cargo_rail_version(&self.cargo_rail)?;
        if self.schema_version != 1
            || !matches!(
                self.provider.as_str(),
                "aws-s3" | "azure-blob" | "cloudflare-r2" | "s3-compatible"
            )
            || !matches!(self.mode.as_str(), "read" | "read-write")
            || self.max_bytes == 0
            || !matches!(self.root_portability.as_str(), "physical" | "remap")
            || !matches!(self.remote_verification.as_str(), "verified" | "not_requested")
        {
            return Err(ActionError::rejected("invalid cache report configuration"));
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Context {
    schema_version: u32,
    run: RunBinding,
    binary: PathBuf,
    workspace: PathBuf,
    recording: PathBuf,
    configuration: Configuration,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Measurements {
    hits: u64,
    misses: u64,
    bypasses: u64,
    failures: u64,
    local_bytes_read: u64,
    remote_bytes_read: u64,
    remote_bytes_written: u64,
    bypass_reasons: BTreeMap<String, u64>,
    failure_reasons: BTreeMap<String, u64>,
    incomplete: bool,
}

impl Measurements {
    fn validate(&self) -> Result<()> {
        for reasons in [&self.bypass_reasons, &self.failure_reasons] {
            if reasons.len() > 128
                || reasons.iter().any(|(reason, count)| {
                    reason.is_empty()
                        || reason.len() > 96
                        || *count == 0
                        || !reason
                            .bytes()
                            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
                })
            {
                return Err(ActionError::rejected("invalid cache measurement reasons"));
            }
        }
        Ok(())
    }

    fn merge(&mut self, other: &Self) -> Result<()> {
        fn add(left: &mut u64, right: u64) -> Result<()> {
            *left = left
                .checked_add(right)
                .ok_or_else(|| ActionError::rejected("cache report totals overflow"))?;
            Ok(())
        }
        add(&mut self.hits, other.hits)?;
        add(&mut self.misses, other.misses)?;
        add(&mut self.bypasses, other.bypasses)?;
        add(&mut self.failures, other.failures)?;
        add(&mut self.local_bytes_read, other.local_bytes_read)?;
        add(&mut self.remote_bytes_read, other.remote_bytes_read)?;
        add(&mut self.remote_bytes_written, other.remote_bytes_written)?;
        for (into, from) in [
            (&mut self.bypass_reasons, &other.bypass_reasons),
            (&mut self.failure_reasons, &other.failure_reasons),
        ] {
            for (reason, count) in from {
                add(into.entry(reason.clone()).or_default(), *count)?;
            }
        }
        self.incomplete |= other.incomplete;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Storage {
    bytes: u64,
    max_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JobRecord {
    schema_version: u32,
    run: RunBinding,
    job: String,
    configuration: Configuration,
    measurements: Option<Measurements>,
    storage: Option<Storage>,
    gaps: BTreeSet<String>,
}

fn valid_job(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
}

pub(crate) fn begin(binary: &Path, workspace: &Path, configuration: &str) -> Result<()> {
    let configuration: Configuration = serde_json::from_str(configuration)
        .map_err(|error| ActionError::rejected(format!("invalid cache configuration: {error}")))?;
    configuration.validate()?;
    let parent = fs::canonicalize(env_string("RUNNER_TEMP")?)?;
    let directory = repository::create_private_directory(&parent, "cargo-rail-cache-report")?;
    let recording = directory.join("recording.json");
    let context = Context {
        schema_version: 1,
        run: RunBinding::current()?,
        binary: binary.to_path_buf(),
        workspace: workspace.to_path_buf(),
        recording,
        configuration,
    };
    let mut command = Command::new(binary);
    command
        .current_dir(workspace)
        .args(["rail", "cache", "report", "--start"])
        .arg(&context.recording)
        .args(["-f", "json"]);
    let output = repository::run_bounded(&mut command, MAX_RECORD_BYTES as usize, MAX_RECORD_BYTES as usize)?;
    repository::validate_machine_success(&output, "cache", "report", "Cache recording initialization")?;
    let context_path = directory.join("context.json");
    write_new(&context_path, &context)?;
    github::publish_environment(&[(RECORDING_ENV, &context.recording), (CONTEXT_ENV, &context_path)])
}

pub(crate) fn collect_action() -> Result<()> {
    let job = env_string("INPUT_JOB")?;
    if !valid_job(&job) {
        return Err(ActionError::rejected(
            "cache report job must be a short unique label using letters, digits, dots, hyphens, or underscores",
        ));
    }
    let context_path = PathBuf::from(env_string(CONTEXT_ENV)?);
    let context: Context = decode(&context_path)?;
    let run = RunBinding::current()?;
    if context.schema_version != 1
        || context.run != run
        || !context.binary.is_absolute()
        || !context.workspace.is_absolute()
        || !context.recording.is_absolute()
        || context.recording.parent() != context_path.parent()
    {
        return Err(ActionError::rejected(
            "cache recording context does not belong to this run",
        ));
    }
    context.configuration.validate()?;
    let mut gaps = BTreeSet::new();
    let measurements = collect_measurements(&context)
        .map_err(|_| gaps.insert("measurements_unavailable".to_string()))
        .ok();
    if measurements.as_ref().is_some_and(|counts| counts.incomplete) {
        gaps.insert("recording_incomplete".to_string());
    }
    let storage = collect_storage(&context)
        .map_err(|_| gaps.insert("storage_unavailable".to_string()))
        .ok();
    let record = JobRecord {
        schema_version: 1,
        run,
        job: job.clone(),
        configuration: context.configuration,
        measurements,
        storage,
        gaps,
    };
    let directory = PathBuf::from(env_string("INPUT_OUTPUT_DIRECTORY")?);
    if !directory.is_absolute() {
        return Err(ActionError::rejected("cache record output directory must be absolute"));
    }
    fs::create_dir_all(&directory)?;
    let directory = fs::canonicalize(directory)?;
    let path = directory.join(format!("{job}.json"));
    if path.try_exists()? {
        let previous: JobRecord = decode(&path)?;
        if previous != record {
            return Err(ActionError::rejected(
                "conflicting cache record already exists for this job",
            ));
        }
    } else {
        write_new(&path, &record)?;
    }
    github::publish(github::Publication {
        summary: None,
        paths: Vec::new(),
        outputs: vec![(
            "record-file".to_string(),
            path.to_str()
                .ok_or_else(|| ActionError::rejected("cache record path is not UTF-8"))?
                .to_string(),
        )],
    })?;
    println!("Cache measurements collected for {job}.");
    Ok(())
}

fn collect_measurements(context: &Context) -> Result<Measurements> {
    let mut command = Command::new(&context.binary);
    command
        .current_dir(&context.workspace)
        .args(["rail", "cache", "report", "--finish"])
        .arg(&context.recording)
        .args(["-f", "json"]);
    let output = repository::run_bounded(&mut command, MAX_RECORD_BYTES as usize, MAX_RECORD_BYTES as usize)?;
    let value = repository::validate_machine_success(&output, "cache", "report", "Cache measurement collection")?;
    if value["operation"] != "finish" {
        return Err(ActionError::rejected("cache recording did not finish"));
    }
    let counts: Measurements = serde_json::from_value(value["measurements"].clone())
        .map_err(|error| ActionError::rejected(format!("invalid cache measurements: {error}")))?;
    counts.validate()?;
    Ok(counts)
}

fn collect_storage(context: &Context) -> Result<Storage> {
    let mut command = Command::new(&context.binary);
    command
        .current_dir(&context.workspace)
        .args(["rail", "cache", "status", "--scope", "local", "-f", "json"]);
    let output = repository::run_bounded(&mut command, 1024 * 1024, MAX_RECORD_BYTES as usize)?;
    let value = repository::validate_machine_success(&output, "cache", "status", "Cache storage collection")?;
    crate::cache::validate_report_status(
        &value,
        &context.configuration.mode,
        &context.configuration.root_portability,
        context.configuration.max_bytes,
    )?;
    let cache = &value["status"]["local"]["cache"];
    let bytes = cache["bytes"]
        .as_u64()
        .ok_or_else(|| ActionError::rejected("cache storage bytes are unavailable"))?;
    let max_bytes = cache["max_bytes"]
        .as_u64()
        .filter(|bytes| *bytes == context.configuration.max_bytes)
        .ok_or_else(|| ActionError::rejected("cache storage authority changed during the job"))?;
    Ok(Storage { bytes, max_bytes })
}

pub(crate) fn report_action() -> Result<()> {
    let expected: Vec<String> = serde_json::from_str(&env_string("INPUT_EXPECTED_JOBS")?)
        .map_err(|error| ActionError::rejected(format!("expected-jobs must be a JSON array: {error}")))?;
    let expected_set = expected.iter().cloned().collect::<BTreeSet<_>>();
    if expected.is_empty()
        || expected.len() > MAX_JOBS
        || expected.len() != expected_set.len()
        || expected.iter().any(|job| !valid_job(job))
    {
        return Err(ActionError::rejected(
            "expected-jobs must contain unique valid job labels",
        ));
    }
    let run = RunBinding::current()?;
    let directory = PathBuf::from(env_string("INPUT_RECORDS_DIRECTORY")?);
    let mut records = BTreeMap::new();
    if directory.try_exists()? {
        for (index, entry) in fs::read_dir(&directory)?.enumerate() {
            if index >= MAX_JOBS {
                return Err(ActionError::rejected("cache record directory exceeds the job bound"));
            }
            let entry = entry?;
            if entry.path().extension().and_then(|extension| extension.to_str()) != Some("json") {
                return Err(ActionError::rejected(
                    "cache record directory must contain only job JSON records",
                ));
            }
            let record: JobRecord = decode(&entry.path())?;
            admit(&mut records, record, &run, &expected_set)?;
        }
    }
    let summary = render(&records, &expected_set)?;
    github::publish_summary(summary)?;
    println!(
        "Cache report ready: {}/{} jobs reported.",
        records.len(),
        expected.len()
    );
    Ok(())
}

fn admit(
    records: &mut BTreeMap<String, JobRecord>,
    record: JobRecord,
    run: &RunBinding,
    expected: &BTreeSet<String>,
) -> Result<()> {
    if record.schema_version != 1 || &record.run != run || !expected.contains(&record.job) {
        return Err(ActionError::rejected(
            "cache record belongs to an unexpected job, run, attempt, or contract",
        ));
    }
    record.configuration.validate()?;
    if let Some(counts) = &record.measurements {
        counts.validate()?;
    }
    if record.gaps.iter().any(|gap| {
        !matches!(
            gap.as_str(),
            "measurements_unavailable" | "recording_incomplete" | "storage_unavailable"
        )
    }) || record.measurements.is_none() != record.gaps.contains("measurements_unavailable")
        || record.storage.is_none() != record.gaps.contains("storage_unavailable")
        || record.measurements.as_ref().is_some_and(|counts| counts.incomplete)
            != record.gaps.contains("recording_incomplete")
        || record
            .storage
            .as_ref()
            .is_some_and(|storage| storage.max_bytes != record.configuration.max_bytes)
    {
        return Err(ActionError::rejected(
            "cache record completeness or storage fields disagree",
        ));
    }
    if let Some(previous) = records.get(&record.job) {
        if previous != &record {
            return Err(ActionError::rejected("conflicting duplicate cache job record"));
        }
    } else {
        records.insert(record.job.clone(), record);
    }
    Ok(())
}

fn render(records: &BTreeMap<String, JobRecord>, expected: &BTreeSet<String>) -> Result<String> {
    let mut totals = Measurements::default();
    let mut measured = 0;
    for record in records.values() {
        if let Some(counts) = &record.measurements {
            totals.merge(counts)?;
            measured += 1;
        }
    }
    let missing = expected
        .iter()
        .filter(|job| !records.contains_key(*job))
        .map(|job| github::markdown_inline(job))
        .collect::<Vec<_>>();
    let mut lines = vec![
        "## Cargo-Rail cache".to_string(),
        String::new(),
        format!("{measured}/{} jobs measured", expected.len()),
        String::new(),
    ];
    if measured > 0 {
        lines.push(format!(
            "**Reused: {} · Misses: {} · Bypasses: {} · Wrapper failures: {}**",
            totals.hits, totals.misses, totals.bypasses, totals.failures
        ));
        lines.push(format!(
            "Local reads: {} · Downloaded: {} · Uploaded: {}",
            bytes(totals.local_bytes_read),
            bytes(totals.remote_bytes_read),
            bytes(totals.remote_bytes_written)
        ));
        if totals.hits == 0 && totals.misses == 0 && totals.bypasses == 0 && totals.failures == 0 {
            lines.push(
                "No compiler-cache outcomes recorded. Cargo freshness can avoid compiler invocation entirely."
                    .to_string(),
            );
        }
    } else {
        lines.push("No cache measurements available.".to_string());
    }
    if !missing.is_empty() {
        lines.push(format!(
            "\n**Missing job reports:** {}. Totals are partial.",
            missing.join(", ")
        ));
    }
    for record in records.values().filter(|record| !record.gaps.is_empty()) {
        let gaps = record
            .gaps
            .iter()
            .map(|gap| match gap.as_str() {
                "measurements_unavailable" => "measurements unavailable",
                "recording_incomplete" => "recording incomplete",
                _ => "storage unavailable",
            })
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("\n**{}:** {gaps}.", github::markdown_inline(&record.job)));
    }
    if !totals.failure_reasons.is_empty() {
        lines.push("\n**Cache problems**".to_string());
        for (reason, count) in &totals.failure_reasons {
            lines.push(format!("- {}: {count}", reason.replace('_', " ")));
        }
    }
    if !totals.bypass_reasons.is_empty() {
        lines.push("\n<details><summary>Why compilation bypassed the cache</summary>\n".to_string());
        for (reason, count) in &totals.bypass_reasons {
            lines.push(format!("- {}: {count}", reason.replace('_', " ")));
        }
        lines.push("\n</details>".to_string());
    }
    if !records.is_empty() {
        lines.push("\n<details><summary>Jobs, storage, and configuration</summary>\n".to_string());
        for record in records.values() {
            let configuration = &record.configuration;
            lines.push(format!(
                "- **{}** — {} · {} · remote {}",
                github::markdown_inline(&record.job),
                configuration.provider,
                configuration.mode,
                if configuration.remote_verification == "verified" {
                    "verified at setup"
                } else {
                    "not checked at setup"
                }
            ));
            if let Some(counts) = &record.measurements {
                lines.push(format!(
                    "  - Reused: {} · Misses: {} · Bypasses: {} · Wrapper failures: {}",
                    counts.hits, counts.misses, counts.bypasses, counts.failures
                ));
            }
            if let Some(storage) = &record.storage {
                lines.push(format!(
                    "  - Local storage: {} / {} limit",
                    bytes(storage.bytes),
                    bytes(storage.max_bytes)
                ));
            }
        }
        lines.push("\n</details>".to_string());
    }
    lines.push("\nCounts cover recorded wrapper outcomes between cache setup and collection. Storage is measured per job; shared storage is not added together.\n".to_string());
    Ok(lines.join("\n"))
}

fn bytes(value: u64) -> String {
    let mut quantity = value as f64;
    for unit in ["B", "KiB", "MiB", "GiB", "TiB"] {
        if quantity < 1024.0 || unit == "TiB" {
            return if unit == "B" {
                format!("{value} B")
            } else {
                format!("{quantity:.1} {unit}")
            };
        }
        quantity /= 1024.0;
    }
    unreachable!()
}

fn decode<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_RECORD_BYTES {
        return Err(ActionError::rejected("cache record must be one bounded regular file"));
    }
    let mut file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(MAX_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(ActionError::rejected("cache record exceeds its byte bound"));
    }
    let value = crate::plan::parse_unique_json(&bytes, "cache record")?;
    serde_json::from_value(value).map_err(|error| ActionError::rejected(format!("invalid cache record: {error}")))
}

fn write_new(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| ActionError::operational(format!("cannot encode cache record: {error}")))?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(ActionError::rejected("cache record exceeds its byte bound"));
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)?.write_all(&bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run() -> RunBinding {
        RunBinding {
            repository: "owner/project".into(),
            run_id: "10".into(),
            attempt: "1".into(),
        }
    }
    fn record(job: &str) -> JobRecord {
        JobRecord {
            schema_version: 1,
            run: run(),
            job: job.into(),
            configuration: Configuration {
                schema_version: 1,
                cargo_rail: "0.26.0".into(),
                provider: "aws-s3".into(),
                mode: "read".into(),
                max_bytes: 1024,
                root_portability: "physical".into(),
                remote_verification: "not_requested".into(),
            },
            measurements: Some(Measurements {
                hits: 4,
                misses: 1,
                bypasses: 2,
                bypass_reasons: BTreeMap::from([("unsupported_invocation".into(), 2)]),
                ..Measurements::default()
            }),
            storage: Some(Storage {
                bytes: 512,
                max_bytes: 1024,
            }),
            gaps: BTreeSet::new(),
        }
    }

    #[test]
    fn workflow_totals_include_every_job_once_and_show_missing_measurements() {
        let expected = BTreeSet::from(["linux".into(), "macos".into(), "windows".into()]);
        let mut records = BTreeMap::new();
        admit(&mut records, record("linux"), &run(), &expected).unwrap();
        admit(&mut records, record("linux"), &run(), &expected).unwrap();
        admit(&mut records, record("macos"), &run(), &expected).unwrap();
        let summary = render(&records, &expected).unwrap();
        assert_eq!(summary.matches("## Cargo-Rail cache").count(), 1);
        assert!(summary.contains("Reused: 8 · Misses: 2 · Bypasses: 4"), "{summary}");
        assert!(summary.contains("2/3 jobs measured"));
        assert!(summary.contains("Missing job reports:** windows"));
        assert!(summary.contains("unsupported invocation: 4"));
        assert!(!summary.contains("s3://"));
        let mut unavailable = record("windows");
        unavailable.measurements = None;
        unavailable.gaps.insert("measurements_unavailable".into());
        admit(&mut records, unavailable, &run(), &expected).unwrap();
        let summary = render(&records, &expected).unwrap();
        assert!(summary.contains("2/3 jobs measured"));
        assert!(summary.contains("windows:** measurements unavailable"));
        assert!(!summary.contains("Missing job reports"));
    }

    #[test]
    fn conflicting_stale_and_inconsistent_records_are_rejected() {
        let expected = BTreeSet::from(["linux".into()]);
        let mut records = BTreeMap::new();
        admit(&mut records, record("linux"), &run(), &expected).unwrap();
        let mut conflict = record("linux");
        conflict.measurements.as_mut().unwrap().hits += 1;
        assert!(admit(&mut records, conflict, &run(), &expected).is_err());
        let mut stale = record("linux");
        stale.run.attempt = "2".into();
        assert!(admit(&mut records, stale, &run(), &expected).is_err());
        let mut missing = record("linux");
        missing.measurements = None;
        assert!(admit(&mut records, missing, &run(), &expected).is_err());
        let mut private = serde_json::to_value(record("linux")).unwrap();
        private["remote_url"] = "s3://private".into();
        assert!(serde_json::from_value::<JobRecord>(private).is_err());
    }

    #[test]
    fn record_remains_small_as_invocation_counts_grow() {
        let mut value = record("linux");
        value.measurements.as_mut().unwrap().hits = 1_000_000;
        assert!(serde_json::to_vec(&value).unwrap().len() < 1024);
        let mut totals = Measurements {
            hits: u64::MAX,
            ..Measurements::default()
        };
        assert!(totals.merge(value.measurements.as_ref().unwrap()).is_err());
    }
}
