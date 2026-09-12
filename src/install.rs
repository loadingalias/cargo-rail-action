use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write as _};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use rscrypto::Sha256;

use crate::github::{Publication, publish};
use crate::repository::{create_private_directory, run_bounded, subprocess_failure};
use crate::{ActionError, MAX_RUNTIME_BYTES, QUALIFIED_TARGETS, Result, build_target, env_string};

const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_CHECKSUM_BYTES: u64 = 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 128;
const MAX_COMPONENT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_SUBPROCESS_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComponentSet {
    Core,
    Cache,
    Surface,
    Complete,
}

impl ComponentSet {
    pub(crate) fn planner(value: &str) -> Result<Self> {
        match value {
            "core" => Ok(Self::Core),
            "surface" => Ok(Self::Surface),
            "complete" => Ok(Self::Complete),
            _ => Err(ActionError::rejected("components must be core, surface, or complete")),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Cache => "cache",
            Self::Surface => "surface",
            Self::Complete => "complete",
        }
    }

    pub(crate) fn needs_surface(self) -> bool {
        matches!(self, Self::Surface | Self::Complete)
    }

    fn accepts(self, capability: &str) -> bool {
        match self {
            Self::Core => capability == "core",
            Self::Cache => matches!(capability, "core" | "cache" | "surface" | "surface-source"),
            Self::Surface => matches!(capability, "core" | "analysis" | "surface" | "surface-source"),
            Self::Complete => matches!(
                capability,
                "core" | "analysis" | "cache" | "distributed" | "surface" | "surface-source"
            ),
        }
    }

    fn required_counts(self) -> BTreeMap<&'static str, usize> {
        let mut counts = BTreeMap::from([("core", 1)]);
        match self {
            Self::Core => {}
            Self::Cache => {
                counts.extend([("cache", 2), ("surface", 1), ("surface-source", 1)]);
            }
            Self::Surface => {
                counts.extend([("analysis", 1), ("surface", 1), ("surface-source", 1)]);
            }
            Self::Complete => {
                counts.extend([
                    ("analysis", 1),
                    ("cache", 2),
                    ("distributed", 1),
                    ("surface", 1),
                    ("surface-source", 1),
                ]);
            }
        }
        counts
    }
}

#[derive(Debug)]
pub(crate) struct InstalledCargoRail {
    directory: PathBuf,
    binary: PathBuf,
}

impl InstalledCargoRail {
    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    pub(crate) fn binary(&self) -> &Path {
        &self.binary
    }
}

#[derive(Debug)]
struct ManifestEntry {
    name: String,
    digest: String,
    bytes: u64,
    capability: String,
}

#[derive(Debug)]
struct ComponentManifest {
    bytes: Vec<u8>,
    entries: Vec<ManifestEntry>,
}

#[derive(Debug)]
struct ArchiveLayout {
    component_directory: String,
    manifest: ComponentManifest,
}

pub(crate) fn validate_cargo_rail_version(value: &str) -> Result<()> {
    let components = value.split('.').collect::<Vec<_>>();
    let valid_numeric = components.len() == 3
        && components.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (part.len() == 1 || !part.starts_with('0'))
        });
    if !valid_numeric || components[0] != "0" || components[1] != "26" {
        return Err(ActionError::rejected(
            "version must be an exact stable Cargo-Rail 0.26 patch release",
        ));
    }
    Ok(())
}

pub(crate) fn install_cargo_rail(version: &str, component_set: ComponentSet) -> Result<InstalledCargoRail> {
    validate_cargo_rail_version(version)?;
    let target = build_target();
    if !QUALIFIED_TARGETS.contains(&target) {
        return Err(ActionError::rejected(
            "this operating system, architecture, or libc is not advertised by Cargo-Rail Action v9",
        ));
    }
    let install_base = installation_base()?;
    let target_root = install_base.join(version).join(target);
    std::fs::create_dir_all(&target_root).map_err(|error| {
        ActionError::operational(format!(
            "cannot create Cargo-Rail installation root '{}': {error}",
            target_root.display()
        ))
    })?;
    validate_real_directory(&target_root, &install_base)?;
    if let Some(installed) = reusable_installation(&target_root, version, target, component_set)? {
        return Ok(installed);
    }

    let runner_temp = canonical_runner_directory("RUNNER_TEMP")?;
    let temporary = TemporaryDirectory::new(&runner_temp, "cargo-rail-download")?;
    let archive_name = cargo_rail_archive_name(target);
    let release_root = format!("https://github.com/loadingalias/cargo-rail/releases/download/v{version}");
    let checksum_path = temporary.path().join("SHA256SUMS");
    download(
        &format!("{release_root}/SHA256SUMS"),
        &checksum_path,
        MAX_CHECKSUM_BYTES,
        "Cargo-Rail release checksum manifest",
    )?;
    let archive_path = temporary.path().join(&archive_name);
    download(
        &format!("{release_root}/{archive_name}"),
        &archive_path,
        MAX_ARCHIVE_BYTES,
        "Cargo-Rail release archive",
    )?;
    let expected_archive_digest = checksum_for(&checksum_path, &archive_name)?;
    let actual_archive_digest = digest_file(&archive_path, MAX_ARCHIVE_BYTES)?;
    if actual_archive_digest != expected_archive_digest {
        return Err(ActionError::rejected(format!("checksum mismatch for {archive_name}")));
    }

    let layout = inspect_archive(&archive_path, version, target, component_set)?;
    let manifest_digest = hex_digest(&layout.manifest.bytes);
    let destination = target_root.join(format!("{}-{manifest_digest}", component_set.as_str()));
    if destination.exists() {
        return verify_installation(&destination, version, target, component_set, &manifest_digest);
    }

    let stage = TemporaryDirectory::new(&target_root, &format!(".{}-stage", component_set.as_str()))?;
    extract_selected(&archive_path, &layout, component_set, stage.path())?;
    write_receipt(
        stage.path(),
        version,
        target,
        component_set,
        &manifest_digest,
        &layout.manifest.entries,
    )?;
    verify_installation(stage.path(), version, target, component_set, &manifest_digest)?;
    let publication_lock = target_root.join(format!(
        ".{}-{manifest_digest}.publication-lock",
        component_set.as_str()
    ));
    let lock = match PublicationLock::acquire(&publication_lock)? {
        Some(lock) => lock,
        None => {
            for _ in 0..100 {
                if destination.exists() {
                    return verify_installation(&destination, version, target, component_set, &manifest_digest);
                }
                if !publication_lock.exists() {
                    return Err(ActionError::operational(
                        "concurrent Cargo-Rail installation ended without publishing its immutable destination",
                    ));
                }
                thread::sleep(Duration::from_millis(50));
            }
            return Err(ActionError::operational(format!(
                "concurrent Cargo-Rail installation did not finish; remove stale lock '{}' only after proving no installer is active",
                publication_lock.display()
            )));
        }
    };
    if destination.exists() {
        return verify_installation(&destination, version, target, component_set, &manifest_digest);
    }
    std::fs::rename(stage.path(), &destination).map_err(|error| {
        ActionError::operational(format!(
            "cannot publish immutable Cargo-Rail installation '{}': {error}",
            destination.display()
        ))
    })?;
    stage.keep();
    drop(lock);
    verify_installation(&destination, version, target, component_set, &manifest_digest)
}

