#![forbid(unsafe_code)]

mod cache;
mod github;
mod install;
mod plan;
mod release;
mod report;
mod repository;

use std::ffi::OsString;
use std::fmt;
use std::io::Write as _;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_RUNTIME_BYTES: u64 = 32 * 1024 * 1024;
const QUALIFIED_TARGETS: [&str; 3] = [
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorKind {
    Operational,
    Rejected,
}

#[derive(Debug)]
struct ActionError {
    kind: ErrorKind,
    message: String,
}

impl ActionError {
    fn operational(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Operational,
            message: message.into(),
        }
    }

    fn rejected(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Rejected,
            message: message.into(),
        }
    }

    fn exit_code(&self) -> i32 {
        match self.kind {
            ErrorKind::Operational => 1,
            ErrorKind::Rejected => 2,
        }
    }
}

impl fmt::Display for ActionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ActionError {}

impl From<std::io::Error> for ActionError {
    fn from(error: std::io::Error) -> Self {
        Self::operational(error.to_string())
    }
}

type Result<T> = std::result::Result<T, ActionError>;

#[derive(Debug, Parser)]
#[command(name = "cargo-rail-action", version, disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(hide = true)]
    SelfCheck(SelfCheck),
    #[command(hide = true)]
    Run {
        #[command(subcommand)]
        operation: RunOperation,
    },
    /// Read a validated Cargo-Rail plan.
    Plan {
        #[command(subcommand)]
        operation: PlanOperation,
    },
    #[command(hide = true)]
    Release {
        #[command(subcommand)]
        operation: ReleaseOperation,
    },
}

#[derive(Debug, Args)]
struct SelfCheck {
    #[arg(long)]
    expect_version: String,
    #[arg(long)]
    expect_target: String,
}

#[derive(Debug, Subcommand)]
enum RunOperation {
    Planner,
    Cache,
    Setup,
    CacheCollect,
    CacheReport,
}

#[derive(Debug, Subcommand)]
enum PlanOperation {
    /// Render the saved plan as readable Markdown, with dependency details expandable.
    Summary(PlanPath),
    /// Print required work IDs as one JSON array.
    Required(PlanPath),
    /// Print true or false for one registered work item.
    IsRequired(PlanWork),
    /// Emit NUL-delimited Cargo arguments; skipped work emits nothing.
    CargoArgs(PlanWork),
    /// Print skipped, workspace, or packages for Cargo work.
    CargoScope(PlanWork),
    /// Emit NUL-delimited package names; skipped or workspace scope emits nothing.
    PackageNames(PlanWork),
    /// Emit NUL-delimited target arguments; no target restriction emits nothing.
    TargetArgs(PlanWork),
    /// Print a JSON include matrix, or all when every declared variant is required.
    Matrix(MatrixArgs),
}

#[derive(Debug, Args)]
struct PlanPath {
    /// Validated plan JSON file from the planner action.
    plan: PathBuf,
}

#[derive(Debug, Args)]
struct PlanWork {
    /// Validated plan JSON file from the planner action.
    plan: PathBuf,
    /// Registered work ID, such as cargo.test.
    work: String,
}

#[derive(Debug, Args)]
struct MatrixArgs {
    /// Validated plan JSON file from the planner action.
    plan: PathBuf,
    /// Registered work ID with variant scope.
    work: String,
    /// Restrict rows to this family and wrap each row under its name.
    #[arg(long)]
    family: Option<String>,
}

#[derive(Debug, Subcommand)]
enum ReleaseOperation {
    Preflight,
    Intent(release::IntentArgs),
    Publish(release::EffectArgs),
    Promote(release::EffectArgs),
}

fn build_target() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
    {
        return "x86_64-unknown-linux-gnu";
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return "aarch64-apple-darwin";
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64", target_env = "msvc"))]
    {
        return "x86_64-pc-windows-msvc";
    }
    #[allow(unreachable_code)]
    "unsupported-build-target"
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Command::SelfCheck(arguments) => {
            if arguments.expect_version != VERSION {
                return Err(ActionError::rejected(format!(
                    "runtime version is {VERSION}, expected {}",
                    arguments.expect_version
                )));
            }
            if arguments.expect_target != build_target() {
                return Err(ActionError::rejected(format!(
                    "runtime target is {}, expected {}",
                    build_target(),
                    arguments.expect_target
                )));
            }
            Ok(())
        }
        Command::Run { operation } => match operation {
            RunOperation::Planner => repository::run_planner(),
            RunOperation::Cache => cache::run_action(),
            RunOperation::Setup => install::run_setup_action(),
            RunOperation::CacheCollect => report::collect_action(),
            RunOperation::CacheReport => report::report_action(),
        },
        Command::Plan { operation } => plan::run_command(operation),
        Command::Release { operation } => match operation {
            ReleaseOperation::Preflight => release::preflight(),
            ReleaseOperation::Intent(arguments) => release::write_intent(&arguments),
            ReleaseOperation::Publish(arguments) => release::publish(&arguments),
            ReleaseOperation::Promote(arguments) => release::promote(&arguments),
        },
    }
}

fn write_stdout(bytes: &[u8]) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(bytes)
        .and_then(|()| stdout.flush())
        .map_err(|error| ActionError::operational(format!("cannot write stdout: {error}")))
}

fn env_os(name: &str) -> Result<OsString> {
    std::env::var_os(name).ok_or_else(|| ActionError::rejected(format!("{name} is required")))
}

fn env_string(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| ActionError::rejected(format!("{name} must be valid UTF-8 and is required")))
}

fn optional_env(name: &str) -> Result<String> {
    match std::env::var(name) {
        Ok(value) => Ok(value),
        Err(std::env::VarError::NotPresent) => Ok(String::new()),
        Err(std::env::VarError::NotUnicode(_)) => Err(ActionError::rejected(format!("{name} must be valid UTF-8"))),
    }
}

fn main() {
    if let Err(error) = run() {
        github::emit_error(&error.to_string());
        std::process::exit(error.exit_code());
    }
}
