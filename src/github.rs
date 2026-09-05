use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::{ActionError, Result, env_os};

const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_DIAGNOSTIC_BYTES: usize = 8 * 1024;
const MAX_SUMMARY_BYTES: usize = 256 * 1024;

#[derive(Debug)]
pub(crate) struct Publication {
    pub(crate) summary: Option<String>,
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) outputs: Vec<(String, String)>,
}

#[derive(Debug)]
struct ControlFiles {
    output: ControlFile,
    path: ControlFile,
    summary: Option<ControlFile>,
}

#[derive(Debug)]
struct ControlFile {
    file: File,
}

impl ControlFiles {
    fn load(needs_summary: bool) -> Result<Self> {
        let output = validated_control_file("GITHUB_OUTPUT")?;
        let path = validated_control_file("GITHUB_PATH")?;
        let summary = if needs_summary {
            Some(validated_control_file("GITHUB_STEP_SUMMARY")?)
        } else {
            None
        };
        Ok(Self { output, path, summary })
    }
}

pub(crate) fn publish(publication: Publication) -> Result<()> {
    let needs_summary = publication.summary.is_some();
    let mut controls = ControlFiles::load(needs_summary)?;
    publish_to(publication, &mut controls)
}

pub(crate) fn publish_environment(values: &[(&str, &Path)]) -> Result<()> {
    let mut payload = String::new();
    for (name, path) in values {
        let value = path
            .to_str()
            .filter(|value| !value.contains(['\r', '\n']))
            .ok_or_else(|| ActionError::rejected("report environment path is invalid"))?;
        if !path.is_absolute() || !name.bytes().all(|byte| byte.is_ascii_uppercase() || byte == b'_') {
            return Err(ActionError::rejected(
                "report environment must use an absolute path and a constant variable name",
            ));
        }
        payload.push_str(&format!("{name}={value}\n"));
    }
    validated_control_file("GITHUB_ENV")?
        .append(payload.as_bytes())
        .map_err(ActionError::from)
}

pub(crate) fn publish_summary(summary: String) -> Result<()> {
    let summary = bounded_text(summary, MAX_SUMMARY_BYTES, "GitHub summary")?;
    validated_control_file("GITHUB_STEP_SUMMARY")?
        .append(summary.as_bytes())
        .map_err(ActionError::from)
}

fn publish_to(publication: Publication, controls: &mut ControlFiles) -> Result<()> {
    let summary = publication
        .summary
        .map(|value| bounded_text(value, MAX_SUMMARY_BYTES, "GitHub summary"))
        .transpose()?;

    let mut path_payload = Vec::new();
    for path in publication.paths {
        if !path.is_absolute() {
            return Err(ActionError::rejected(format!(
                "PATH publication must use an absolute directory: {}",
                path.display()
            )));
        }
        let text = path
            .to_str()
            .ok_or_else(|| ActionError::rejected("PATH publication contains non-UTF-8 data"))?;
        if text.contains(['\r', '\n']) {
            return Err(ActionError::rejected("PATH publication contains a line break"));
        }
        path_payload.extend_from_slice(text.as_bytes());
        path_payload.push(b'\n');
    }

    let mut output_payload = Vec::new();
    for (name, value) in publication.outputs {
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(ActionError::rejected(format!("invalid GitHub output name {name:?}")));
        }
        if value.contains(['\r', '\n']) {
            return Err(ActionError::rejected(format!(
                "GitHub output {name} must be single-line"
            )));
        }
        if value.len() > MAX_OUTPUT_BYTES {
            return Err(ActionError::rejected(format!(
                "GitHub output {name} exceeds the 256 KiB value bound"
            )));
        }
        output_payload.extend_from_slice(name.as_bytes());
        output_payload.push(b'=');
        output_payload.extend_from_slice(value.as_bytes());
        output_payload.push(b'\n');
    }

    let mut attempted_any = false;
    let result = (|| {
        if let (Some(summary_file), Some(summary_payload)) = (&mut controls.summary, summary.as_deref()) {
            attempted_any = true;
            summary_file.append(summary_payload.as_bytes())?;
        }
        if !path_payload.is_empty() {
            attempted_any = true;
            controls.path.append(&path_payload)?;
        }
        if !output_payload.is_empty() {
            attempted_any = true;
            controls.output.append(&output_payload)?;
        }
        Ok(())
    })();
    result.map_err(|error: std::io::Error| {
        let suffix = if attempted_any {
            "; a partial summary or environment-file write may remain, but no output from this failed step is valid authority"
        } else {
            ""
        };
        ActionError::operational(format!("cannot publish GitHub action result: {error}{suffix}"))
    })
}

fn validated_control_file(name: &str) -> Result<ControlFile> {
    let path = PathBuf::from(env_os(name)?);
    open_control_file(name, &path)
}

fn open_control_file(name: &str, path: &Path) -> Result<ControlFile> {
    if !path.is_absolute() {
        return Err(ActionError::rejected(format!(
            "{name} must name an absolute runner-control file"
        )));
    }
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| ActionError::operational(format!("cannot inspect {name} '{}': {error}", path.display())))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(ActionError::rejected(format!(
            "{name} must name an existing regular non-symbolic runner-control file"
        )));
    }
    let opened = open_append_handle(path)
        .map_err(|error| ActionError::operational(format!("cannot open {name} '{}': {error}", path.display())))?;
    let opened_metadata = opened
        .metadata()
        .map_err(|error| ActionError::operational(format!("cannot verify {name} '{}': {error}", path.display())))?;
    if !opened_control_file_is_valid(&metadata, &opened_metadata) {
        return Err(ActionError::rejected(format!("{name} changed before publication")));
    }
    Ok(ControlFile { file: opened })
}