pub(crate) fn cargo_rail_archive_name(target: &str) -> String {
    format!("cargo-rail-{target}.zip")
}

pub(crate) fn run_setup_action() -> Result<()> {
    let version = env_string("INPUT_VERSION")?;
    validate_cargo_rail_version(&version)?;
    let installed = install_cargo_rail(&version, ComponentSet::Core)?;
    publish(Publication {
        summary: None,
        paths: vec![runtime_directory()?, installed.directory().to_path_buf()],
        outputs: vec![("version".to_string(), version.clone())],
    })?;
    println!("Cargo-Rail setup ready: {version}");
    Ok(())
}

pub(crate) fn runtime_directory() -> Result<PathBuf> {
    let executable = std::env::current_exe()
        .map_err(|error| ActionError::operational(format!("cannot locate action runtime: {error}")))?;
    verify_runtime_directory(&executable)
}

fn verify_runtime_directory(executable: &Path) -> Result<PathBuf> {
    let directory = executable
        .parent()
        .ok_or_else(|| ActionError::operational("action runtime has no installation directory"))?;
    let directory_metadata = std::fs::symlink_metadata(directory)
        .map_err(|error| ActionError::operational(format!("cannot inspect action runtime directory: {error}")))?;
    if !directory_metadata.is_dir() || directory_metadata.file_type().is_symlink() {
        return Err(ActionError::rejected(
            "action runtime directory is not a real directory",
        ));
    }
    let expected_name = format!(
        "cargo-rail-action-{}{}",
        build_target(),
        if cfg!(windows) { ".exe" } else { "" }
    );
    if executable.file_name().and_then(OsStr::to_str) != Some(expected_name.as_str()) {
        return Err(ActionError::rejected("action runtime executable name is invalid"));
    }
    let prefix = format!("{}-", build_target());
    let expected_digest = directory
        .file_name()
        .and_then(OsStr::to_str)
        .and_then(|name| name.strip_prefix(&prefix))
        .filter(|digest| valid_digest(digest))
        .ok_or_else(|| ActionError::rejected("action runtime directory identity is invalid"))?;
    let mut entries = std::fs::read_dir(directory)
        .map_err(|error| ActionError::operational(format!("cannot inspect action runtime inventory: {error}")))?;
    let only_entry = entries
        .next()
        .transpose()
        .map_err(ActionError::from)?
        .ok_or_else(|| ActionError::rejected("action runtime directory is empty"))?;
    let has_another_entry = entries.next().transpose().map_err(ActionError::from)?.is_some();
    if only_entry.path() != executable || has_another_entry {
        return Err(ActionError::rejected(
            "action runtime directory must contain only its authenticated executable",
        ));
    }
    if digest_file(executable, MAX_RUNTIME_BYTES)? != expected_digest {
        return Err(ActionError::rejected("action runtime executable identity changed"));
    }
    Ok(directory.to_path_buf())
}

fn installation_base() -> Result<PathBuf> {
    let root = match std::env::var_os("RUNNER_TOOL_CACHE") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => PathBuf::from(env_string("RUNNER_TEMP")?),
    };
    let canonical = std::fs::canonicalize(&root)
        .map_err(|error| ActionError::operational(format!("cannot resolve runner installation root: {error}")))?;
    if !canonical.is_dir() {
        return Err(ActionError::rejected(
            "runner installation root must be an existing directory",
        ));
    }
    let base = canonical.join("cargo-rail-action").join("cargo-rail");
    std::fs::create_dir_all(&base)
        .map_err(|error| ActionError::operational(format!("cannot create action installation root: {error}")))?;
    let resolved = std::fs::canonicalize(&base)
        .map_err(|error| ActionError::operational(format!("cannot resolve action installation root: {error}")))?;
    if !resolved.starts_with(&canonical) {
        return Err(ActionError::rejected(
            "action installation root escapes the runner-controlled directory",
        ));
    }
    Ok(resolved)
}

fn reusable_installation(
    target_root: &Path,
    version: &str,
    target: &str,
    component_set: ComponentSet,
) -> Result<Option<InstalledCargoRail>> {
    let prefix = format!("{}-", component_set.as_str());
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(target_root)
        .map_err(|error| ActionError::operational(format!("cannot inspect installation root: {error}")))?
    {
        let entry = entry.map_err(|error| {
            ActionError::operational(format!("cannot inspect an immutable installation entry: {error}"))
        })?;
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            candidates.push(entry.path());
        }
    }
    candidates.sort();
    if candidates.len() > 1 {
        return Err(ActionError::rejected(format!(
            "multiple immutable {} installations exist under '{}'; remove the stale directories explicitly",
            component_set.as_str(),
            target_root.display()
        )));
    }
    let Some(candidate) = candidates.pop() else {
        return Ok(None);
    };
    let name = candidate
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| ActionError::rejected("immutable installation directory name is not UTF-8"))?;
    let digest = name
        .strip_prefix(&prefix)
        .filter(|value| valid_digest(value))
        .ok_or_else(|| ActionError::rejected("immutable installation directory has an invalid manifest identity"))?;
    verify_installation(&candidate, version, target, component_set, digest).map(Some)
}

