//! FFmpeg-backed media export. Deterministic frames come from `offline`.

use crate::{
    ffmpeg,
    offline::{hash_draw_commands, hash_rgb, AudioWindow, FrameRange, FrameSession, RenderedFrame},
};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

#[derive(Debug, Serialize)]
pub struct ExportReport {
    pub ffmpeg_source: String,
    pub ffmpeg_path: String,
    pub ffprobe_path: String,
    pub submitted_frames: u64,
    pub fps: u32,
    pub timeline_start: f64,
    pub timeline_end: f64,
    pub bgm_offset: f64,
    pub audio_window: AudioWindow,
    pub deterministic_frame_hashes_match: bool,
    pub first_pass_hash: String,
    pub second_pass_hash: String,
    pub frame_diagnostics: Vec<FrameDiagnostic>,
    pub probe: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct FrameDiagnostic {
    pub output_local_frame_index: u64,
    pub global_frame_index: u64,
    pub requested_timeline_time: f64,
    pub runtime_timeline: Option<f64>,
    pub runtime_update: [f64; 4],
    pub runtime_update_after_callbacks: [f64; 4],
    pub timescale: f64,
    pub callback_count: usize,
    pub runtime_entity_count: usize,
    pub active_entity_count: usize,
    pub draw_count: usize,
    pub raw_draw_sha1: String,
    pub rgb_sha1: String,
    pub target_entity_draws: Vec<EntityDrawDiagnostic>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EntityDrawDiagnostic {
    pub draw_node: usize,
    pub sprite_id: u32,
    pub corners: [[f64; 2]; 4],
    pub unlerp_values: Vec<UnlerpDiagnostic>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnlerpDiagnostic {
    pub node: usize,
    pub function: String,
    pub arguments: Vec<usize>,
    pub argument_values: Vec<Option<f64>>,
    pub result: f64,
}

pub struct ExportRequest<'a> {
    pub engine: &'a Path,
    pub resources: &'a Path,
    pub level: &'a Path,
    pub music: &'a Path,
    pub output: &'a Path,
    pub start_time: f64,
    pub duration: f64,
    pub fps: u32,
    pub width: u32,
    pub height: u32,
    pub trace_entity_id: Option<usize>,
    pub level_option_overrides: &'a [(usize, f64)],
}

pub fn export(request: ExportRequest<'_>) -> Result<ExportReport> {
    for (label, path) in [
        ("engine", request.engine),
        ("resources", request.resources),
        ("level data", request.level),
        ("music", request.music),
    ] {
        if !path.is_file() {
            bail!(
                "{label} input is not an accessible file: {}",
                path.display()
            );
        }
    }
    if let Some(parent) = request
        .output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating output directory {}", parent.display()))?;
    }
    if request.width % 2 != 0 || request.height % 2 != 0 {
        bail!("H.264 yuv420p output requires even frame dimensions");
    }
    let range = FrameRange::new(request.start_time, request.duration, request.fps)?;
    let package = crate::formats::load_engine(request.engine)?;
    let level = crate::formats::load_level(request.level)?;
    let skin_name = package
        .metadata
        .skin_name
        .as_deref()
        .context("engine has no default skin name")?;
    let bgm_offset = level.bgm_offset.unwrap_or(0.0);
    if !bgm_offset.is_finite() {
        bail!("level bgmOffset must be finite");
    }
    let audio = crate::offline::audio_window(range.start_time(), bgm_offset)?;

    let make_session = || {
        FrameSession::new(
            &package.watch,
            &package.rom,
            &package.configuration,
            &level,
            request.resources,
            skin_name,
            package.metadata.background_name.as_deref(),
            package.metadata.effect_name.as_deref(),
            package.metadata.particle_name.as_deref(),
            request.width,
            request.height,
            request.fps,
            request.level_option_overrides,
        )
    };
    let expected_frames = render_diagnostic_pass(
        make_session()?,
        range,
        &package.watch,
        request.trace_entity_id,
    )?;
    let first_pass_hash = hash_sequence(
        &expected_frames
            .iter()
            .map(|frame| frame.rgb_sha1.clone())
            .collect::<Vec<_>>(),
    );

    let installation = ffmpeg::resolve()?;
    let mut child = Command::new(&installation.ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "rawvideo",
            "-pixel_format",
            "rgb24",
            "-video_size",
            &format!("{}x{}", request.width, request.height),
            "-framerate",
            &request.fps.to_string(),
            "-i",
            "pipe:0",
            "-i",
            &request.music.to_string_lossy(),
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-vf",
            "format=yuv420p",
            "-af",
            &audio_filter(audio),
            "-frames:v",
            &range.frame_count.to_string(),
            "-t",
            &format!("{:.12}", range.frame_count as f64 / f64::from(range.fps)),
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            "18",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-movflags",
            "+faststart",
            &request.output.to_string_lossy(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("launching FFmpeg at {}", installation.ffmpeg.display()))?;

    let stdin = child
        .stdin
        .take()
        .context("FFmpeg process did not expose its raw-video input")?;
    let actual_frames = match stream_frames(
        make_session()?,
        range,
        stdin,
        &expected_frames,
        &package.watch,
        request.trace_entity_id,
    ) {
        Ok(frames) => frames,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error).context("streaming deterministic RGB frames to FFmpeg");
        }
    };
    let output = child
        .wait_with_output()
        .context("waiting for FFmpeg export")?;
    if !output.status.success() {
        bail!(
            "FFmpeg failed with status {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let probe_output = Command::new(&installation.ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration,start_time:stream=index,codec_type,codec_name,width,height,pix_fmt,r_frame_rate,avg_frame_rate,time_base,duration,nb_frames,start_time",
            "-of",
            "json",
            &request.output.to_string_lossy(),
        ])
        .output()
        .context("launching FFprobe on rendered MP4")?;
    if !probe_output.status.success() {
        bail!(
            "FFprobe failed with status {:?}: {}",
            probe_output.status.code(),
            String::from_utf8_lossy(&probe_output.stderr).trim()
        );
    }
    let probe: Value =
        serde_json::from_slice(&probe_output.stdout).context("decoding FFprobe JSON result")?;
    validate_probe(
        &probe,
        range.frame_count,
        request.width,
        request.height,
        request.fps,
        range.frame_count as f64 / f64::from(range.fps),
    )?;
    let second_pass_hash = hash_sequence(
        &actual_frames
            .iter()
            .map(|frame| frame.rgb_sha1.clone())
            .collect::<Vec<_>>(),
    );
    let deterministic_frame_hashes_match =
        expected_frames
            .iter()
            .zip(&actual_frames)
            .all(|(first, second)| {
                first.raw_draw_sha1 == second.raw_draw_sha1 && first.rgb_sha1 == second.rgb_sha1
            });
    Ok(ExportReport {
        ffmpeg_source: match installation.source {
            ffmpeg::FfmpegSource::SharedAddons => "shared addons".to_owned(),
            ffmpeg::FfmpegSource::Managed => "managed".to_owned(),
        },
        ffmpeg_path: installation.ffmpeg.display().to_string(),
        ffprobe_path: installation.ffprobe.display().to_string(),
        submitted_frames: range.frame_count,
        fps: range.fps,
        timeline_start: range.start_time(),
        timeline_end: range.time(range.first_index + range.frame_count),
        bgm_offset,
        audio_window: audio,
        deterministic_frame_hashes_match,
        first_pass_hash,
        second_pass_hash,
        frame_diagnostics: actual_frames,
        probe,
    })
}

