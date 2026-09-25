//! Repository-wide audit of Cargo-Rail Action references.
//!
//! Discovery reads every tracked or unignored YAML file in the repository, not a
//! supplied list. Each reference is checked against the interface of the action
//! it names, as bundled in this runtime. A step output, job output, or `needs`
//! consumer that the current interface does not provide is an error: GitHub
//! evaluates it as an empty string, so a job condition would silently skip work.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use serde_json::{Map, Value};

use crate::repository::run_bounded;
use crate::{ActionError, Result, VERSION};

const OWNER: &str = "loadingalias/cargo-rail-action";
const MAX_LISTING_BYTES: usize = 16 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// Bundled interfaces keyed by the path after the repository name.
const METADATA: [(&str, &str); 6] = [
    ("", include_str!("../action.yaml")),
    ("setup", include_str!("../setup/action.yaml")),
    ("cache", include_str!("../cache/action.yaml")),
    ("cache/collect", include_str!("../cache/collect/action.yaml")),
    ("cache/report", include_str!("../cache/report/action.yaml")),
    ("release", include_str!("../release/action.yaml")),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Severity {
    Error,
    Warning,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Finding {
    pub(crate) file: String,
    pub(crate) line: usize,
    pub(crate) severity: Severity,
    pub(crate) message: String,
}

#[derive(Debug)]
pub(crate) struct Reference {
    file: String,
    line: usize,
    action: String,
    reference: String,
    immutable: bool,
    major: Option<u64>,
    job: String,
    inputs: Vec<String>,
}

#[derive(Debug, Default)]
pub(crate) struct Audit {
    pub(crate) references: Vec<Reference>,
    pub(crate) conditions: Vec<(String, usize, String, String)>,
    pub(crate) findings: Vec<Finding>,
}

impl Audit {
    pub(crate) fn errors(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == Severity::Error)
            .count()
    }

    /// Human inventory followed by one line per finding.
    pub(crate) fn render(&self) -> String {
        let files = self
            .references
            .iter()
            .map(|reference| reference.file.as_str())
            .collect::<BTreeSet<_>>();
        let mut lines = vec![format!(
            "Cargo-Rail Action references: {} in {} file{}",
            self.references.len(),
            files.len(),
            if files.len() == 1 { "" } else { "s" }
        )];
        for reference in &self.references {
            let version = reference
                .major
                .map_or_else(|| "unknown version".to_string(), |major| format!("v{major}"));
            let pin = if reference.immutable { "immutable" } else { "mutable" };
            let inputs = if reference.inputs.is_empty() {
                "none".to_string()
            } else {
                reference.inputs.join(", ")
            };
            lines.push(format!(
                "  {}:{}  {}@{} ({version}, {pin})  job {}  inputs: {inputs}",
                reference.file,
                reference.line,
                action_name(&reference.action),
                reference.reference,
                reference.job
            ));
        }
        if !self.conditions.is_empty() {
            lines.push("Job conditions that read Cargo-Rail outputs:".to_string());
            for (file, line, job, condition) in &self.conditions {
                lines.push(format!("  {file}:{line}  job {job}: {condition}"));
            }
        }
        for finding in &self.findings {
            let level = match finding.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            };
            lines.push(format!(
                "{}:{}: {level}: {}",
                finding.file, finding.line, finding.message
            ));
        }
        let warnings = self.findings.len() - self.errors();
        lines.push(format!("{} errors, {warnings} warnings", self.errors()));
        lines.push(String::new());
        lines.join("\n")
    }

    /// GitHub annotations for every finding, located at its file and line.
    pub(crate) fn annotations(&self) -> String {
        let mut output = String::new();
        for finding in &self.findings {
            let level = match finding.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            };
            let escape = |value: &str| value.replace('%', "%25").replace('\r', "%0D").replace('\n', "%0A");
            let property = |value: &str| escape(value).replace(':', "%3A").replace(',', "%2C");
            output.push_str(&format!(
                "::{level} file={},line={}::{}\n",
                property(&finding.file),
                finding.line,
                escape(&finding.message)
            ));
        }
        output
    }
}