fn verify_installation(
    directory: &Path,
    version: &str,
    target: &str,
    component_set: ComponentSet,
    manifest_digest: &str,
) -> Result<InstalledCargoRail> {
    let metadata = std::fs::symlink_metadata(directory).map_err(|error| {
        ActionError::operational(format!(
            "cannot inspect immutable installation '{}': {error}",
            directory.display()
        ))
    })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return corrupt_installation(directory, "destination is not a real directory");
    }
    let receipt_path = directory.join("cargo-rail-action-install-v1.tsv");
    let receipt = read_bounded_file(&receipt_path, MAX_MANIFEST_BYTES, "installation receipt")?;
    let receipt_text =
        std::str::from_utf8(&receipt).map_err(|_| ActionError::rejected("installation receipt is not UTF-8"))?;
    if receipt_text.contains('\r') || !receipt_text.ends_with('\n') {
        return corrupt_installation(directory, "receipt is not canonical LF-only text");
    }
    let mut lines = receipt_text.lines();
    let expected_header = format!(
        "cargo-rail-action-installed-v1\t{version}\t{target}\t{}\t{manifest_digest}",
        component_set.as_str()
    );
    if lines.next() != Some(expected_header.as_str()) {
        return corrupt_installation(directory, "receipt authority is incompatible");
    }
    let mut entries = Vec::new();
    for line in lines {
        entries.push(parse_manifest_entry(line, "installation receipt")?);
    }
    if entries.is_empty() {
        return corrupt_installation(directory, "receipt has no components");
    }
    validate_selected_entries(&entries, target, component_set)?;
    let expected_names = entries.iter().map(|entry| entry.name.as_str()).collect::<BTreeSet<_>>();
    let mut actual_names = std::fs::read_dir(directory)
        .map_err(|error| ActionError::operational(format!("cannot inspect immutable installation: {error}")))?
        .map(|entry| {
            entry
                .map_err(ActionError::from)?
                .file_name()
                .into_string()
                .map_err(|_| ActionError::rejected("immutable installation contains a non-UTF-8 name"))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    actual_names.remove("cargo-rail-action-install-v1.tsv");
    if actual_names.iter().map(String::as_str).collect::<BTreeSet<_>>() != expected_names {
        return corrupt_installation(directory, "directory inventory disagrees with its receipt");
    }
    for entry in &entries {
        let path = directory.join(&entry.name);
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|_| ActionError::rejected(format!("installed component {} is missing", entry.name)))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != entry.bytes {
            return corrupt_installation(directory, &format!("component {} is missing or changed", entry.name));
        }
        if digest_file(&path, entry.bytes)? != entry.digest {
            return corrupt_installation(directory, &format!("component {} digest changed", entry.name));
        }
    }
    let binary = directory.join(cargo_rail_name(target));
    verify_binary_version(&binary, version)?;
    Ok(InstalledCargoRail {
        directory: directory.to_path_buf(),
        binary,
    })
}

fn corrupt_installation<T>(directory: &Path, detail: &str) -> Result<T> {
    Err(ActionError::rejected(format!(
        "immutable Cargo-Rail installation '{}' is corrupt ({detail}); remove that exact directory and rerun",
        directory.display()
    )))
}

fn verify_binary_version(binary: &Path, version: &str) -> Result<()> {
    let mut command = Command::new(binary);
    command.args(["rail", "--version"]);
    let output = run_bounded(&mut command, MAX_SUBPROCESS_BYTES, MAX_SUBPROCESS_BYTES)?;
    if !output.status.success() {
        return Err(subprocess_failure("installed Cargo-Rail self-check", &output));
    }
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| ActionError::rejected("installed Cargo-Rail version output is not UTF-8"))?;
    if text.trim() != format!("cargo-rail {version}") {
        return Err(ActionError::rejected(format!(
            "installed Cargo-Rail version disagrees with requested {version}"
        )));
    }
    Ok(())
}

fn download(url: &str, destination: &Path, maximum: u64, subject: &str) -> Result<()> {
    let mut command = Command::new("curl");
    command
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--retry",
            "3",
            "--proto",
            "=https",
            "--tlsv1.2",
            "--max-filesize",
            &maximum.to_string(),
            "--output",
        ])
        .arg(destination)
        .arg(url);
    let output = run_bounded(&mut command, 0, MAX_SUBPROCESS_BYTES)?;
    if !output.status.success() {
        return Err(subprocess_failure(subject, &output));
    }
    let metadata = std::fs::symlink_metadata(destination)
        .map_err(|error| ActionError::operational(format!("cannot inspect downloaded {subject}: {error}")))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > maximum {
        return Err(ActionError::rejected(format!(
            "downloaded {subject} exceeds its authenticated file bound"
        )));
    }
    Ok(())
}

fn checksum_for(path: &Path, archive_name: &str) -> Result<String> {
    let bytes = read_bounded_file(path, MAX_CHECKSUM_BYTES, "release checksum manifest")?;
    let text =
        std::str::from_utf8(&bytes).map_err(|_| ActionError::rejected("release checksum manifest is not UTF-8"))?;
    if text.contains('\r') {
        return Err(ActionError::rejected(
            "release checksum manifest must use LF line endings",
        ));
    }
    let mut selected = Vec::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let mut fields = line.split_ascii_whitespace();
        let digest = fields.next().unwrap_or_default();
        let name = fields.next().unwrap_or_default().trim_start_matches('*');
        if fields.next().is_some() || !valid_digest(&digest.to_ascii_lowercase()) || name.contains(['/', '\\']) {
            return Err(ActionError::rejected(
                "release checksum manifest contains an invalid row",
            ));
        }
        if name == archive_name {
            selected.push(digest.to_ascii_lowercase());
        }
    }
    if selected.len() != 1 {
        return Err(ActionError::rejected(format!(
            "release checksum manifest must contain exactly one entry for {archive_name}"
        )));
    }
    Ok(selected.remove(0))
}

