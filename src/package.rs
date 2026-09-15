//! Product runtime manifest generation; release policy belongs to Cargo-Rail.
use crate::{ActionError, MAX_RUNTIME_BYTES, QUALIFIED_TARGETS, Result, VERSION};
use clap::Args;
use rscrypto::Sha256;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
};
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_RELEASE_RUNTIME_BYTES: u64 = 16 * 1024 * 1024;
#[derive(Debug, Args)]
pub(crate) struct PackageArgs {
    #[arg(long, required = true)]
    asset: Vec<PathBuf>,
    #[arg(long)]
    output: PathBuf,
}
#[derive(Debug)]
struct AssetRecord {
    name: String,
    bytes: u64,
    sha256: String,
}

pub(crate) fn package(args: &PackageArgs) -> Result<()> {
    let assets = asset_records(&args.asset)?;
    let bytes = generate_runtime_manifest(&assets)?;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args.output)?
        .write_all(&bytes)?;
    Ok(())
}
fn asset_records(paths: &[PathBuf]) -> Result<Vec<AssetRecord>> {
    let mut records = Vec::with_capacity(paths.len());
    for path in paths {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|name| valid_asset_name(name))
            .ok_or_else(|| ActionError::rejected("release asset must have one pathless UTF-8 name"))?;
        let maximum = if matches!(name, "cargo-rail-action-runtime-v1.tsv" | "LICENSE") {
            MAX_MANIFEST_BYTES
        } else {
            MAX_RUNTIME_BYTES
        };
        let bytes = read_bounded(path, maximum, "release asset")?;
        if name == "LICENSE" && bytes != crate::LICENSE_BYTES {
            return Err(ActionError::rejected(
                "release LICENSE does not match the runtime source license",
            ));
        }
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
    if by_name.len() != targets.len() + 1
        || !by_name.get("LICENSE").is_some_and(|asset| {
            asset.bytes == crate::LICENSE_BYTES.len() as u64 && asset.sha256 == digest_bytes(crate::LICENSE_BYTES)
        })
        || targets.iter().any(|target| !by_name.contains_key(runtime_name(target)))
        || assets
            .iter()
            .any(|asset| asset.bytes == 0 || asset.bytes > MAX_RELEASE_RUNTIME_BYTES)
    {
        return Err(ActionError::rejected(
            "runtime manifest generation requires the exact qualified bounded executables and source LICENSE",
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
    Ok(bytes)
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

fn runtime_name(target: &str) -> &'static str {
    match target {
        "aarch64-apple-darwin" => "cargo-rail-action-aarch64-apple-darwin",
        "aarch64-unknown-linux-gnu" => "cargo-rail-action-aarch64-unknown-linux-gnu",
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