fn action_name(action: &str) -> String {
    if action.is_empty() {
        OWNER.to_string()
    } else {
        format!("{OWNER}/{action}")
    }
}

struct Interface {
    inputs: BTreeSet<String>,
    outputs: BTreeSet<String>,
}

fn interfaces() -> BTreeMap<&'static str, Interface> {
    METADATA
        .iter()
        .map(|(action, metadata)| {
            let value: Value = serde_saphyr::from_str(metadata).expect("bundled action metadata is YAML");
            let keys = |field: &str| {
                value[field]
                    .as_object()
                    .map(|object| object.keys().cloned().collect())
                    .unwrap_or_default()
            };
            (
                *action,
                Interface {
                    inputs: keys("inputs"),
                    outputs: keys("outputs"),
                },
            )
        })
        .collect()
}

/// Audit every YAML file tracked or unignored below the repository root.
pub(crate) fn audit_repository(root: &Path) -> Result<Audit> {
    let mut command = Command::new("git");
    command.current_dir(root).args([
        "ls-files",
        "-z",
        "--cached",
        "--others",
        "--exclude-standard",
        "--",
        "*.yml",
        "*.yaml",
    ]);
    let listing = run_bounded(&mut command, MAX_LISTING_BYTES, MAX_LISTING_BYTES)?;
    if !listing.status.success() {
        return Err(ActionError::operational(format!(
            "cannot list repository YAML files: {}",
            String::from_utf8_lossy(&listing.stderr).trim()
        )));
    }
    let paths = listing
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect::<BTreeSet<_>>();
    let interfaces = interfaces();
    let mut audit = Audit::default();
    for path in paths {
        let full = root.join(&path);
        let Ok(metadata) = std::fs::metadata(&full) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        if metadata.len() > MAX_FILE_BYTES {
            audit.findings.push(Finding {
                file: path,
                line: 1,
                severity: Severity::Warning,
                message: format!("file exceeds {MAX_FILE_BYTES} bytes and was not audited"),
            });
            continue;
        }
        let text = std::fs::read_to_string(&full)
            .map_err(|error| ActionError::operational(format!("cannot read '{path}': {error}")))?;
        audit_file(&path, &text, &interfaces, &mut audit);
    }
    audit.findings.sort();
    Ok(audit)
}

fn audit_file(path: &str, text: &str, interfaces: &BTreeMap<&str, Interface>, audit: &mut Audit) {
    if !text.to_ascii_lowercase().contains(OWNER) {
        return;
    }
    let value = match serde_saphyr::from_str::<Value>(text) {
        Ok(value) => value,
        Err(error) => {
            audit.findings.push(Finding {
                file: path.to_string(),
                line: 1,
                severity: Severity::Error,
                message: format!("references {OWNER} but is not valid YAML, so it was not audited: {error}"),
            });
            return;
        }
    };
    let file = File { path, text };
    if let Some(jobs) = value.get("jobs").and_then(Value::as_object) {
        audit_workflow(&file, jobs, interfaces, audit);
    } else if let Some(steps) = value.pointer("/runs/steps").and_then(Value::as_array) {
        let job = Map::from_iter([("steps".to_string(), Value::Array(steps.clone()))]);
        let mut local = JobFacts::default();
        audit_job(&file, "(composite action)", &job, interfaces, audit, &mut local);
    } else {
        audit.findings.push(Finding {
            file: path.to_string(),
            line: file.line_of(OWNER, 0),
            severity: Severity::Warning,
            message: format!("mentions {OWNER} outside a workflow job or composite action step"),
        });
    }
}

struct File<'a> {
    path: &'a str,
    text: &'a str,
}

impl File<'_> {
    /// First line at or after `after` (zero-based) that contains `needle`.
    fn line_of(&self, needle: &str, after: usize) -> usize {
        self.text
            .lines()
            .enumerate()
            .skip(after)
            .find(|(_, line)| line.contains(needle))
            .or_else(|| self.text.lines().enumerate().find(|(_, line)| line.contains(needle)))
            .map_or(1, |(index, _)| index + 1)
    }

    fn job_line(&self, job: &str) -> usize {
        let key = format!("{job}:");
        self.text
            .lines()
            .position(|line| line.trim() == key || line.trim_start().starts_with(&format!("{key} ")))
            .unwrap_or(0)
    }
}