fn render_diagnostic_pass(
    mut session: FrameSession<'_>,
    range: FrameRange,
    watch: &crate::watch::WatchData,
    target_entity_id: Option<usize>,
) -> Result<Vec<FrameDiagnostic>> {
    (0..range.frame_count)
        .map(|segment_index| {
            let frame_index = range.index(segment_index)?;
            let frame = session.render_global_frame(frame_index)?;
            Ok(frame_diagnostic(
                segment_index,
                frame_index,
                range.time(frame_index),
                &frame,
                watch,
                target_entity_id,
            ))
        })
        .collect()
}

fn stream_frames(
    mut session: FrameSession<'_>,
    range: FrameRange,
    mut stdin: impl Write,
    expected_frames: &[FrameDiagnostic],
    watch: &crate::watch::WatchData,
    target_entity_id: Option<usize>,
) -> Result<Vec<FrameDiagnostic>> {
    if expected_frames.len() != range.frame_count as usize {
        bail!("determinism preflight returned an unexpected number of frames");
    }
    let mut actual_frames = Vec::with_capacity(expected_frames.len());
    for (segment_index, expected) in expected_frames.iter().enumerate() {
        let global_index = range.index(segment_index as u64)?;
        let frame = session.render_global_frame(global_index)?;
        let diagnostic = frame_diagnostic(
            segment_index as u64,
            global_index,
            range.time(global_index),
            &frame,
            watch,
            target_entity_id,
        );
        if diagnostic.raw_draw_sha1 != expected.raw_draw_sha1
            || diagnostic.rgb_sha1 != expected.rgb_sha1
        {
            bail!("frame {global_index} changed between deterministic passes");
        }
        stdin
            .write_all(&frame.rgb)
            .with_context(|| format!("writing raw RGB frame {global_index} to FFmpeg"))?;
        actual_frames.push(diagnostic);
    }
    stdin.flush().context("flushing raw RGB frames to FFmpeg")?;
    drop(stdin);
    Ok(actual_frames)
}