impl ControlFile {
    fn append(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.file.write_all(bytes)?;
        self.file.flush()
    }
}

#[cfg(unix)]
fn open_append_handle(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().append(true).open(path)
}

#[cfg(windows)]
fn open_append_handle(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt as _;

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    // Keep the captured pathname attached to this handle and inspect a
    // reparse point itself instead of following it to another write target.
    OpenOptions::new()
        .append(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn open_append_handle(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().append(true).open(path)
}

#[cfg(unix)]
fn opened_control_file_is_valid(path: &std::fs::Metadata, opened: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;

    opened.is_file() && path.dev() == opened.dev() && path.ino() == opened.ino()
}

#[cfg(windows)]
fn opened_control_file_is_valid(_path: &std::fs::Metadata, opened: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

    opened.is_file() && opened.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

#[cfg(not(any(unix, windows)))]
fn opened_control_file_is_valid(_path: &std::fs::Metadata, _opened: &std::fs::Metadata) -> bool {
    false
}

fn bounded_text(value: String, maximum: usize, subject: &str) -> Result<String> {
    if value.len() > maximum {
        return Err(ActionError::rejected(format!(
            "{subject} exceeds its {maximum}-byte bound"
        )));
    }
    Ok(value)
}

pub(crate) fn markdown_inline(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('|', "&#124;")
        .replace('`', "&#96;")
        .replace('\\', "&#92;")
        .replace('*', "&#42;")
        .replace('_', "&#95;")
        .replace('[', "&#91;")
        .replace(']', "&#93;")
        .replace('~', "&#126;")
        .replace(['\r', '\n'], " ")
}

fn redact(mut message: String) -> String {
    for name in ["INPUT_REPOSITORY_TOKEN", "INPUT_REMOTE", "GH_TOKEN"] {
        if let Ok(secret) = std::env::var(name)
            && !secret.is_empty()
        {
            message = message.replace(&secret, "[REDACTED]");
        }
    }
    message
}

fn truncate_utf8(value: &str, maximum: usize) -> &str {
    if value.len() <= maximum {
        return value;
    }
    let mut end = maximum;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

pub(crate) fn emit_error(message: &str) {
    let redacted = redact(message.to_string());
    let escaped = redacted.replace('%', "%25").replace('\r', "%0D").replace('\n', "%0A");
    let bounded = truncate_utf8(&escaped, MAX_DIAGNOSTIC_BYTES);
    eprintln!("::error::{bounded}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temporary_directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cargo-rail-action-github-test.{}.{}",
            std::process::id(),
            TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("temporary directory");
        path
    }

    #[test]
    fn hostile_text_cannot_create_markdown_structure() {
        assert_eq!(markdown_inline("`x`|a\nb"), "&#96;x&#96;&#124;a b");
        assert_eq!(
            markdown_inline("**[name](url)** _text_ ~text~ \\"),
            "&#42;&#42;&#91;name&#93;(url)&#42;&#42; &#95;text&#95; &#126;text&#126; &#92;"
        );
    }

    #[test]
    fn truncation_keeps_utf8_valid() {
        assert_eq!(truncate_utf8("aé", 2), "a");
        let escaped = "%".repeat(MAX_DIAGNOSTIC_BYTES).replace('%', "%25");
        assert_eq!(
            truncate_utf8(&escaped, MAX_DIAGNOSTIC_BYTES).len(),
            MAX_DIAGNOSTIC_BYTES
        );
    }

    #[test]
    fn output_failure_reports_prior_non_authoritative_writes() {
        let directory = temporary_directory();
        let summary = directory.join("summary");
        let path = directory.join("path");
        let output = directory.join("output");
        fs::write(&summary, []).expect("summary file");
        fs::write(&path, []).expect("path file");
        fs::write(&output, []).expect("output file");
        let mut controls = ControlFiles {
            output: ControlFile {
                file: File::open(&output).expect("read-only output handle"),
            },
            path: ControlFile {
                file: OpenOptions::new().append(true).open(&path).expect("path handle"),
            },
            summary: Some(ControlFile {
                file: OpenOptions::new().append(true).open(&summary).expect("summary handle"),
            }),
        };
        let error = publish_to(
            Publication {
                summary: Some("summary\n".to_string()),
                paths: vec![directory.clone()],
                outputs: vec![("version".to_string(), "0.26.0".to_string())],
            },
            &mut controls,
        )
        .expect_err("read-only output handle must fail");
        assert!(
            error
                .to_string()
                .contains("partial summary or environment-file write may remain")
        );
        assert_eq!(fs::read_to_string(summary).expect("summary contents"), "summary\n");
        assert_eq!(
            fs::read_to_string(path).expect("path contents"),
            format!("{}\n", directory.display())
        );
        assert_eq!(fs::read(output).expect("output contents"), b"");
        fs::remove_dir_all(directory).expect("remove fixture");
    }

    #[cfg(unix)]
    #[test]
    fn publication_stays_bound_to_the_validated_control_file() {
        let directory = temporary_directory();
        let output = directory.join("output");
        let original = directory.join("original-output");
        fs::write(&output, []).expect("output file");
        let mut control = open_control_file("GITHUB_OUTPUT", &output).expect("validated handle");
        fs::rename(&output, &original).expect("move validated file");
        fs::write(&output, b"replacement\n").expect("replacement file");

        control.append(b"authority=yes\n").expect("append original handle");

        assert_eq!(fs::read(&output).expect("replacement contents"), b"replacement\n");
        assert_eq!(fs::read(&original).expect("original contents"), b"authority=yes\n");
        fs::remove_dir_all(directory).expect("remove fixture");
    }
}
