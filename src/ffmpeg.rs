//! FFmpeg executable discovery and managed Windows provisioning.
//!
//! The shared `addons` directory is preferred. Managed provisioning is lazy: it
//! happens only when a caller asks this module to resolve FFmpeg.

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const DISTRIBUTION_URL: &str =
    "https://github.com/GyanD/codexffmpeg/releases/download/8.1/ffmpeg-8.1-full_build.zip";
const DISTRIBUTION_SHA256: &str =
    "587B1C37DE29C5003D01CF65DA10001BAC43A58B88E61AF0FC77C61DAFF04761";
const MANAGED_VERSION: &str = "8.1";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FfmpegSource {
    SharedAddons,
    Managed,
}

#[derive(Debug, Clone, Serialize)]
pub struct FfmpegInstallation {
    pub source: FfmpegSource,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

/// Find a validated shared installation, otherwise use or provision the
/// application-managed installation. PATH is deliberately never consulted.
pub fn resolve() -> Result<FfmpegInstallation> {
    let app_root = application_root()?;
    #[cfg(windows)]
    {
        let shared = app_root
            .parent()
            .ok_or_else(|| anyhow!("application root has no parent directory"))?
            .join("addons");
        if let Ok(installation) = validate_pair(
            FfmpegSource::SharedAddons,
            shared.join("ffmpeg.exe"),
            shared.join("ffprobe.exe"),
        ) {
            return Ok(installation);
        }
    }

    let (ffmpeg_name, ffprobe_name) = binary_names();
    let managed = app_root
        .join("dependencies")
        .join("ffmpeg")
        .join(MANAGED_VERSION);
    if let Ok(installation) = validate_pair(
        FfmpegSource::Managed,
        managed.join(ffmpeg_name),
        managed.join(ffprobe_name),
    ) {
        return Ok(installation);
    }

    install_managed(&managed)?;
    validate_pair(
        FfmpegSource::Managed,
        managed.join(ffmpeg_name),
        managed.join(ffprobe_name),
    )
    .context("newly provisioned FFmpeg did not pass validation")
}

#[cfg(windows)]
fn binary_names() -> (&'static str, &'static str) {
    ("ffmpeg.exe", "ffprobe.exe")
}

#[cfg(not(windows))]
fn binary_names() -> (&'static str, &'static str) {
    ("ffmpeg", "ffprobe")
}

/// Resolve the application directory independently of the process working
/// directory. Cargo debug/release binaries use the repository as their root.
fn application_root() -> Result<PathBuf> {
    let executable = std::env::current_exe().context("locating Sono-Renderer executable")?;
    let executable_dir = executable
        .parent()
        .ok_or_else(|| anyhow!("executable path has no parent directory"))?;
    let profile = executable_dir.file_name().and_then(|s| s.to_str());
    if matches!(profile, Some("debug" | "release")) {
        return Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    }
    Ok(executable_dir.to_path_buf())
}

fn validate_pair(
    source: FfmpegSource,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
) -> Result<FfmpegInstallation> {
    validate_binary(&ffmpeg, "ffmpeg")?;
    validate_binary(&ffprobe, "ffprobe")?;
    let ffmpeg_version = version_token(&run_version(&ffmpeg)?, "ffmpeg")?;
    let ffprobe_version = version_token(&run_version(&ffprobe)?, "ffprobe")?;
    if ffmpeg_version != ffprobe_version {
        bail!("FFmpeg and FFprobe versions do not match ({ffmpeg_version} vs {ffprobe_version})");
    }
    Ok(FfmpegInstallation {
        source,
        ffmpeg,
        ffprobe,
    })
}

fn validate_binary(path: &Path, label: &str) -> Result<()> {
    let metadata =
        fs::metadata(path).with_context(|| format!("accessing {} at {}", label, path.display()))?;
    if !metadata.is_file() || metadata.len() == 0 {
        bail!("{} is not a non-empty file: {}", label, path.display());
    }
    Ok(())
}

fn run_version(path: &Path) -> Result<String> {
    let output = Command::new(path)
        .arg("-version")
        .output()
        .with_context(|| format!("launching {}", path.display()))?;
    // Windows PowerShell can report an unusual pipeline status for ffprobe.
    // A valid version banner is authoritative here; don't reject solely by code.
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !text
        .lines()
        .any(|line| line.starts_with("ffmpeg version ") || line.starts_with("ffprobe version "))
    {
        bail!(
            "{} did not report a valid version banner (process status {:?})",
            path.display(),
            output.status.code()
        );
    }
    Ok(text)
}

fn version_token(output: &str, program: &str) -> Result<String> {
    let prefix = format!("{program} version ");
    output
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|version| version.split_whitespace().next())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("{program} version output could not be parsed"))
}