fn frame_diagnostic(
    output_index: u64,
    global_index: u64,
    time: f64,
    frame: &RenderedFrame,
    watch: &crate::watch::WatchData,
    target_entity_id: Option<usize>,
) -> FrameDiagnostic {
    let target_entity_draws = target_entity_id.map_or_else(Vec::new, |entity_id| {
        frame
            .report
            .display_list
            .sprites
            .iter()
            .filter(|draw| {
                draw.provenance.as_ref().and_then(|source| source.entity_id) == Some(entity_id)
            })
            .map(|draw| {
                let unlerp_values = draw
                    .trace
                    .as_ref()
                    .into_iter()
                    .flat_map(|trace| trace.node_values.iter())
                    .filter_map(|(&node_id, &value)| {
                        let node = watch.nodes.get(node_id)?;
                        let function = node.func.as_deref()?;
                        if !matches!(function, "Unlerp" | "UnlerpClamped") {
                            return None;
                        }
                        let arguments: Vec<usize> = node
                            .args
                            .iter()
                            .filter_map(|arg| arg.as_u64().and_then(|id| usize::try_from(id).ok()))
                            .collect();
                        let argument_values = arguments
                            .iter()
                            .map(|argument| {
                                draw.trace
                                    .as_ref()
                                    .and_then(|trace| trace.node_values.get(argument).copied())
                            })
                            .collect();
                        Some(UnlerpDiagnostic {
                            node: node_id,
                            function: function.to_owned(),
                            arguments,
                            argument_values,
                            result: value,
                        })
                    })
                    .collect();
                EntityDrawDiagnostic {
                    draw_node: draw
                        .provenance
                        .as_ref()
                        .map_or(usize::MAX, |source| source.draw_node),
                    sprite_id: draw.sprite_id,
                    corners: draw.corners,
                    unlerp_values,
                }
            })
            .collect()
    });
    FrameDiagnostic {
        output_local_frame_index: output_index,
        global_frame_index: global_index,
        requested_timeline_time: time,
        runtime_timeline: frame.report.timeline,
        runtime_update: frame.report.runtime_update,
        runtime_update_after_callbacks: frame.report.runtime_update_after_callbacks,
        timescale: frame.report.timescale,
        callback_count: frame.report.callbacks.len(),
        runtime_entity_count: frame.report.runtime_entity_count,
        active_entity_count: frame.report.active_entity_count,
        draw_count: frame.report.display_list.sprites.len(),
        raw_draw_sha1: hash_draw_commands(&frame.report.display_list),
        rgb_sha1: hash_rgb(&frame.rgb),
        target_entity_draws,
    }
}

