//! Self-update for official OpenDoc CLI release packages.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use reqwest::blocking::Client;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::catalog::feed::http_client;
use crate::docsets::{error, lock};
use crate::error::CliError;

const REPO: &str = "daidarzzz/opendoc-cli";
const MARKER: &str = "opendoc-cli.release";
const MAX_PACKAGE: u64 = 128 * 1024 * 1024;
const MAX_BINARY: u64 = 64 * 1024 * 1024;
#[cfg(windows)]
const BINARIES: [&str; 2] = ["odc.exe", "opendoc-cli.exe"];
#[cfg(not(windows))]
const BINARIES: [&str; 2] = ["odc", "opendoc-cli"];

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

impl Release {
    fn asset_url(&self, name: &str) -> Result<&str, CliError> {
        self.assets
            .iter()
            .find(|asset| asset.name == name)
            .map(|asset| asset.browser_download_url.as_str())
            .filter(|url| {
                *url == format!(
                    "https://github.com/{REPO}/releases/download/{}/{name}",
                    self.tag_name
                )
            })
            .ok_or_else(|| {
                error(format!(
                    "{} does not have a release asset named {name}",
                    self.tag_name
                ))
            })
    }
}

pub(crate) fn run(check: bool) -> Result<(), CliError> {
    let current = parse_version(env!("CARGO_PKG_VERSION"))?;
    let client = http_client().map_err(error)?;
    let release: Release = client
        .get(format!(
            "https://api.github.com/repos/{REPO}/releases/latest"
        ))
        .send()
        .and_then(|res| res.error_for_status())
        .and_then(|res| res.json())
        .map_err(error)?;
    let latest = parse_version(&release.tag_name)?;
    if !latest.pre.is_empty() {
        return Err(error("the latest release is not a stable version"));
    }
    println!("Installed: {current} · Latest: {latest}");
    if current >= latest {
        println!("OpenDoc CLI is up to date.");
        return Ok(());
    }
    if check {
        return Ok(());
    }
    let directory = install_dir()?;
    let (target, extension) = target()?;
    let name = format!("opendoc-cli-{}-{target}.{extension}", release.tag_name);
    let package_url = release.asset_url(&name)?;
    let checksum_url = release.asset_url(&format!("{name}.sha256"))?;
    let _lock = lock(&directory.join(".odc-update.lock"))?;
    let stage = tempfile::Builder::new()
        .prefix(".odc-update-")
        .tempdir_in(&directory)
        .map_err(error)?;

    let checksum = read_checksum(&client, checksum_url, &name)?;
    let package = stage.path().join(format!("package.{extension}"));
    println!("Downloading {name}…");
    download(&client, package_url, &package, &checksum)?;
    unpack(&package, stage.path())?;
    for name in BINARIES {
        validate_binary(&stage.path().join(name))?;
    }

    #[cfg(not(windows))]
    {
        replace_pair(&directory, stage.path())?;
        println!("Updated OpenDoc CLI to {latest}.");
    }
    #[cfg(windows)]
    {
        finish_on_windows(&directory, stage, &current, &latest)?;
        println!("OpenDoc CLI {latest} will be installed after this process exits.");
    }
    Ok(())
}

fn parse_version(value: &str) -> Result<Version, CliError> {
    Version::parse(value.strip_prefix('v').unwrap_or(value)).map_err(error)
}

fn target() -> Result<(&'static str, &'static str), CliError> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") if cfg!(target_env = "gnu") => {
            Ok(("x86_64-unknown-linux-gnu", "tar.gz"))
        }
        ("macos", "x86_64") => Ok(("x86_64-apple-darwin", "tar.gz")),
        ("macos", "aarch64") => Ok(("aarch64-apple-darwin", "tar.gz")),
        ("windows", "x86_64") if cfg!(target_env = "msvc") => Ok(("x86_64-pc-windows-msvc", "zip")),
        _ => Err(error(
            "there is no official release package for this platform",
        )),
    }
}