#[cfg(windows)]
fn install_managed(destination: &Path) -> Result<()> {
    let managed_root = destination
        .parent()
        .ok_or_else(|| anyhow!("managed FFmpeg path has no parent"))?;
    fs::create_dir_all(managed_root).context("creating managed FFmpeg directory")?;
    let archive_path = managed_root.join(format!("ffmpeg-{MANAGED_VERSION}.zip"));
    download_archive(&archive_path)?;
    verify_archive(&archive_path)?;

    let staging = managed_root.join(format!("{MANAGED_VERSION}.installing"));
    if staging.exists() {
        fs::remove_dir_all(&staging).context("removing incomplete FFmpeg staging directory")?;
    }
    fs::create_dir_all(&staging).context("creating FFmpeg staging directory")?;
    if let Err(error) = extract_executables(&archive_path, &staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    let (ffmpeg_name, ffprobe_name) = binary_names();
    validate_pair(
        FfmpegSource::Managed,
        staging.join(ffmpeg_name),
        staging.join(ffprobe_name),
    )
    .context("validating staged FFmpeg executables")?;

    if destination.exists() {
        fs::remove_dir_all(destination).context("replacing invalid managed FFmpeg installation")?;
    }
    fs::rename(&staging, destination).context("activating managed FFmpeg installation")?;
    let _ = fs::remove_file(&archive_path);
    Ok(())
}

#[cfg(windows)]
fn download_archive(destination: &Path) -> Result<()> {
    let destination = destination.canonicalize().or_else(|_| {
        let parent = destination.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        Ok::<_, std::io::Error>(parent.join(destination.file_name().unwrap_or_default()))
    })?;
    let script_path = destination.to_string_lossy().replace('\'', "''");
    let command = format!(
        "$ErrorActionPreference='Stop'; [Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12; Invoke-WebRequest -Uri '{DISTRIBUTION_URL}' -OutFile '{script_path}'"
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &command])
        .output()
        .context("starting the managed FFmpeg download")?;
    ensure_success(output, "downloading managed FFmpeg archive").map(|_| ())
}

#[cfg(not(windows))]
fn download_archive(_destination: &Path) -> Result<()> {
    bail!("automatic managed FFmpeg provisioning is not yet available on this platform")
}

#[cfg(not(windows))]
fn install_managed(_destination: &Path) -> Result<()> {
    bail!("automatic managed FFmpeg provisioning is not yet available on this platform")
}

fn verify_archive(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        let escaped = path.to_string_lossy().replace('\'', "''");
        let command = format!(
            "$ErrorActionPreference='Stop'; (Get-FileHash -Algorithm SHA256 -LiteralPath '{escaped}').Hash"
        );
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &command])
            .output()
            .context("calculating managed FFmpeg archive SHA-256")?;
        let result = ensure_success(output, "calculating managed FFmpeg archive SHA-256")?;
        let actual = String::from_utf8_lossy(&result.stdout)
            .trim()
            .to_ascii_uppercase();
        if actual != DISTRIBUTION_SHA256 {
            let _ = fs::remove_file(path);
            bail!("managed FFmpeg archive SHA-256 mismatch (got {actual})");
        }
        return Ok(());
    }
    #[cfg(not(windows))]
    bail!("managed FFmpeg archive verification is currently supported on Windows only")
}

fn extract_executables(archive_path: &Path, destination: &Path) -> Result<()> {
    let file = fs::File::open(archive_path).context("opening managed FFmpeg archive")?;
    let mut archive = zip::ZipArchive::new(file).context("reading managed FFmpeg ZIP archive")?;
    for required in ["ffmpeg.exe", "ffprobe.exe"] {
        let mut found = false;
        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .context("reading FFmpeg ZIP entry")?;
            let name = entry.name().replace('\\', "/");
            if name.ends_with(&format!("/bin/{required}")) || name == format!("bin/{required}") {
                let output_path = destination.join(required);
                let mut output = fs::File::create(&output_path)
                    .with_context(|| format!("creating {}", output_path.display()))?;
                let copied = std::io::copy(&mut entry, &mut output)
                    .with_context(|| format!("extracting {required}"))?;
                if copied == 0 {
                    bail!("managed archive contained an empty {required}");
                }
                output
                    .flush()
                    .context("flushing extracted FFmpeg executable")?;
                found = true;
                break;
            }
        }
        if !found {
            bail!("managed archive does not contain {required}");
        }
    }
    Ok(())
}

fn ensure_success(output: Output, action: &str) -> Result<Output> {
    if !output.status.success() {
        bail!(
            "{action} failed with status {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output)
}