fn hash_sequence(hashes: &[String]) -> String {
    let mut digest = Sha1::new();
    for hash in hashes {
        digest.update(hash.as_bytes());
    }
    hex::encode(digest.finalize())
}

fn audio_filter(window: AudioWindow) -> String {
    format!(
        "atrim=start={:.12},asetpts=PTS-STARTPTS+{:.12}/TB",
        window.source_start, window.leading_silence
    )
}

fn validate_probe(
    probe: &Value,
    frames: u64,
    width: u32,
    height: u32,
    fps: u32,
    expected_duration: f64,
) -> Result<()> {
    let streams = probe
        .get("streams")
        .and_then(Value::as_array)
        .context("FFprobe JSON has no streams array")?;
    let video = streams.iter().find(|s| s["codec_type"] == "video");
    let audio = streams.iter().find(|s| s["codec_type"] == "audio");
    let video = video.context("export has no video stream")?;
    let audio = audio.context("export has no audio stream")?;
    if video["width"].as_u64() != Some(u64::from(width))
        || video["height"].as_u64() != Some(u64::from(height))
    {
        bail!("FFprobe video dimensions do not match requested frame size");
    }
    let actual_frames = video["nb_frames"]
        .as_str()
        .and_then(|value| value.parse::<u64>().ok())
        .context("FFprobe video frame count is missing or invalid")?;
    if actual_frames != frames {
        bail!("FFprobe video frame count does not match submitted frame count");
    }
    if streams.len() != 2 {
        bail!(
            "expected exactly one video and one audio stream, got {}",
            streams.len()
        );
    }
    let frame_rate = video["avg_frame_rate"]
        .as_str()
        .context("FFprobe average video frame rate is missing")?;
    if parse_rational(frame_rate).context("invalid FFprobe average video frame rate")?
        != f64::from(fps)
    {
        bail!("FFprobe video frame rate does not match requested FPS");
    }
    if video["codec_name"].as_str().is_none_or(str::is_empty)
        || audio["codec_name"].as_str().is_none_or(str::is_empty)
    {
        bail!("FFprobe did not report both output codecs");
    }
    if video["codec_name"] != "h264"
        || video["pix_fmt"] != "yuv420p"
        || audio["codec_name"] != "aac"
    {
        bail!("FFprobe codecs or pixel format differ from the configured MP4 output");
    }
    let duration = probe["format"]["duration"]
        .as_str()
        .and_then(|value| value.parse::<f64>().ok())
        .context("FFprobe format duration is missing or invalid")?;
    if (duration - expected_duration).abs() > 1.0 / f64::from(fps) {
        bail!("FFprobe output duration differs from the requested frame sequence");
    }
    for stream in [video, audio] {
        let stream_duration = stream["duration"]
            .as_str()
            .and_then(|value| value.parse::<f64>().ok())
            .context("FFprobe stream duration is missing or invalid")?;
        if (stream_duration - expected_duration).abs() > 1.0 / f64::from(fps) {
            bail!("FFprobe stream duration differs from the requested segment");
        }
        let start = stream["start_time"]
            .as_str()
            .and_then(|value| value.parse::<f64>().ok())
            .context("FFprobe stream start time is missing or invalid")?;
        if start.abs() > 1e-6 {
            bail!("FFprobe stream does not start at output time zero");
        }
    }
    Ok(())
}

fn parse_rational(value: &str) -> Option<f64> {
    let (numerator, denominator) = value.split_once('/')?;
    let numerator = numerator.parse::<f64>().ok()?;
    let denominator = denominator.parse::<f64>().ok()?;
    (denominator != 0.0).then_some(numerator / denominator)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_filter_trims_to_timeline_position_and_applies_leading_silence() {
        assert_eq!(
            audio_filter(AudioWindow {
                source_start: 4.5,
                leading_silence: 0.0
            }),
            "atrim=start=4.500000000000,asetpts=PTS-STARTPTS+0.000000000000/TB"
        );
        assert!(audio_filter(AudioWindow {
            source_start: 0.0,
            leading_silence: 0.05
        })
        .contains("+0.050000000000/TB"));
    }
}