fn inspect_archive(path: &Path, version: &str, target: &str, component_set: ComponentSet) -> Result<ArchiveLayout> {
    let (entries, manifests) = inspect_zip(path)?;
    if manifests.len() != 1 {
        return Err(ActionError::rejected(
            "Cargo-Rail archive must contain exactly one cargo-rail-components-v1.tsv",
        ));
    }
    let (manifest_path, manifest_bytes) = manifests.into_iter().next().expect("one manifest");
    let component_directory = Path::new(&manifest_path)
        .parent()
        .and_then(Path::to_str)
        .unwrap_or_default()
        .replace('\\', "/");
    let manifest = parse_component_manifest(manifest_bytes, version, target)?;
    validate_manifest_inventory(&manifest.entries, target, component_set)?;
    let mut declared = BTreeMap::from([(manifest_path, manifest.bytes.len() as u64)]);
    for entry in &manifest.entries {
        let archive_path = if component_directory.is_empty() {
            entry.name.clone()
        } else {
            format!("{component_directory}/{}", entry.name)
        };
        declared.insert(archive_path, entry.bytes);
    }
    if entries != declared {
        return Err(ActionError::rejected(
            "Cargo-Rail archive inventory disagrees with its component manifest",
        ));
    }
    Ok(ArchiveLayout {
        component_directory,
        manifest,
    })
}

type InspectedEntries = (BTreeMap<String, u64>, Vec<(String, Vec<u8>)>);

fn inspect_zip(path: &Path) -> Result<InspectedEntries> {
    let file = File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| ActionError::rejected(format!("cannot inspect Cargo-Rail ZIP archive: {error}")))?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(ActionError::rejected("Cargo-Rail archive exceeds the 128-entry bound"));
    }
    let mut files = BTreeMap::new();
    let mut manifests = Vec::new();
    let mut names = BTreeSet::new();
    let mut expanded = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| ActionError::rejected(format!("cannot inspect Cargo-Rail ZIP entry: {error}")))?;
        let name = safe_archive_name(Path::new(entry.name()))?;
        if !names.insert(name.clone()) {
            return Err(ActionError::rejected(format!(
                "Cargo-Rail archive contains duplicate entry {name}"
            )));
        }
        if entry.is_dir() {
            validate_zip_entry_compression(&name, true, entry.compression())?;
            continue;
        }
        let unix_kind = entry.unix_mode().map(|mode| mode & 0o170000).unwrap_or(0);
        if !entry.is_file() || !matches!(unix_kind, 0 | 0o100000) {
            return Err(ActionError::rejected(format!(
                "Cargo-Rail ZIP contains a non-regular entry: {name}"
            )));
        }
        validate_zip_entry_compression(&name, false, entry.compression())?;
        let size = entry.size();
        if size > MAX_COMPONENT_BYTES {
            return Err(ActionError::rejected(format!(
                "Cargo-Rail archive entry {name} exceeds its file bound"
            )));
        }
        expanded = expanded
            .checked_add(size)
            .filter(|total| *total <= MAX_EXPANDED_BYTES)
            .ok_or_else(|| ActionError::rejected("Cargo-Rail archive exceeds its expanded byte bound"))?;
        files.insert(name.clone(), size);
        if Path::new(&name).file_name() == Some(OsStr::new("cargo-rail-components-v1.tsv")) {
            if size > MAX_MANIFEST_BYTES {
                return Err(ActionError::rejected("Cargo-Rail component manifest exceeds 64 KiB"));
            }
            let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
            entry.read_to_end(&mut bytes).map_err(|error| {
                ActionError::rejected(format!("cannot read Cargo-Rail component manifest: {error}"))
            })?;
            manifests.push((name, bytes));
        }
    }
    Ok((files, manifests))
}

fn validate_zip_entry_compression(name: &str, directory: bool, compression: zip::CompressionMethod) -> Result<()> {
    let expected = if directory {
        zip::CompressionMethod::Stored
    } else {
        zip::CompressionMethod::Deflated
    };
    if compression != expected {
        return Err(ActionError::rejected(format!(
            "Cargo-Rail ZIP entry {name} does not use the exact v0.26 compression contract"
        )));
    }
    Ok(())
}

fn safe_archive_name(path: &Path) -> Result<String> {
    if path.is_absolute() {
        return Err(ActionError::rejected("archive entry path must be relative"));
    }
    let text = path
        .to_str()
        .ok_or_else(|| ActionError::rejected("archive entry path is not UTF-8"))?;
    if text.is_empty() || text.contains('\\') || text.as_bytes().contains(&0) {
        return Err(ActionError::rejected("archive entry path is malformed"));
    }
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ActionError::rejected(format!("archive entry path is unsafe: {text}")));
    }
    let mut normalized = String::new();
    for component in path.components() {
        let Component::Normal(component) = component else {
            return Err(ActionError::rejected(format!("archive entry path is unsafe: {text}")));
        };
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(
            component
                .to_str()
                .ok_or_else(|| ActionError::rejected("archive entry path is not UTF-8"))?,
        );
    }
    Ok(normalized)
}

fn parse_component_manifest(bytes: Vec<u8>, version: &str, target: &str) -> Result<ComponentManifest> {
    if bytes.len() as u64 > MAX_MANIFEST_BYTES || bytes.contains(&b'\r') || !bytes.ends_with(b"\n") {
        return Err(ActionError::rejected(
            "Cargo-Rail component manifest is not canonical LF-only text",
        ));
    }
    let text =
        std::str::from_utf8(&bytes).map_err(|_| ActionError::rejected("Cargo-Rail component manifest is not UTF-8"))?;
    if !text.is_ascii() {
        return Err(ActionError::rejected("Cargo-Rail component manifest must be ASCII"));
    }
    let mut lines = text.lines();
    let expected = format!("cargo-rail-components-v1\t{version}\t{target}");
    if lines.next() != Some(expected.as_str()) {
        return Err(ActionError::rejected(
            "Cargo-Rail component manifest authority is incompatible",
        ));
    }
    let mut entries = Vec::new();
    let mut names = BTreeSet::new();
    for line in lines {
        let entry = parse_manifest_entry(line, "Cargo-Rail component manifest")?;
        if !names.insert(entry.name.clone()) {
            return Err(ActionError::rejected(
                "Cargo-Rail component manifest contains duplicate names",
            ));
        }
        entries.push(entry);
    }
    if entries.is_empty() {
        return Err(ActionError::rejected("Cargo-Rail component manifest has no entries"));
    }
    Ok(ComponentManifest { bytes, entries })
}

