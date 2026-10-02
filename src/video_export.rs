//! FFmpeg-backed media export. Deterministic frames come from `offline`.

use crate::runtime::{
    AudioEffectEvent, ScheduledEffect, ScheduledLoopedEffect, ScheduledLoopedEffectStop,
};
use crate::{
    ffmpeg,
    offline::{hash_draw_commands, hash_rgb, AudioWindow, FrameRange, FrameSession, RenderedFrame},
    render::RenderConfig,
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
    time::{Duration, Instant},
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
    pub vm_evaluations: u64,
    pub runtime_entity_count: usize,
    pub active_entity_count: usize,
    pub draw_count: usize,
    pub raw_draw_sha1: String,
    pub rgb_sha1: String,
    pub target_entity_draws: Vec<EntityDrawDiagnostic>,
    #[serde(skip)]
    audio_events: Vec<AudioEffectEvent>,
    #[serde(skip)]
    scheduled_effects: Vec<ScheduledEffect>,
    #[serde(skip)]
    scheduled_looped_effects: Vec<ScheduledLoopedEffect>,
    #[serde(skip)]
    scheduled_looped_effect_stops: Vec<ScheduledLoopedEffectStop>,
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
    pub skin_name: Option<&'a str>,
    pub particles_enabled: bool,
    pub sfx_enabled: bool,
    pub bgm_enabled: bool,
    pub ui: &'a crate::render_ui::RendererUiConfig,
    pub backend: crate::render::RenderBackend,
    pub profile: bool,
    pub profile_frames: bool,
}

pub fn export_config(config: &RenderConfig) -> Result<ExportReport> {
    let output = config
        .output
        .as_deref()
        .context("video output path is required")?;
    if config.layers.bgm && config.music.is_none() {
        bail!("BGM is enabled but no music file is selected");
    }
    let options: Vec<_> = config
        .level_options
        .iter()
        .map(|item| (item.index, item.value))
        .collect();
    export(ExportRequest {
        engine: &config.engine,
        resources: &config.resources,
        level: &config.level,
        music: config.music.as_deref().unwrap_or(&config.resources),
        output,
        start_time: config.start_time,
        duration: config.duration,
        fps: config.fps,
        width: config.width,
        height: config.height,
        trace_entity_id: config.trace_entity_id,
        level_option_overrides: &options,
        skin_name: config.skin.as_deref(),
        particles_enabled: config.layers.particles,
        sfx_enabled: config.layers.sfx,
        bgm_enabled: config.layers.bgm,
        ui: &config.ui,
        backend: config.backend,
        profile: config.profile,
        profile_frames: config.profile_frames,
    })
}

