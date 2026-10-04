//! Bounded media preparation through the application's managed FFmpeg pair.
use crate::{
    export_timeline::{ExportTimeline, AUDIO_RATE},
    ffmpeg::FfmpegInstallation,
};
use anyhow::{bail, Context, Result};
use std::{path::Path, process::Command};

pub fn source_duration(installation: &FfmpegInstallation, path: &Path) -> Result<f64> {
    let output = crate::export_control::hide_child_window(&mut Command::new(&installation.ffprobe))
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "format=duration:stream=codec_type,duration",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .context("probing BGM source")?;
    if !output.status.success() {
        bail!(
            "BGM probe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    if value["streams"]
        .as_array()
        .is_none_or(|streams| streams.is_empty())
    {
        bail!("BGM has no audio stream");
    }
    let duration = [
        &value["streams"][0]["duration"],
        &value["format"]["duration"],
    ]
    .into_iter()
    .filter_map(|v| v.as_str().and_then(|v| v.parse::<f64>().ok()))
    .find(|v| v.is_finite() && *v > 0.0)
    .context("BGM has no finite positive source duration")?;
    Ok(duration)
}

pub fn prepare_bgm(
    installation: &FfmpegInstallation,
    path: &Path,
    timeline: ExportTimeline,
) -> Result<tempfile::NamedTempFile> {
    prepare_bgm_controlled(installation, path, timeline, &Default::default())
}

pub(crate) fn prepare_bgm_controlled(
    installation: &FfmpegInstallation,
    path: &Path,
    timeline: ExportTimeline,
    control: &crate::export_control::ExportControl,
) -> Result<tempfile::NamedTempFile> {
    let file = tempfile::Builder::new().suffix(".pcm").tempfile()?;
    let output = control
        .output(
            crate::export_control::hide_child_window(&mut Command::new(&installation.ffmpeg))
                .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
                .arg(path)
                .args([
                    "-map",
                    "0:a:0",
                    "-af",
                    &timeline.bgm_filter(),
                    "-ar",
                    &AUDIO_RATE.to_string(),
                    "-ac",
                    "2",
                    "-c:a",
                    "pcm_s16le",
                    "-f",
                    "s16le",
                ])
                .arg(file.path()),
        )
        .context("preparing zero-based padded BGM PCM")?;
    if !output.status.success() {
        bail!(
            "BGM PCM preparation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let bytes = file.as_file().metadata()?.len();
    if bytes != timeline.audio_sample_frames * 4 {
        bail!(
            "BGM PCM has {bytes} bytes, expected {}",
            timeline.audio_sample_frames * 4
        );
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offline::FrameRange;
    #[test]
    fn managed_pcm_clips_and_pads_exact_sample_intervals() {
        // Exercise installed managed tools without making unit tests provision tools.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("dependencies/ffmpeg/8.1");
        let suffix = if cfg!(windows) { ".exe" } else { "" };
        let installation = FfmpegInstallation {
            source: crate::ffmpeg::FfmpegSource::Managed,
            ffmpeg: root.join(format!("ffmpeg{suffix}")),
            ffprobe: root.join(format!("ffprobe{suffix}")),
        };
        if !installation.ffmpeg.is_file() {
            eprintln!("managed PCM integration check skipped: managed FFmpeg not installed");
            return;
        }
        let source = tempfile::Builder::new().suffix(".wav").tempfile().unwrap();
        // Constant PCM makes clipping/padding boundaries observable without AAC rounding.
        let pcm = [10000_i16.to_le_bytes(); 4410].concat();
        std::fs::write(source.path(), crate::audio::wav_header(44_100, 1, &pcm)).unwrap();
        for (start, offset, leading, audible) in [
            (0.0, -0.05, 2205, 4410),
            (0.0, 0.05, 0, 2205),
            (0.25, 0.0, 0, 0),
            (0.0, -1.0, 8820, 0),
            (0.05, 0.0, 0, 2205),
        ] {
            let timeline =
                ExportTimeline::new(start, FrameRange::new(start, 0.2, 20).unwrap(), offset)
                    .unwrap();
            let file = prepare_bgm(&installation, source.path(), timeline).unwrap();
            let bytes = std::fs::read(file.path()).unwrap();
            assert_eq!(bytes.len(), 8820 * 4);
            let frames: Vec<_> = bytes
                .chunks_exact(4)
                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                .collect();
            assert!(frames[..leading].iter().all(|v| *v == 0));
            assert!(frames[leading..leading + audible].iter().all(|v| *v != 0));
            assert!(frames[leading + audible..].iter().all(|v| *v == 0));
        }
    }
}