fn parse_manifest_entry(line: &str, subject: &str) -> Result<ManifestEntry> {
    let mut fields = line.split('\t');
    let (Some(name), Some(digest), Some(size), Some(capability)) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return Err(ActionError::rejected(format!("{subject} contains an invalid row")));
    };
    if fields.next().is_some() {
        return Err(ActionError::rejected(format!("{subject} contains an invalid row")));
    }
    if name.is_empty()
        || name.starts_with('.')
        || name.contains(['/', '\\'])
        || !valid_digest(digest)
        || size.is_empty()
        || !size.bytes().all(|byte| byte.is_ascii_digit())
        || (size.len() > 1 && size.starts_with('0'))
        || capability.is_empty()
    {
        return Err(ActionError::rejected(format!(
            "{subject} contains invalid component authority"
        )));
    }
    let bytes = size
        .parse::<u64>()
        .map_err(|_| ActionError::rejected(format!("{subject} component size is invalid")))?;
    if bytes > MAX_COMPONENT_BYTES {
        return Err(ActionError::rejected(format!(
            "{subject} component size exceeds its bound"
        )));
    }
    Ok(ManifestEntry {
        name: name.to_string(),
        digest: digest.to_string(),
        bytes,
        capability: capability.to_string(),
    })
}

fn validate_manifest_inventory(entries: &[ManifestEntry], target: &str, requested: ComponentSet) -> Result<()> {
    let expected = expected_component_names(target);
    let mut counts = BTreeMap::<&str, usize>::new();
    for entry in entries {
        if expected.get(entry.name.as_str()) != Some(&entry.capability.as_str()) {
            return Err(ActionError::rejected(format!(
                "Cargo-Rail component {} has an unexpected name or capability",
                entry.name
            )));
        }
        *counts.entry(&entry.capability).or_default() += 1;
    }
    for (capability, count) in requested.required_counts() {
        if counts.get(capability).copied().unwrap_or(0) != count {
            return Err(ActionError::rejected(format!(
                "Cargo-Rail archive does not provide the complete {} component set",
                requested.as_str()
            )));
        }
    }
    Ok(())
}

fn validate_selected_entries(entries: &[ManifestEntry], target: &str, requested: ComponentSet) -> Result<()> {
    validate_manifest_inventory(entries, target, requested)?;
    let names = entries.iter().map(|entry| entry.name.as_str()).collect::<BTreeSet<_>>();
    if names.len() != entries.len() {
        return Err(ActionError::rejected(
            "installation receipt contains duplicate component names",
        ));
    }
    if entries.iter().any(|entry| !requested.accepts(&entry.capability)) {
        return Err(ActionError::rejected(
            "installation receipt contains an unrequested capability",
        ));
    }
    Ok(())
}

fn expected_component_names(target: &str) -> BTreeMap<&'static str, &'static str> {
    let windows = target.ends_with("windows-msvc");
    if windows {
        BTreeMap::from([
            ("cargo-rail.exe", "core"),
            ("cargo-rail-compiler-observation.exe", "analysis"),
            ("cargo-rail-native-rustc-wrapper.exe", "cache"),
            ("cargo-rail-native-rustc-worker.exe", "cache"),
            ("cargo-rail-distributed-worker.exe", "distributed"),
            ("cargo-rail-fact-driver.exe", "surface"),
            ("cargo-rail-fact-driver-source-v1.json", "surface-source"),
        ])
    } else {
        BTreeMap::from([
            ("cargo-rail", "core"),
            ("cargo-rail-compiler-observation", "analysis"),
            ("cargo-rail-native-rustc-wrapper", "cache"),
            ("cargo-rail-native-rustc-worker", "cache"),
            ("cargo-rail-distributed-worker", "distributed"),
            ("cargo-rail-fact-driver", "surface"),
            ("cargo-rail-fact-driver-source-v1.json", "surface-source"),
        ])
    }
}

fn extract_selected(
    archive_path: &Path,
    layout: &ArchiveLayout,
    component_set: ComponentSet,
    destination: &Path,
) -> Result<()> {
    let selected = layout
        .manifest
        .entries
        .iter()
        .filter(|entry| component_set.accepts(&entry.capability))
        .map(|entry| (entry.name.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let prefix = if layout.component_directory.is_empty() {
        String::new()
    } else {
        format!("{}/", layout.component_directory)
    };
    let file = File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| ActionError::rejected(format!("cannot reopen Cargo-Rail ZIP archive: {error}")))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| ActionError::rejected(format!("cannot read Cargo-Rail ZIP entry: {error}")))?;
        if !entry.is_file() {
            continue;
        }
        let name = safe_archive_name(Path::new(entry.name()))?;
        if let Some(component) = name.strip_prefix(&prefix).and_then(|name| selected.get(name).copied()) {
            write_component(&mut entry, destination, component)?;
        }
    }
    for component in selected.values() {
        if !destination.join(&component.name).is_file() {
            return Err(ActionError::rejected(format!(
                "selected component {} was not extracted",
                component.name
            )));
        }
    }
    Ok(())
}

fn write_component(reader: &mut impl Read, destination: &Path, component: &ManifestEntry) -> Result<()> {
    let path = destination.join(&component.name);
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| ActionError::operational(format!("cannot stage component {}: {error}", component.name)))?;
    let copied = std::io::copy(&mut reader.take(component.bytes + 1), &mut output)
        .map_err(|error| ActionError::operational(format!("cannot extract component {}: {error}", component.name)))?;
    output.flush()?;
    if copied != component.bytes {
        return Err(ActionError::rejected(format!(
            "component {} size changed during extraction",
            component.name
        )));
    }
    if digest_file(&path, component.bytes)? != component.digest {
        return Err(ActionError::rejected(format!(
            "component {} digest changed during extraction",
            component.name
        )));
    }
    set_component_permissions(&path, component.capability == "surface-source")
}