pub fn export(request: ExportRequest<'_>) -> Result<ExportReport> {
    let export_start = request.profile.then(Instant::now);
    let mut profile = request.profile.then(|| {
        crate::profiling::ProfileCollector::new(
            request.profile_frames,
            match request.backend {
                crate::render::RenderBackend::Cpu => "cpu",
                crate::render::RenderBackend::Wgpu => "wgpu",
            },
            (request.width, request.height),
            request.fps,
        )
    });
    let startup_start = request.profile.then(Instant::now);
    for (label, path) in [
        ("engine", request.engine),
        ("resources", request.resources),
        ("level data", request.level),
    ] {
        if !path.is_file() {
            bail!(
                "{label} input is not an accessible file: {}",
                path.display()
            );
        }
    }
    if request.bgm_enabled && !request.music.is_file() {
        bail!(
            "BGM input is not an accessible file: {}",
            request.music.display()
        );
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
    let resource_defaults: std::collections::BTreeMap<_, _> =
        package.metadata.resource_defaults().into_iter().collect();
    let skin_name = request
        .skin_name
        .or_else(|| {
            resource_defaults
                .get("skins")
                .map(String::as_str)
                .or(package.metadata.skin_name.as_deref())
        })
        .context("engine has no default skin name")?;
    let effect_name = resource_defaults
        .get("effects")
        .map(String::as_str)
        .or(package.metadata.effect_name.as_deref());
    let particle_name = resource_defaults
        .get("particles")
        .map(String::as_str)
        .or(package.metadata.particle_name.as_deref());
    let background_name = resource_defaults
        .get("backgrounds")
        .map(String::as_str)
        .or(package.metadata.background_name.as_deref());
    let bgm_offset = level.bgm_offset.unwrap_or(0.0);
    if !bgm_offset.is_finite() {
        bail!("level bgmOffset must be finite");
    }
    let audio = crate::offline::audio_window(range.start_time(), bgm_offset)?;

    let make_session = || {
        FrameSession::new_configured(
            &package.watch,
            &package.rom,
            &package.configuration,
            &level,
            request.resources,
            skin_name,
            background_name,
            effect_name,
            particle_name,
            request.width,
            request.height,
            request.fps,
            request.level_option_overrides,
            request.particles_enabled,
            request.ui.clone(),
            request.start_time,
            request.duration,
            request.backend,
            request.profile,
            request.trace_entity_id.is_some(),
        )
    };
    let initial_session = make_session()?;
    if let Some(p) = profile.as_mut() {
        p.record(
            "Initialization/startup",
            startup_start.map_or(Duration::ZERO, |t| t.elapsed()),
        );
    }
    let preflight_start = request.profile.then(Instant::now);
    let expected_frames = render_diagnostic_pass(
        initial_session,
        range,
        &package.watch,
        request.trace_entity_id,
        profile.as_mut(),
    )?;
    if let Some(p) = profile.as_mut() {
        p.record(
            "Determinism preflight",
            preflight_start.map_or(Duration::ZERO, |t| t.elapsed()),
        );
    }
    let audio_start = request.profile.then(Instant::now);
    let effect_assets = request
        .sfx_enabled
        .then_some(effect_name)
        .flatten()
        .map(|name| crate::formats::load_effect_assets_optional(request.resources, name))
        .transpose()?
        .flatten()
        .unwrap_or_else(|| crate::formats::EffectAssets {
            clips: std::collections::BTreeMap::new(),
        });
    let bindings = if request.sfx_enabled {
        crate::offline::effect_clip_bindings(&package.watch)?
    } else {
        std::collections::BTreeMap::new()
    };
    let (events, scheduled, loop_starts, loop_stops) = collect_audio_requests(&expected_frames);
    let effect_mix = if request.sfx_enabled {
        crate::audio::mix_effects_wav(
            &effect_assets,
            &bindings,
            &events,
            &scheduled,
            &loop_starts,
            &loop_stops,
            range.start_time(),
            range.frame_count as f64 / f64::from(range.fps),
        )?
    } else {
        crate::audio::SfxMixResult {
            wav: None,
            warnings: vec![],
        }
    };
    for warning in &effect_mix.warnings {
        eprintln!("warning: {warning}");
    }
    let effect_audio = effect_mix.wav;
    let first_pass_hash = hash_sequence(
        &expected_frames
            .iter()
            .map(|frame| frame.rgb_sha1.clone())
            .collect::<Vec<_>>(),
    );
    if let Some(p) = profile.as_mut() {
        p.record(
            "Audio/SFX preparation",
            audio_start.map_or(Duration::ZERO, |t| t.elapsed()),
        );
    }

    let ffmpeg_start = request.profile.then(Instant::now);
    let installation = ffmpeg::resolve()?;
    let mut command = Command::new(&installation.ffmpeg);
    command.args([
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
    ]);
    if request.bgm_enabled {
        command.arg("-i").arg(request.music);
    }
    let sfx_file = effect_audio
        .as_ref()
        .map(|bytes| {
            let mut file = tempfile::Builder::new()
                .suffix(".wav")
                .tempfile()
                .context("creating temporary SFX mix")?;
            file.write_all(bytes).context("writing temporary SFX mix")?;
            Ok::<_, anyhow::Error>(file)
        })
        .transpose()?;
    if let Some(file) = &sfx_file {
        command.arg("-i").arg(file.path());
    }
    command.args(audio_map_args(
        request.bgm_enabled,
        sfx_file.is_some(),
        audio,
    ));
    command.args([
        "-vf",
        "format=yuv420p",
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
        "-movflags",
        "+faststart",
    ]);
    if request.bgm_enabled || sfx_file.is_some() {
        command.args(["-c:a", "aac", "-b:a", "192k"]);
    }
    command.arg(&request.output);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("launching FFmpeg at {}", installation.ffmpeg.display()))?;
    if let Some(p) = profile.as_mut() {
        p.record(
            "FFmpeg startup",
            ffmpeg_start.map_or(Duration::ZERO, |t| t.elapsed()),
        );
    }

    let streaming_session_start = request.profile.then(Instant::now);
    let streaming_session = make_session()?;
    if let Some(p) = profile.as_mut() {
        p.record(
            "Streaming session initialization",
            streaming_session_start.map_or(Duration::ZERO, |t| t.elapsed()),
        );
        p.begin_render();
    }

    let stdin = child
        .stdin
        .take()
        .context("FFmpeg process did not expose its raw-video input")?;
    let actual_frames = match stream_frames(
        streaming_session,
        range,
        stdin,
        &expected_frames,
        &package.watch,
        request.trace_entity_id,
        profile.as_mut(),
    ) {
        Ok(frames) => frames,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error).context("streaming deterministic RGB frames to FFmpeg");
        }
    };
    if let Some(p) = profile.as_mut() {
        p.finish_render();
    }
    let finalize_start = request.profile.then(Instant::now);
    let output = child
        .wait_with_output()
        .context("waiting for FFmpeg export")?;
    if let Some(p) = profile.as_mut() {
        p.record(
            "FFmpeg finalization",
            finalize_start.map_or(Duration::ZERO, |t| t.elapsed()),
        );
    }
    if !output.status.success() {
        bail!(
            "FFmpeg failed with status {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let probe_start = request.profile.then(Instant::now);
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
    if let Some(p) = profile.as_ref() {
        let _ = p;
    }
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
        request.bgm_enabled || sfx_file.is_some(),
    )?;
    if let Some(p) = profile.as_mut() {
        p.record(
            "FFprobe validation",
            probe_start.map_or(Duration::ZERO, |t| t.elapsed()),
        );
    }
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
    if let (Some(p), Some(start)) = (profile.as_mut(), export_start) {
        p.record("Total export", start.elapsed());
        p.print(start.elapsed());
    }
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
    mut profile: Option<&mut crate::profiling::ProfileCollector>,
) -> Result<Vec<FrameDiagnostic>> {
    (0..range.frame_count)
        .map(|segment_index| {
            let frame_index = range.index(segment_index)?;
            let frame = session.render_global_frame(frame_index)?;
            if let (Some(p), Some(frame_profile)) = (profile.as_deref_mut(), frame.profile.as_ref())
            {
                p.record_preflight(frame_profile);
            }
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
    profile: Option<&mut crate::profiling::ProfileCollector>,
) -> Result<Vec<FrameDiagnostic>> {
    if expected_frames.len() != range.frame_count as usize {
        bail!("determinism preflight returned an unexpected number of frames");
    }
    let mut actual_frames = Vec::with_capacity(expected_frames.len());
    let mut profile = profile;
    for (segment_index, expected) in expected_frames.iter().enumerate() {
        let frame_wall = profile.as_ref().map(|_| Instant::now());
        let global_index = range.index(segment_index as u64)?;
        let frame = session.render_global_frame(global_index)?;
        let frame_profile = frame.profile.clone();
        let diagnostic_start = profile.as_ref().map(|_| Instant::now());
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
        if let (Some(start), Some(p)) = (diagnostic_start, profile.as_deref_mut()) {
            p.record("Frame diagnostics/hash", start.elapsed());
        }
        let handoff_start = profile.as_ref().map(|_| Instant::now());
        stdin
            .write_all(&frame.rgb)
            .with_context(|| format!("writing raw RGB frame {global_index} to FFmpeg"))?;
        if let (Some(start), Some(p), Some(frame_profile)) = (
            handoff_start,
            profile.as_deref_mut(),
            frame_profile.as_ref(),
        ) {
            p.record_frame(
                "stream",
                global_index,
                range.time(global_index),
                frame_profile,
                start.elapsed(),
                frame_wall.map_or(frame_profile.total, |t| t.elapsed()),
            );
        }
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
        vm_evaluations: frame.report.vm_evaluations,
        runtime_entity_count: frame.report.runtime_entity_count,
        active_entity_count: frame.report.active_entity_count,
        draw_count: frame.report.display_list.sprites.len(),
        raw_draw_sha1: hash_draw_commands(&frame.report.display_list),
        rgb_sha1: hash_rgb(&frame.rgb),
        target_entity_draws,
        audio_events: frame.report.audio_events.clone(),
        scheduled_effects: frame.report.scheduled_effects.clone(),
        scheduled_looped_effects: frame.report.scheduled_looped_effects.clone(),
        scheduled_looped_effect_stops: frame.report.scheduled_looped_effect_stops.clone(),
    }
}

fn collect_audio_requests(
    frames: &[FrameDiagnostic],
) -> (
    Vec<AudioEffectEvent>,
    Vec<ScheduledEffect>,
    Vec<ScheduledLoopedEffect>,
    Vec<ScheduledLoopedEffectStop>,
) {
    let mut events = Vec::new();
    let mut scheduled = Vec::new();
    let mut loop_starts = Vec::new();
    let mut loop_stops = Vec::new();
    for frame in frames {
        events.extend(frame.audio_events.iter().cloned());
        scheduled.extend(frame.scheduled_effects.iter().cloned());
        loop_starts.extend(frame.scheduled_looped_effects.iter().cloned());
        loop_stops.extend(frame.scheduled_looped_effect_stops.iter().cloned());
    }
    (events, scheduled, loop_starts, loop_stops)
}

fn hash_sequence(hashes: &[String]) -> String {
    let mut digest = Sha1::new();
    for hash in hashes {
        digest.update(hash.as_bytes());
    }
    hex::encode(digest.finalize())
}

fn audio_map_args(has_bgm: bool, has_sfx: bool, audio: AudioWindow) -> Vec<String> {
    match (has_bgm, has_sfx) {
        (true, true) => vec![
            "-map".into(),
            "0:v:0".into(),
            "-map".into(),
            "[aout]".into(),
            "-filter_complex".into(),
            format!(
                "[1:a]{}[bgm];[bgm][2:a]amix=inputs=2:duration=first:normalize=0[aout]",
                audio_filter(audio)
            ),
        ],
        (true, false) => vec![
            "-map".into(),
            "0:v:0".into(),
            "-map".into(),
            "1:a:0".into(),
            "-af".into(),
            audio_filter(audio),
        ],
        (false, true) => vec!["-map".into(), "0:v:0".into(), "-map".into(), "1:a:0".into()],
        (false, false) => vec!["-map".into(), "0:v:0".into()],
    }
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
    has_audio: bool,
) -> Result<()> {
    let streams = probe
        .get("streams")
        .and_then(Value::as_array)
        .context("FFprobe JSON has no streams array")?;
    let video = streams.iter().find(|s| s["codec_type"] == "video");
    let audio = streams.iter().find(|s| s["codec_type"] == "audio");
    let video = video.context("export has no video stream")?;
    if has_audio && audio.is_none() {
        bail!("export has no audio stream");
    }
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
    if streams.len() != if has_audio { 2 } else { 1 } {
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
    if video["codec_name"].as_str().is_none_or(str::is_empty) {
        bail!("FFprobe did not report video codec");
    }
    if video["codec_name"] != "h264" || video["pix_fmt"] != "yuv420p" {
        bail!("FFprobe codecs or pixel format differ from the configured MP4 output");
    }
    if has_audio && audio.is_some_and(|stream| stream["codec_name"] != "aac") {
        bail!("FFprobe audio codec differs from the configured MP4 output");
    }
    let duration = probe["format"]["duration"]
        .as_str()
        .and_then(|value| value.parse::<f64>().ok())
        .context("FFprobe format duration is missing or invalid")?;
    if (duration - expected_duration).abs() > 1.0 / f64::from(fps) {
        bail!("FFprobe output duration differs from the requested frame sequence");
    }
    for stream in std::iter::once(video).chain(audio.into_iter()) {
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

    #[test]
    fn bgm_is_mapped_alone_without_sfx_and_mixed_when_sfx_exists() {
        let window = AudioWindow {
            source_start: 1.25,
            leading_silence: 0.0,
        };
        let without_sfx = audio_map_args(true, false, window).join(" ");
        assert!(without_sfx.contains("1:a:0"));
        assert!(without_sfx.contains("atrim=start=1.250000000000"));

        let with_sfx = audio_map_args(true, true, window).join(" ");
        assert!(with_sfx.contains("[1:a]"));
        assert!(with_sfx.contains("[2:a]"));
        assert!(with_sfx.contains("amix=inputs=2"));
        assert!(with_sfx.contains("atrim=start=1.250000000000"));

        let sfx_only = audio_map_args(false, true, window).join(" ");
        assert!(sfx_only.contains("1:a:0"));
        assert!(!sfx_only.contains("atrim"));
        assert_eq!(audio_map_args(false, false, window), ["-map", "0:v:0"]);
    }
}