fn install_dir() -> Result<PathBuf, CliError> {
    let exe = std::env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(error)?;
    let directory = exe
        .parent()
        .ok_or_else(|| error("cannot locate the installation directory"))?;
    let filename = exe
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if !BINARIES
        .iter()
        .any(|name| filename.eq_ignore_ascii_case(name))
        || !directory.join(MARKER).is_file()
    {
        return Err(error(format!(
            "this installation is managed externally or has no release marker. Update through the original installer, or download a package from https://github.com/{REPO}/releases. For Cargo, rerun: cargo install --git https://github.com/{REPO} --locked"
        )));
    }
    for name in BINARIES {
        let path = directory.join(name);
        if !fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_file()) {
            return Err(error(format!(
                "missing release executable: {}",
                path.display()
            )));
        }
    }
    Ok(directory.to_path_buf())
}

fn read_checksum(client: &Client, url: &str, name: &str) -> Result<String, CliError> {
    let mut body = String::new();
    client
        .get(url)
        .send()
        .and_then(|res| res.error_for_status())
        .map_err(error)?
        .take(4097)
        .read_to_string(&mut body)
        .map_err(error)?;
    parse_checksum(&body, name)
}

fn parse_checksum(body: &str, name: &str) -> Result<String, CliError> {
    let mut fields = body.split_ascii_whitespace();
    let checksum = fields.next().unwrap_or_default();
    let filename = fields.next().unwrap_or_default().trim_start_matches('*');
    if body.len() > 4096
        || checksum.len() != 64
        || !checksum.bytes().all(|b| b.is_ascii_hexdigit())
        || filename != name
        || fields.next().is_some()
    {
        return Err(error("invalid release checksum"));
    }
    Ok(checksum.to_ascii_lowercase())
}

fn download(client: &Client, url: &str, package: &Path, checksum: &str) -> Result<(), CliError> {
    let response = client
        .get(url)
        .send()
        .and_then(|res| res.error_for_status())
        .map_err(error)?;
    save_verified(response, package, checksum)
}

fn save_verified(source: impl Read, package: &Path, checksum: &str) -> Result<(), CliError> {
    let mut source = source.take(MAX_PACKAGE + 1);
    let mut out = fs::File::create_new(package).map_err(error)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = source.read(&mut buffer).map_err(error)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        out.write_all(&buffer[..count]).map_err(error)?;
    }
    if source.limit() == 0 {
        return Err(error("release archive exceeds the download size limit"));
    }
    if format!("{:x}", digest.finalize()) != checksum {
        return Err(error("release archive SHA-256 does not match"));
    }
    out.sync_all().map_err(error)
}

#[cfg(not(windows))]
fn unpack(package: &Path, stage: &Path) -> Result<(), CliError> {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Component;

    let decoder = flate2::read::GzDecoder::new(fs::File::open(package).map_err(error)?);
    let mut tar = tar::Archive::new(decoder.take(MAX_PACKAGE));
    for entry in tar.entries().map_err(error)? {
        let mut entry = entry.map_err(error)?;
        let path = entry.path().map_err(error)?;
        let mut components = path.components().filter(|c| *c != Component::CurDir);
        let Some(Component::Normal(name)) = components.next() else {
            continue;
        };
        if components.next().is_some() || !BINARIES.iter().any(|binary| name == *binary) {
            continue;
        }
        if !entry.header().entry_type().is_file() || entry.size() > MAX_BINARY {
            return Err(error(format!(
                "invalid release archive entry: {}",
                path.display()
            )));
        }
        let mut out = fs::File::create_new(stage.join(name)).map_err(error)?;
        std::io::copy(&mut entry, &mut out).map_err(error)?;
        out.set_permissions(fs::Permissions::from_mode(0o755))
            .map_err(error)?;
        out.sync_all().map_err(error)?;
    }
    Ok(())
}