fn write_receipt(
    directory: &Path,
    version: &str,
    target: &str,
    component_set: ComponentSet,
    manifest_digest: &str,
    entries: &[ManifestEntry],
) -> Result<()> {
    let mut receipt = format!(
        "cargo-rail-action-installed-v1\t{version}\t{target}\t{}\t{manifest_digest}\n",
        component_set.as_str()
    );
    for entry in entries.iter().filter(|entry| component_set.accepts(&entry.capability)) {
        receipt.push_str(&format!(
            "{}\t{}\t{}\t{}\n",
            entry.name, entry.digest, entry.bytes, entry.capability
        ));
    }
    let path = directory.join("cargo-rail-action-install-v1.tsv");
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .and_then(|mut file| file.write_all(receipt.as_bytes()).and_then(|()| file.flush()))
        .map_err(|error| ActionError::operational(format!("cannot write installation receipt: {error}")))?;
    set_component_permissions(&path, true)
}

fn digest_file(path: &Path, maximum: u64) -> Result<String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| ActionError::operational(format!("cannot inspect '{}': {error}", path.display())))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > maximum {
        return Err(ActionError::rejected(format!(
            "'{}' is not a bounded regular file",
            path.display()
        )));
    }
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(read).unwrap_or(u64::MAX))
            .ok_or_else(|| ActionError::rejected("file size overflow while hashing"))?;
        if total > maximum {
            return Err(ActionError::rejected("file grew beyond its bound while hashing"));
        }
        hasher.update(&buffer[..read]);
    }
    if total != metadata.len() {
        return Err(ActionError::rejected("file changed while it was hashed"));
    }
    Ok(hex_bytes(&hasher.finalize()))
}

fn read_bounded_file(path: &Path, maximum: u64, subject: &str) -> Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| ActionError::operational(format!("cannot inspect {subject}: {error}")))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > maximum {
        return Err(ActionError::rejected(format!(
            "{subject} is not a bounded regular file"
        )));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    File::open(path)?.take(maximum + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum {
        return Err(ActionError::rejected(format!("{subject} exceeds its byte bound")));
    }
    Ok(bytes)
}

fn hex_digest(bytes: &[u8]) -> String {
    hex_bytes(&Sha256::digest(bytes))
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn cargo_rail_name(target: &str) -> &'static str {
    if target.ends_with("windows-msvc") {
        "cargo-rail.exe"
    } else {
        "cargo-rail"
    }
}

fn validate_real_directory(path: &Path, root: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    let canonical = std::fs::canonicalize(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || !canonical.starts_with(root) {
        return Err(ActionError::rejected(
            "installation directory is linked or escapes runner authority",
        ));
    }
    Ok(())
}

fn canonical_runner_directory(name: &str) -> Result<PathBuf> {
    let path = PathBuf::from(env_string(name)?);
    let canonical = std::fs::canonicalize(&path)
        .map_err(|error| ActionError::operational(format!("cannot resolve {name}: {error}")))?;
    if !canonical.is_dir() {
        return Err(ActionError::rejected(format!("{name} must name an existing directory")));
    }
    Ok(canonical)
}

#[cfg(unix)]
fn set_component_permissions(path: &Path, private_data: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = if private_data { 0o600 } else { 0o700 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(ActionError::from)
}

#[cfg(not(unix))]
fn set_component_permissions(_path: &Path, _private_data: bool) -> Result<()> {
    Ok(())
}

#[derive(Debug)]
struct TemporaryDirectory {
    path: PathBuf,
    retained: bool,
}

#[derive(Debug)]
struct PublicationLock {
    path: PathBuf,
}

impl PublicationLock {
    fn acquire(path: &Path) -> Result<Option<Self>> {
        match std::fs::create_dir(path) {
            Ok(()) => Ok(Some(Self {
                path: path.to_path_buf(),
            })),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
            Err(error) => Err(ActionError::operational(format!(
                "cannot acquire Cargo-Rail installation publication lock '{}': {error}",
                path.display()
            ))),
        }
    }
}

impl Drop for PublicationLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.path);
    }
}