#[derive(Default)]
struct JobFacts {
    /// Step ID to action path for this job's Cargo-Rail steps.
    steps: BTreeMap<String, String>,
    /// Every job output name; outputs of other steps are exported too.
    exports: BTreeSet<String>,
    plans: bool,
}

fn audit_workflow(
    file: &File<'_>,
    jobs: &Map<String, Value>,
    interfaces: &BTreeMap<&str, Interface>,
    audit: &mut Audit,
) {
    let mut facts = BTreeMap::new();
    for (name, job) in jobs {
        let Some(job) = job.as_object() else { continue };
        let mut local = JobFacts::default();
        audit_job(file, name, job, interfaces, audit, &mut local);
        facts.insert(name.as_str(), local);
    }
    for (name, job) in jobs {
        let Some(job) = job.as_object() else { continue };
        let start = file.job_line(name);
        let strings = strings(&Value::Object(job.clone()));
        for (producer, output) in expressions(&strings, "needs.") {
            let Some(producer_facts) = facts.get(producer.as_str()) else {
                continue;
            };
            if producer_facts.steps.is_empty() {
                continue;
            }
            if !producer_facts.exports.contains(&output) {
                audit.findings.push(Finding {
                    file: file.path.to_string(),
                    line: file.line_of(&format!("needs.{producer}.outputs.{output}"), start),
                    severity: Severity::Error,
                    message: format!(
                        "job {name} reads needs.{producer}.outputs.{output}, which job {producer} does not export; \
                         GitHub evaluates it as empty"
                    ),
                });
            }
        }
        if let Some(condition) = job.get("if").and_then(Value::as_str)
            && expressions(&[condition.to_string()], "needs.")
                .iter()
                .any(|(producer, _)| {
                    facts
                        .get(producer.as_str())
                        .is_some_and(|facts| !facts.steps.is_empty())
                })
        {
            audit.conditions.push((
                file.path.to_string(),
                file.line_of("if:", start),
                name.clone(),
                condition.trim().to_string(),
            ));
        }
        let reads_plan = strings.iter().any(|value| value.contains("cargo-rail-action plan "));
        if reads_plan && !facts[name.as_str()].plans {
            for producer in needs(job) {
                let Some(producer_job) = jobs.get(&producer).and_then(Value::as_object) else {
                    continue;
                };
                if !facts.get(producer.as_str()).is_some_and(|facts| facts.plans) {
                    continue;
                }
                let (consumer, planner) = (runs_on(job), runs_on(producer_job));
                if consumer != planner {
                    audit.findings.push(Finding {
                        file: file.path.to_string(),
                        line: file.line_of("runs-on", start),
                        severity: Severity::Warning,
                        message: format!(
                            "job {name} reads a plan from job {producer}, which runs on {planner}, \
                             but this job runs on {consumer}; a plan authorizes selectors only on its own platform, \
                             so plan again in this job"
                        ),
                    });
                }
            }
        }
    }
}