#[cfg(windows)]
fn unpack(package: &Path, stage: &Path) -> Result<(), CliError> {
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command",
            "$ErrorActionPreference = 'Stop'; Expand-Archive -LiteralPath $env:ODC_UPDATE_ARCHIVE -DestinationPath $env:ODC_UPDATE_STAGE"])
        .env("ODC_UPDATE_ARCHIVE", package)
        .env("ODC_UPDATE_STAGE", stage)
        .output().map_err(error)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(error(format!(
            "cannot unpack the release: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

fn validate_binary(path: &Path) -> Result<(), CliError> {
    let meta = fs::symlink_metadata(path).map_err(error)?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_BINARY {
        return Err(error(format!(
            "invalid release executable: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_pair(directory: &Path, stage: &Path) -> Result<(), CliError> {
    // Back up both existing executables before publishing either new one.
    let backup = stage.join("previous");
    fs::create_dir(&backup).map_err(error)?;
    for name in BINARIES {
        fs::hard_link(directory.join(name), backup.join(name)).map_err(error)?;
    }
    let mut replaced = Vec::new();
    for name in BINARIES {
        if let Err(e) = fs::rename(stage.join(name), directory.join(name)) {
            for previous in replaced {
                if let Err(restore) = fs::rename(backup.join(previous), directory.join(previous)) {
                    // Keep a usable copy even if automatic rollback fails.
                    let recovery = tempfile::Builder::new()
                        .prefix(".odc-recovery-")
                        .tempdir_in(directory)
                        .map_err(error)?
                        .keep();
                    fs::copy(backup.join(previous), recovery.join(previous)).map_err(error)?;
                    return Err(error(format!(
                        "update failed: {e}. Could not restore {previous}: {restore}. Backup: {}",
                        recovery.display()
                    )));
                }
            }
            return Err(error(format!("update failed: {e}")));
        }
        replaced.push(name);
    }
    Ok(())
}

#[cfg(windows)]
fn finish_on_windows(
    directory: &Path,
    stage: tempfile::TempDir,
    current: &Version,
    latest: &Version,
) -> Result<(), CliError> {
    use base64::{engine::general_purpose::STANDARD, Engine};

    let script: Vec<u8> = include_str!("update_windows.ps1")
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let script = STANDARD.encode(script);
    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &script])
        .env("ODC_UPDATE_PARENT", std::process::id().to_string())
        .env("ODC_UPDATE_DIR", directory)
        .env("ODC_UPDATE_STAGE", stage.path())
        .env("ODC_UPDATE_CURRENT", current.to_string())
        .env("ODC_UPDATE_LATEST", latest.to_string())
        .spawn()
        .map_err(error)?;
    stage.keep();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrupt_release_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let package = dir.path().join("package");
        let wanted = format!("{:x}", Sha256::digest(b"correct"));
        assert!(save_verified(&b"corrupt"[..], &package, &wanted).is_err());
        let name = "opendoc-cli-v0.4.0-test.tar.gz";
        assert!(parse_checksum(&format!("{wanted}  other-file"), name).is_err());
        assert_eq!(
            parse_checksum(&format!("{wanted}  {name}\n"), name).unwrap(),
            wanted
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn verified_release_archive_replaces_both_executables() {
        let dir = tempfile::tempdir().unwrap();
        let stage = tempfile::tempdir_in(dir.path()).unwrap();
        let package = stage.path().join("release.tar.gz");
        let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        for name in BINARIES {
            fs::write(dir.path().join(name), b"previous").unwrap();
            let content = format!("new {name}");
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            archive
                .append_data(&mut header, format!("./{name}"), content.as_bytes())
                .unwrap();
        }
        let bytes = archive.into_inner().unwrap().finish().unwrap();
        let checksum = format!("{:x}", Sha256::digest(&bytes));
        save_verified(bytes.as_slice(), &package, &checksum).unwrap();
        unpack(&package, stage.path()).unwrap();
        for name in BINARIES {
            validate_binary(&stage.path().join(name)).unwrap();
        }
        replace_pair(dir.path(), stage.path()).unwrap();
        for name in BINARIES {
            assert_eq!(
                fs::read_to_string(dir.path().join(name)).unwrap(),
                format!("new {name}")
            );
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn failed_second_replacement_restores_first_executable() {
        let dir = tempfile::tempdir().unwrap();
        let stage = tempfile::tempdir_in(dir.path()).unwrap();
        for name in BINARIES {
            fs::write(dir.path().join(name), b"previous").unwrap();
        }
        fs::write(stage.path().join(BINARIES[0]), b"new").unwrap();
        assert!(replace_pair(dir.path(), stage.path()).is_err());
        for name in BINARIES {
            assert_eq!(fs::read(dir.path().join(name)).unwrap(), b"previous");
        }
    }
}