impl TemporaryDirectory {
    fn new(parent: &Path, prefix: &str) -> Result<Self> {
        Ok(Self {
            path: create_private_directory(parent, prefix)?,
            retained: false,
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn keep(mut self) {
        self.retained = true;
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        if !self.retained {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component_manifest(target: &str, name: &str, contents: &[u8]) -> Vec<u8> {
        format!(
            "cargo-rail-components-v1\t0.26.0\t{target}\n{name}\t{}\t{}\tcore\n",
            hex_digest(contents),
            contents.len()
        )
        .into_bytes()
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        write_zip_with_compression(path, entries, zip::CompressionMethod::Deflated);
    }

    fn write_zip_with_compression(path: &Path, entries: &[(&str, &[u8])], compression: zip::CompressionMethod) {
        let mut archive = zip::ZipWriter::new(File::create(path).expect("archive"));
        let options = zip::write::SimpleFileOptions::default().compression_method(compression);
        for (name, contents) in entries {
            archive.start_file(name, options).expect("archive entry");
            archive.write_all(contents).expect("entry bytes");
        }
        archive.finish().expect("finish zip");
    }

    #[test]
    fn version_contract_accepts_only_stable_026_patches() {
        assert!(validate_cargo_rail_version("0.26.0").is_ok());
        assert!(validate_cargo_rail_version("0.26.19").is_ok());
        for rejected in ["0.25.9", "0.27.0", "0.26.0-rc.1", "0.26.0+build", "00.26.0"] {
            assert!(validate_cargo_rail_version(rejected).is_err(), "accepted {rejected}");
        }
    }

    #[test]
    fn qualified_cargo_rail_archives_use_one_deflated_zip_contract() {
        assert_eq!(
            QUALIFIED_TARGETS.map(cargo_rail_archive_name),
            [
                "cargo-rail-aarch64-apple-darwin.zip".to_string(),
                "cargo-rail-x86_64-pc-windows-msvc.zip".to_string(),
                "cargo-rail-x86_64-unknown-linux-gnu.zip".to_string(),
            ]
        );
        assert!(validate_zip_entry_compression("cargo-rail", false, zip::CompressionMethod::Deflated).is_ok());
        assert!(validate_zip_entry_compression("cargo-rail", false, zip::CompressionMethod::Stored).is_err());
        assert!(validate_zip_entry_compression("components/", true, zip::CompressionMethod::Stored).is_ok());
        assert!(validate_zip_entry_compression("components/", true, zip::CompressionMethod::Deflated).is_err());
    }

    #[test]
    fn stored_zip_regular_entries_are_rejected() {
        let temporary = TemporaryDirectory::new(&std::env::temp_dir(), "cargo-rail-action-stored-zip-test")
            .expect("temporary directory");
        let archive_path = temporary.path().join("cargo-rail.zip");
        write_zip_with_compression(
            &archive_path,
            &[("bundle/cargo-rail", b"uncompressed component")],
            zip::CompressionMethod::Stored,
        );

        assert!(
            inspect_zip(&archive_path)
                .expect_err("stored regular entry must fail")
                .to_string()
                .contains("compression contract")
        );
    }

    #[test]
    fn manifest_rows_are_pathless_and_canonical() {
        let digest = "a".repeat(64);
        let entry = parse_manifest_entry(&format!("cargo-rail\t{digest}\t12\tcore"), "fixture").expect("valid entry");
        assert_eq!(entry.name, "cargo-rail");
        assert_eq!(entry.digest, digest);
        assert_eq!(entry.bytes, 12);
        assert_eq!(entry.capability, "core");
        assert!(parse_manifest_entry(&format!("../cargo-rail\t{digest}\t12\tcore"), "fixture").is_err());
        assert!(parse_manifest_entry(&format!("cargo-rail\t{digest}\t012\tcore"), "fixture").is_err());
    }

    #[test]
    fn publication_lock_has_one_owner() {
        let temporary =
            TemporaryDirectory::new(&std::env::temp_dir(), "cargo-rail-action-lock-test").expect("temporary directory");
        let path = temporary.path().join("publication-lock");
        let first = PublicationLock::acquire(&path).expect("first lock").expect("owner");
        assert!(PublicationLock::acquire(&path).expect("contender").is_none());
        drop(first);
        assert!(PublicationLock::acquire(&path).expect("reacquire").is_some());
    }

    #[test]
    fn core_archive_extracts_exact_component_for_each_target() {
        let temporary = TemporaryDirectory::new(&std::env::temp_dir(), "cargo-rail-action-core-zip-test")
            .expect("temporary directory");
        for (target, name) in [
            ("aarch64-apple-darwin", "cargo-rail"),
            ("x86_64-unknown-linux-gnu", "cargo-rail"),
            ("x86_64-pc-windows-msvc", "cargo-rail.exe"),
        ] {
            let archive_path = temporary.path().join("cargo-rail.zip");
            let component = b"authenticated component";
            let manifest = component_manifest(target, name, component);
            write_zip(
                &archive_path,
                &[
                    (&format!("bundle/{name}"), component),
                    ("bundle/cargo-rail-components-v1.tsv", &manifest),
                ],
            );
            let layout = inspect_archive(&archive_path, "0.26.0", target, ComponentSet::Core).expect("inspect archive");
            let extracted = temporary.path().join(target);
            std::fs::create_dir(&extracted).expect("extract directory");
            extract_selected(&archive_path, &layout, ComponentSet::Core, &extracted).expect("extract component");
            let files = std::fs::read_dir(&extracted)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<Vec<_>>();
            assert_eq!(files, [std::ffi::OsString::from(name)]);
            assert_eq!(std::fs::read(extracted.join(name)).unwrap(), component);
        }
    }

    #[test]
    fn cache_archives_and_receipts_require_the_driver_and_source() {
        let temporary = TemporaryDirectory::new(&std::env::temp_dir(), "cargo-rail-action-cache-test").unwrap();
        for (target, extension) in [("aarch64-apple-darwin", ""), ("x86_64-pc-windows-msvc", ".exe")] {
            let components = [
                (format!("cargo-rail{extension}"), "core"),
                (format!("cargo-rail-native-rustc-wrapper{extension}"), "cache"),
                (format!("cargo-rail-native-rustc-worker{extension}"), "cache"),
                (format!("cargo-rail-fact-driver{extension}"), "surface"),
                ("cargo-rail-fact-driver-source-v1.json".into(), "surface-source"),
            ];
            let contents = b"authenticated component";
            let mut manifest = format!("cargo-rail-components-v1\t0.26.0\t{target}\n");
            for (name, capability) in &components {
                manifest.push_str(&format!(
                    "{name}\t{}\t{}\t{capability}\n",
                    hex_digest(contents),
                    contents.len()
                ));
            }
            let archive_path = temporary.path().join(format!("{target}.zip"));
            let paths = components
                .iter()
                .map(|(name, _)| format!("bundle/{name}"))
                .collect::<Vec<_>>();
            let mut entries = paths
                .iter()
                .map(|path| (path.as_str(), contents.as_slice()))
                .collect::<Vec<_>>();
            entries.push(("bundle/cargo-rail-components-v1.tsv", manifest.as_bytes()));
            write_zip(&archive_path, &entries);
            let layout = inspect_archive(&archive_path, "0.26.0", target, ComponentSet::Cache).unwrap();
            let extracted = temporary.path().join(target);
            std::fs::create_dir(&extracted).unwrap();
            extract_selected(&archive_path, &layout, ComponentSet::Cache, &extracted).unwrap();
            let inventory = std::fs::read_dir(&extracted)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .collect::<BTreeSet<_>>();
            assert_eq!(inventory, components.iter().map(|(name, _)| name.clone()).collect());
            for (name, _) in &components {
                assert_eq!(std::fs::read(extracted.join(name)).unwrap(), contents);
            }
            write_receipt(
                &extracted,
                "0.26.0",
                target,
                ComponentSet::Cache,
                &hex_digest(manifest.as_bytes()),
                &layout.manifest.entries,
            )
            .unwrap();
            let receipt = std::fs::read_to_string(extracted.join("cargo-rail-action-install-v1.tsv")).unwrap();
            let rows = receipt
                .lines()
                .skip(1)
                .map(|line| parse_manifest_entry(line, "receipt").unwrap())
                .collect::<Vec<_>>();
            validate_selected_entries(&rows, target, ComponentSet::Cache).unwrap();
            assert_eq!(rows.len(), 5);
            for missing in ["surface", "surface-source"] {
                let incomplete = rows
                    .iter()
                    .filter(|entry| entry.capability != missing)
                    .map(|entry| ManifestEntry {
                        name: entry.name.clone(),
                        digest: entry.digest.clone(),
                        bytes: entry.bytes,
                        capability: entry.capability.clone(),
                    })
                    .collect::<Vec<_>>();
                assert!(
                    validate_selected_entries(&incomplete, target, ComponentSet::Cache)
                        .unwrap_err()
                        .to_string()
                        .contains("complete cache component set")
                );
                let incomplete_manifest = manifest
                    .lines()
                    .filter(|line| !line.ends_with(&format!("\t{missing}")))
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n";
                let mut incomplete_entries = paths
                    .iter()
                    .zip(&components)
                    .filter(|(_, (_, capability))| *capability != missing)
                    .map(|(path, _)| (path.as_str(), contents.as_slice()))
                    .collect::<Vec<_>>();
                incomplete_entries.push(("bundle/cargo-rail-components-v1.tsv", incomplete_manifest.as_bytes()));
                write_zip(&archive_path, &incomplete_entries);
                assert!(
                    inspect_archive(&archive_path, "0.26.0", target, ComponentSet::Cache)
                        .unwrap_err()
                        .to_string()
                        .contains("complete cache component set")
                );
            }
        }
    }

    #[test]
    #[ignore = "requires a Cargo-Rail release archive built for this host"]
    fn cache_release_archive_installs_and_revalidates_actual_components() {
        let archive = PathBuf::from(std::env::var_os("CARGO_RAIL_TEST_RELEASE_ARCHIVE").expect("release archive"));
        let version = std::env::var("CARGO_RAIL_TEST_RELEASE_VERSION").expect("release version");
        let target = build_target();
        let layout = inspect_archive(&archive, &version, target, ComponentSet::Cache).unwrap();
        let temporary = TemporaryDirectory::new(&std::env::temp_dir(), "cargo-rail-action-release-test").unwrap();
        extract_selected(&archive, &layout, ComponentSet::Cache, temporary.path()).unwrap();
        let digest = hex_digest(&layout.manifest.bytes);
        write_receipt(
            temporary.path(),
            &version,
            target,
            ComponentSet::Cache,
            &digest,
            &layout.manifest.entries,
        )
        .unwrap();
        let installed = verify_installation(temporary.path(), &version, target, ComponentSet::Cache, &digest).unwrap();
        assert_eq!(installed.binary(), temporary.path().join(cargo_rail_name(target)));
        for name in [
            if target.ends_with("windows-msvc") {
                "cargo-rail-fact-driver.exe"
            } else {
                "cargo-rail-fact-driver"
            },
            "cargo-rail-fact-driver-source-v1.json",
        ] {
            let path = temporary.path().join(name);
            let bytes = std::fs::read(&path).unwrap();
            let mut changed = bytes.clone();
            changed[0] ^= 1;
            std::fs::write(&path, changed).unwrap();
            assert!(
                verify_installation(temporary.path(), &version, target, ComponentSet::Cache, &digest)
                    .unwrap_err()
                    .to_string()
                    .contains("digest changed")
            );
            std::fs::write(path, bytes).unwrap();
        }
        verify_installation(temporary.path(), &version, target, ComponentSet::Cache, &digest).unwrap();
    }

    #[test]
    fn zip_symlink_is_rejected_even_when_unselected() {
        let temporary = TemporaryDirectory::new(&std::env::temp_dir(), "cargo-rail-action-zip-link-test")
            .expect("temporary directory");
        let archive_path = temporary.path().join("cargo-rail.zip");
        let mut archive = zip::ZipWriter::new(File::create(&archive_path).expect("archive"));
        archive
            .add_symlink("bundle/link", "../../escape", zip::write::SimpleFileOptions::default())
            .expect("symlink entry");
        archive.finish().expect("finish zip");
        assert!(
            inspect_zip(&archive_path)
                .expect_err("symlink must fail")
                .to_string()
                .contains("non-regular")
        );
    }

    #[test]
    fn undeclared_archive_file_is_rejected_before_extraction() {
        let temporary = TemporaryDirectory::new(&std::env::temp_dir(), "cargo-rail-action-extra-test")
            .expect("temporary directory");
        let archive_path = temporary.path().join("cargo-rail.zip");
        let component = b"authenticated component";
        let manifest = component_manifest("x86_64-unknown-linux-gnu", "cargo-rail", component);
        write_zip(
            &archive_path,
            &[
                ("bundle/cargo-rail", component),
                ("bundle/undeclared", b"extra"),
                ("bundle/cargo-rail-components-v1.tsv", &manifest),
            ],
        );

        assert!(
            inspect_archive(&archive_path, "0.26.0", "x86_64-unknown-linux-gnu", ComponentSet::Core,)
                .expect_err("undeclared file must fail")
                .to_string()
                .contains("inventory")
        );
    }

    #[test]
    fn runtime_path_authority_requires_one_authenticated_executable() {
        let temporary = TemporaryDirectory::new(&std::env::temp_dir(), "cargo-rail-action-runtime-path-test")
            .expect("temporary directory");
        let runtime = b"authenticated action runtime";
        let digest = hex_bytes(&Sha256::digest(runtime));
        let directory = temporary.path().join(format!("{}-{digest}", build_target()));
        std::fs::create_dir(&directory).expect("runtime directory");
        let executable = directory.join(format!(
            "cargo-rail-action-{}{}",
            build_target(),
            if cfg!(windows) { ".exe" } else { "" }
        ));
        std::fs::write(&executable, runtime).expect("runtime executable");

        assert_eq!(
            verify_runtime_directory(&executable).expect("exact runtime inventory"),
            directory
        );

        std::fs::write(directory.join("cargo"), b"unexpected executable").expect("extra executable");
        assert!(
            verify_runtime_directory(&executable)
                .expect_err("extra runtime entry must fail")
                .to_string()
                .contains("only its authenticated executable")
        );

        std::fs::remove_file(directory.join("cargo")).expect("remove extra executable");
        std::fs::write(&executable, b"changed action runtime").expect("change runtime executable");
        assert!(
            verify_runtime_directory(&executable)
                .expect_err("changed runtime must fail")
                .to_string()
                .contains("identity changed")
        );
    }
}