fn audit_job(
    file: &File<'_>,
    name: &str,
    job: &Map<String, Value>,
    interfaces: &BTreeMap<&str, Interface>,
    audit: &mut Audit,
    facts: &mut JobFacts,
) {
    let start = file.job_line(name);
    if let Some(uses) = job.get("uses").and_then(Value::as_str)
        && uses.to_ascii_lowercase().contains(OWNER)
    {
        audit.findings.push(Finding {
            file: file.path.to_string(),
            line: file.line_of(uses, start),
            severity: Severity::Error,
            message: format!("job {name} calls {uses} as a reusable workflow; Cargo-Rail Action provides none"),
        });
    }
    let current = VERSION
        .split('.')
        .next()
        .and_then(|major| major.parse::<u64>().ok())
        .expect("package version has a numeric major");
    for step in job.get("steps").and_then(Value::as_array).into_iter().flatten() {
        let Some(uses) = step.get("uses").and_then(Value::as_str) else {
            continue;
        };
        let Some((action, reference)) = parse_uses(uses) else {
            continue;
        };
        let line = file.line_of(uses, start);
        let comment = file
            .text
            .lines()
            .nth(line - 1)
            .and_then(|text| text.split_once('#'))
            .map(|(_, comment)| comment.trim().to_string());
        let immutable = matches!(reference.len(), 40 | 64) && reference.bytes().all(|byte| byte.is_ascii_hexdigit());
        let major = if immutable {
            comment.as_deref().and_then(major_version)
        } else {
            major_version(&reference)
        };
        let inputs = step
            .get("with")
            .and_then(Value::as_object)
            .map(|with| with.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let finding = |severity, message: String| Finding {
            file: file.path.to_string(),
            line,
            severity,
            message,
        };
        match (major, interfaces.get(action.as_str())) {
            (_, None) => audit.findings.push(finding(
                Severity::Error,
                format!("{} is not an action this release provides", action_name(&action)),
            )),
            (Some(major), _) if major < current => audit.findings.push(finding(
                Severity::Error,
                format!(
                    "{}@{reference} is v{major}; migrate every Cargo-Rail Action reference to v{current} together",
                    action_name(&action)
                ),
            )),
            (Some(major), _) if major > current => audit.findings.push(finding(
                Severity::Warning,
                format!("v{major} is newer than this v{current} audit; run the matching release's audit"),
            )),
            (None, _) => audit.findings.push(finding(
                Severity::Warning,
                format!(
                    "cannot tell which release {} names; pin a commit with a trailing `# v{current}.x.y` comment \
                     or use a release tag",
                    action_name(&action)
                ),
            )),
            _ => {}
        }
        if let Some(interface) = interfaces.get(action.as_str()) {
            for input in &inputs {
                if !interface.inputs.contains(input) {
                    audit.findings.push(Finding {
                        file: file.path.to_string(),
                        line: file.line_of(&format!("{input}:"), line - 1),
                        severity: Severity::Error,
                        message: format!(
                            "{} has no input {input}; inputs are {}",
                            action_name(&action),
                            interface.inputs.iter().cloned().collect::<Vec<_>>().join(", ")
                        ),
                    });
                }
            }
        }
        if action.is_empty() {
            facts.plans = true;
        }
        if let Some(id) = step.get("id").and_then(Value::as_str) {
            facts.steps.insert(id.to_string(), action.clone());
        }
        audit.references.push(Reference {
            file: file.path.to_string(),
            line,
            action,
            reference,
            immutable,
            major,
            job: name.to_string(),
            inputs,
        });
    }
    if facts.steps.is_empty() {
        return;
    }
    let exports = job.get("outputs").and_then(Value::as_object);
    facts
        .exports
        .extend(exports.into_iter().flatten().map(|(export, _)| export.clone()));
    for (id, output) in expressions(&strings(&Value::Object(job.clone())), "steps.") {
        let Some(action) = facts.steps.get(&id) else { continue };
        let Some(interface) = interfaces.get(action.as_str()) else {
            continue;
        };
        if !interface.outputs.contains(&output) {
            audit.findings.push(Finding {
                file: file.path.to_string(),
                line: file.line_of(&format!("steps.{id}.outputs.{output}"), start),
                severity: Severity::Error,
                message: format!(
                    "{} has no output {output}; outputs are {}, so steps.{id}.outputs.{output} is always empty",
                    action_name(action),
                    interface.outputs.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
            });
        }
    }
    for (export, value) in job.get("outputs").and_then(Value::as_object).into_iter().flatten() {
        let Some(value) = value.as_str() else { continue };
        for (id, output) in expressions(&[value.to_string()], "steps.") {
            let Some(action) = facts.steps.get(&id) else { continue };
            if action.is_empty() && output == "plan-file" {
                audit.findings.push(Finding {
                    file: file.path.to_string(),
                    line: file.line_of(&format!("steps.{id}.outputs.plan-file"), start),
                    severity: Severity::Warning,
                    message: format!(
                        "job {name} exports plan-file as output {export}; it is a path on this runner only, \
                         so upload the file as an artifact instead"
                    ),
                });
            }
        }
    }
}

/// Split `owner/repo[/path]@ref` for this Action, case-insensitively.
fn parse_uses(uses: &str) -> Option<(String, String)> {
    let (target, reference) = uses.trim().split_once('@')?;
    let lower = target.to_ascii_lowercase();
    let action = lower.strip_prefix(OWNER)?;
    let action = if action.is_empty() {
        String::new()
    } else {
        action.strip_prefix('/')?.trim_end_matches('/').to_string()
    };
    Some((action, reference.to_string()))
}

fn major_version(value: &str) -> Option<u64> {
    let digits = value.trim().strip_prefix('v')?;
    let major = digits.split('.').next()?;
    major.parse().ok()
}

/// Every `prefix<name>.outputs.<output>` reference in the given strings.
fn expressions(values: &[String], prefix: &str) -> BTreeSet<(String, String)> {
    let identifier = |text: &str| {
        text.chars()
            .take_while(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
            .collect::<String>()
    };
    let mut found = BTreeSet::new();
    for value in values {
        for (index, _) in value.match_indices(prefix) {
            if value[..index]
                .chars()
                .next_back()
                .is_some_and(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
            {
                continue;
            }
            let rest = &value[index + prefix.len()..];
            let name = identifier(rest);
            let Some(rest) = rest[name.len()..].strip_prefix(".outputs.") else {
                continue;
            };
            let output = identifier(rest);
            if !name.is_empty() && !output.is_empty() {
                found.insert((name, output));
            }
        }
    }
    found
}

fn strings(value: &Value) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        match value {
            Value::String(text) => found.push(text.clone()),
            Value::Array(values) => stack.extend(values),
            Value::Object(object) => stack.extend(object.values()),
            _ => {}
        }
    }
    found
}

fn needs(job: &Map<String, Value>) -> Vec<String> {
    match job.get("needs") {
        Some(Value::String(name)) => vec![name.clone()],
        Some(Value::Array(names)) => names.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

fn runs_on(job: &Map<String, Value>) -> String {
    match job.get("runs-on") {
        Some(Value::String(label)) => label.clone(),
        Some(other) => other.to_string(),
        None => "an unspecified runner".to_string(),
    }
}

/// `cargo-rail-action audit`: print the inventory and fail on any error.
pub(crate) fn run_command(root: Option<&Path>) -> Result<()> {
    let root = match root {
        Some(root) => root.to_path_buf(),
        None => repository_root(Path::new("."))?,
    };
    let audit = audit_repository(&root)?;
    crate::write_stdout(audit.render().as_bytes())?;
    if audit.errors() > 0 {
        return Err(ActionError::rejected(format!(
            "workflow audit found {} error{}",
            audit.errors(),
            if audit.errors() == 1 { "" } else { "s" }
        )));
    }
    Ok(())
}

/// Audit before planning so a partial migration fails before any job runs.
pub(crate) fn audit_before_planning(workspace: &Path) -> Result<()> {
    let root = repository_root(workspace)?;
    let audit = audit_repository(&root)?;
    eprint!("{}", audit.render());
    eprint!("{}", audit.annotations());
    if audit.errors() > 0 {
        return Err(ActionError::rejected(format!(
            "workflow audit found {} error{}; fix every Cargo-Rail Action reference before planning",
            audit.errors(),
            if audit.errors() == 1 { "" } else { "s" }
        )));
    }
    Ok(())
}

fn repository_root(directory: &Path) -> Result<std::path::PathBuf> {
    let mut command = Command::new("git");
    command.current_dir(directory).args(["rev-parse", "--show-toplevel"]);
    let output = run_bounded(&mut command, 64 * 1024, 64 * 1024)?;
    if !output.status.success() {
        return Err(ActionError::operational(format!(
            "workflow audit requires a Git checkout: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(std::path::PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audit(files: &[(&str, &str)]) -> Audit {
        let interfaces = interfaces();
        let mut audit = Audit::default();
        for (path, text) in files {
            audit_file(path, text, &interfaces, &mut audit);
        }
        audit.findings.sort();
        audit
    }

    const CURRENT: &str = "\
on: pull_request
jobs:
  plan:
    runs-on: ubuntu-latest
    outputs:
      required-work: ${{ steps.rail.outputs.required-work }}
    steps:
      - uses: loadingalias/cargo-rail-action@v10
        id: rail
        with:
          since: origin/main
  test:
    needs: plan
    if: contains(fromJSON(needs.plan.outputs.required-work), 'cargo.test')
    runs-on: ubuntu-latest
    steps:
      - run: cargo test
";

    #[test]
    fn a_complete_current_migration_has_no_findings() {
        let audit = audit(&[(".github/workflows/ci.yml", CURRENT)]);
        assert!(audit.findings.is_empty(), "{:?}", audit.findings);
        assert_eq!(audit.references.len(), 1);
        assert_eq!(audit.conditions.len(), 1);
        let rendered = audit.render();
        assert!(rendered.contains(".github/workflows/ci.yml:8  loadingalias/cargo-rail-action@v10 (v10, mutable)"));
        assert!(rendered.contains("inputs: since"));
        assert!(rendered.contains("job test: contains(fromJSON(needs.plan.outputs.required-work), 'cargo.test')"));
    }

    #[test]
    fn every_planner_integration_is_found_including_unlisted_files() {
        let composite = "\
runs:
  using: composite
  steps:
    - uses: loadingalias/cargo-rail-action@v10
      id: rail
";
        let audit = audit(&[
            (".github/workflows/a.yml", CURRENT),
            (".github/workflows/b.yml", CURRENT),
            (".github/workflows/c.yaml", CURRENT),
            (".github/actions/route/action.yml", composite),
        ]);
        assert_eq!(audit.references.len(), 4, "{:?}", audit.references);
        assert!(audit.findings.is_empty(), "{:?}", audit.findings);
    }

    #[test]
    fn mixed_versions_fail_with_exact_locations() {
        let legacy = "\
jobs:
  plan:
    runs-on: ubuntu-latest
    steps:
      - uses: loadingalias/cargo-rail-action@3d3c42e5aac5ba805825da76410c181273ba90b1 # v6.1.2
        id: rail
";
        let audit = audit(&[
            (".github/workflows/ci.yml", CURRENT),
            (".github/workflows/old.yml", legacy),
        ]);
        assert_eq!(audit.errors(), 1, "{:?}", audit.findings);
        let finding = &audit.findings[0];
        assert_eq!((finding.file.as_str(), finding.line), (".github/workflows/old.yml", 5));
        assert!(
            finding
                .message
                .contains("is v6; migrate every Cargo-Rail Action reference to v10 together")
        );
        assert!(audit.render().contains("(v6, immutable)"));
    }

    #[test]
    fn legacy_boolean_consumers_fail_before_execution() {
        let workflow = "\
jobs:
  plan:
    runs-on: ubuntu-latest
    outputs:
      test: ${{ steps.rail.outputs.test }}
      required-work: ${{ steps.rail.outputs.required-work }}
    steps:
      - uses: loadingalias/cargo-rail-action@v10
        id: rail
      - if: steps.rail.outputs.docs == 'true'
        run: cargo doc
  test:
    needs: [plan]
    if: needs.plan.outputs.build == 'true' || needs.plan.outputs.test == 'true'
    runs-on: ubuntu-latest
    steps:
      - run: cargo test
";
        let audit = audit(&[(".github/workflows/ci.yml", workflow)]);
        let located = audit
            .findings
            .iter()
            .map(|finding| (finding.line, finding.message.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(audit.errors(), 3, "{located:?}");
        assert!(
            located
                .iter()
                .any(|(line, message)| *line == 5 && message.contains("has no output test"))
        );
        assert!(
            located
                .iter()
                .any(|(line, message)| *line == 10 && message.contains("has no output docs"))
        );
        assert!(located.iter().any(|(line, message)| *line == 14
            && message.contains("needs.plan.outputs.build, which job plan does not export")));
    }

    #[test]
    fn outputs_of_other_steps_remain_valid_exports() {
        let workflow = "\
jobs:
  plan:
    runs-on: ubuntu-latest
    outputs:
      host-tests: ${{ steps.scope.outputs.required }}
    steps:
      - uses: loadingalias/cargo-rail-action@v10
        id: rail
      - id: scope
        run: echo required=true >> \"$GITHUB_OUTPUT\"
  test:
    needs: plan
    if: needs.plan.outputs.host-tests == 'true'
    runs-on: ubuntu-latest
    steps:
      - run: cargo test
";
        let audit = audit(&[(".github/workflows/ci.yml", workflow)]);
        assert!(audit.findings.is_empty(), "{:?}", audit.findings);
    }

    #[test]
    fn unknown_inputs_and_actions_are_errors() {
        let workflow = "\
jobs:
  plan:
    runs-on: ubuntu-latest
    steps:
      - uses: loadingalias/cargo-rail-action@v10
        with:
          confidence-profile: strict
      - uses: loadingalias/cargo-rail-action/removed@v10
";
        let audit = audit(&[(".github/workflows/ci.yml", workflow)]);
        let messages = audit
            .findings
            .iter()
            .map(|finding| (finding.line, finding.message.clone()))
            .collect::<Vec<_>>();
        assert_eq!(audit.errors(), 2, "{messages:?}");
        assert!(
            messages
                .iter()
                .any(|(line, message)| *line == 7 && message.contains("has no input confidence-profile"))
        );
        assert!(
            messages
                .iter()
                .any(|(line, message)| *line == 8 && message.contains("removed is not an action"))
        );
    }

    #[test]
    fn plan_transfer_hazards_are_warnings() {
        let workflow = "\
jobs:
  plan:
    runs-on: ubuntu-latest
    outputs:
      plan: ${{ steps.rail.outputs.plan-file }}
      required-work: ${{ steps.rail.outputs.required-work }}
    steps:
      - uses: loadingalias/cargo-rail-action@v10
        id: rail
  macos:
    needs: plan
    runs-on: macos-latest
    steps:
      - uses: loadingalias/cargo-rail-action/setup@v10
      - run: cargo-rail-action plan cargo-args \"$PLAN_FILE\" cargo.test
  linux:
    needs: plan
    runs-on: ubuntu-latest
    steps:
      - run: cargo-rail-action plan cargo-args \"$PLAN_FILE\" cargo.test
  replanned:
    needs: plan
    runs-on: windows-latest
    steps:
      - uses: loadingalias/cargo-rail-action@v10
      - run: cargo-rail-action plan cargo-args \"$PLAN_FILE\" cargo.test
";
        let audit = audit(&[(".github/workflows/ci.yml", workflow)]);
        assert_eq!(audit.errors(), 0, "{:?}", audit.findings);
        let warnings = audit
            .findings
            .iter()
            .map(|finding| (finding.line, finding.message.clone()))
            .collect::<Vec<_>>();
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(
            warnings
                .iter()
                .any(|(line, message)| *line == 5 && message.contains("exports plan-file"))
        );
        assert!(warnings.iter().any(|(line, message)| *line == 12
            && message.contains("job macos reads a plan from job plan, which runs on ubuntu-latest")));
    }

    #[test]
    fn unparseable_references_are_never_skipped_silently() {
        let audit = audit(&[(
            ".github/workflows/broken.yml",
            "jobs: [\n  uses: loadingalias/cargo-rail-action@v10\n",
        )]);
        assert_eq!(audit.errors(), 1);
        assert!(audit.findings[0].message.contains("is not valid YAML"));
    }

    #[test]
    fn annotations_carry_file_and_line() {
        let audit = Audit {
            findings: vec![Finding {
                file: "a,b:c.yml".to_string(),
                line: 3,
                severity: Severity::Warning,
                message: "100%\nnext".to_string(),
            }],
            ..Audit::default()
        };
        assert_eq!(
            audit.annotations(),
            "::warning file=a%2Cb%3Ac.yml,line=3::100%25%0Anext\n"
        );
    }
}
