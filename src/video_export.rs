//! FFmpeg-backed media export. Deterministic frames come from `offline`.

use crate::runtime::{
    AudioEffectEvent, ScheduledEffect, ScheduledLoopedEffect, ScheduledLoopedEffectStop,
};
use crate::{
    ffmpeg,
    offline::{
        hash_draw_commands, hash_rgb, AudioWindow, EventSession, FrameRange, FrameSession,
        PreparedFrame, RenderedFrame,
    },
    render::RenderConfig,
};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
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
    pub sfx_event_sha1: String,
    pub sfx_event_counts: SfxEventCounts,
    pub event_pass_runtime_frames: u64,
    pub event_pass_callbacks: u64,
    pub event_pass_evaluations: u64,
    pub stream_runtime_frames: u64,
    pub stream_callbacks: u64,
    pub stream_evaluations: u64,
    pub concurrent_sfx_prepass_used: bool,
    pub rgb_spool_bytes: u64,
    pub frame_pipeline: Option<FramePipelineStats>,
    pub deterministic_frame_hashes_match: Option<bool>,
    pub first_pass_hash: Option<String>,
    pub second_pass_hash: String,
    pub frame_diagnostics: Vec<FrameDiagnostic>,
    pub probe: Value,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct FramePipelineStats {
    pub queue_capacity: usize,
    pub queue_high_water: usize,
    pub producer_watch_ms: f64,
    pub producer_snapshot_ms: f64,
    pub producer_blocked_ms: f64,
    pub queue_residence_ms: f64,
    pub consumer_waiting_ms: f64,
    pub render_ms: f64,
    pub output_ms: f64,
    pub first_frame_latency_ms: f64,
    pub render_phase_ms: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct SfxEventCounts {
    pub audio_events: u64,
    pub scheduled_effects: u64,
    pub loop_starts: u64,
    pub loop_stops: u64,
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
    pub runtime_frame_count: u64,
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
    pub validate_determinism: bool,
    pub concurrent_sfx_prepass: bool,
    pub frame_pipeline: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrepassPlan {
    None,
    EventsOnly,
    FullValidation,
}

fn prepass_plan(sfx_enabled: bool, validate_determinism: bool) -> PrepassPlan {
    if validate_determinism {
        PrepassPlan::FullValidation
    } else if sfx_enabled {
        PrepassPlan::EventsOnly
    } else {
        PrepassPlan::None
    }
}

const MAX_CONCURRENT_RGB_SPOOL_BYTES: u64 = 2 * 1024 * 1024 * 1024;

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
        validate_determinism: config.validate_determinism,
        concurrent_sfx_prepass: config.concurrent_sfx_prepass,
        frame_pipeline: config.frame_pipeline,
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
    let plan = prepass_plan(request.sfx_enabled, request.validate_determinism);
    let rgb_spool_bytes = range
        .frame_count
        .checked_mul(u64::from(request.width))
        .and_then(|bytes| bytes.checked_mul(u64::from(request.height)))
        .and_then(|bytes| bytes.checked_mul(3))
        .context("concurrent RGB spool size overflows")?;
    let concurrent_sfx_prepass_used = plan == PrepassPlan::EventsOnly
        && request.concurrent_sfx_prepass
        && !request.frame_pipeline
        && rgb_spool_bytes <= MAX_CONCURRENT_RGB_SPOOL_BYTES;
    if plan == PrepassPlan::EventsOnly
        && request.concurrent_sfx_prepass
        && !request.frame_pipeline
        && !concurrent_sfx_prepass_used
    {
        eprintln!(
            "warning: concurrent SFX export needs {rgb_spool_bytes} bytes of temporary RGB storage (limit {MAX_CONCURRENT_RGB_SPOOL_BYTES}); using sequential SFX event collection"
        );
    }
    let mut expected_frames = None;
    let mut concurrent_output: Option<(Vec<FrameDiagnostic>, tempfile::NamedTempFile)> = None;
    let mut audio_requests = empty_audio_requests();
    let mut event_pass_runtime_frames = 0;
    let mut event_pass_callbacks = 0;
    let mut event_pass_evaluations = 0;
    if plan == PrepassPlan::FullValidation {
        let initialization = make_session()?;
        if let Some(p) = profile.as_mut() {
            p.record(
                "Validation session initialization",
                startup_start.map_or(Duration::ZERO, |t| t.elapsed()),
            );
        }
        let preflight_start = request.profile.then(Instant::now);
        let frames = render_diagnostic_pass(
            initialization,
            range,
            &package.watch,
            request.trace_entity_id,
            profile.as_mut(),
        )?;
        if let Some(p) = profile.as_mut() {
            let elapsed = preflight_start.map_or(Duration::ZERO, |t| t.elapsed());
            p.record("Full determinism validation preflight", elapsed);
            p.record_preflight_wrapper(elapsed);
        }
        audio_requests = collect_audio_requests(&frames);
        expected_frames = Some(frames);
    } else if let Some(p) = profile.as_mut() {
        p.record(
            "Initialization/startup",
            startup_start.map_or(Duration::ZERO, |t| t.elapsed()),
        );
    }
    if plan == PrepassPlan::EventsOnly {
        let prepass_start = request.profile.then(Instant::now);
        let event_session = EventSession::new_configured(
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
            request.profile,
            request.trace_entity_id.is_some(),
        )?;
        if concurrent_sfx_prepass_used {
            let session_start = request.profile.then(Instant::now);
            let stream_session = make_session()?;
            if let Some(p) = profile.as_mut() {
                p.record(
                    "Concurrent session initialization",
                    session_start.map_or(Duration::ZERO, |start| start.elapsed()),
                );
            }
            let ConcurrentStreamOutput {
                events,
                frames,
                spool,
                profile: stream_profile,
            } = concurrent_event_and_render(
                event_session,
                stream_session,
                range,
                &package.watch,
                request.trace_entity_id,
                profile.take(),
                request.profile,
            )?;
            audio_requests = events.requests;
            event_pass_runtime_frames = events.runtime_frames;
            event_pass_callbacks = events.callbacks;
            event_pass_evaluations = events.evaluations;
            profile = stream_profile;
            if let Some(p) = profile.as_mut() {
                for (runtime, wall) in &events.profiles {
                    p.record_event_only(runtime, *wall);
                }
                p.record("SFX event-only worker wall", events.elapsed);
                p.record(
                    "Concurrent SFX/render wall",
                    prepass_start.map_or(Duration::ZERO, |s| s.elapsed()),
                );
            }
            concurrent_output = Some((frames, spool));
        } else {
            let output = collect_event_only_requests(
                event_session,
                range,
                request.profile,
                &AtomicU8::new(0),
            )?;
            audio_requests = output.requests;
            event_pass_runtime_frames = output.runtime_frames;
            event_pass_callbacks = output.callbacks;
            event_pass_evaluations = output.evaluations;
            if let Some(p) = profile.as_mut() {
                for (runtime, wall) in &output.profiles {
                    p.record_event_only(runtime, *wall);
                }
                p.record(
                    "SFX runtime/event-only prepass",
                    prepass_start.map_or(Duration::ZERO, |s| s.elapsed()),
                );
            }
        }
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
    let sfx_event_sha1 = hash_audio_requests(&audio_requests)?;
    let sfx_event_counts = count_audio_requests(&audio_requests);
    let (events, scheduled, loop_starts, loop_stops) = audio_requests;
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
    let first_pass_hash = expected_frames.as_ref().map(|frames| {
        hash_sequence(
            &frames
                .iter()
                .map(|frame| frame.rgb_sha1.clone())
                .collect::<Vec<_>>(),
        )
    });
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
    ]);
    if let Some((_, spool)) = concurrent_output.as_ref() {
        command.arg("-i").arg(spool.path());
    } else {
        command.args(["-i", "pipe:0"]);
    }
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
    // Drain FFmpeg diagnostics to a file so a noisy failure cannot fill a
    // stderr pipe while the render worker is blocked writing video frames.
    let mut ffmpeg_stderr_capture =
        tempfile::tempfile().context("creating temporary FFmpeg diagnostic capture")?;
    let ffmpeg_stderr_writer = ffmpeg_stderr_capture
        .try_clone()
        .context("cloning FFmpeg diagnostic capture handle")?;
    let mut child = command
        .stdin(if concurrent_output.is_some() {
            Stdio::null()
        } else {
            Stdio::piped()
        })
        .stdout(Stdio::null())
        .stderr(Stdio::from(ffmpeg_stderr_writer))
        .spawn()
        .with_context(|| format!("launching FFmpeg at {}", installation.ffmpeg.display()))?;
    if let Some(p) = profile.as_mut() {
        p.record(
            "FFmpeg startup",
            ffmpeg_start.map_or(Duration::ZERO, |t| t.elapsed()),
        );
    }

    let mut frame_pipeline_stats = None;
    let (actual_frames, _spool_guard) = if let Some((frames, spool)) = concurrent_output.take() {
        (frames, Some(spool))
    } else {
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
        let frames = if request.frame_pipeline {
            match stream_frames_pipelined(
                streaming_session,
                range,
                stdin,
                expected_frames.as_deref(),
                &package.watch,
                request.trace_entity_id,
                request.profile,
            ) {
                Ok(result) => {
                    frame_pipeline_stats = Some(result.stats.clone());
                    if let Some(p) = profile.as_mut() {
                        p.set_pipeline_steady_window(
                            range.frame_count.saturating_sub(1),
                            Duration::from_secs_f64(
                                (result.stats.render_phase_ms
                                    - result.stats.first_frame_latency_ms)
                                    .max(0.0)
                                    / 1000.0,
                            ),
                        );
                        eprintln!("Pipeline stage totals overlap across frames; render includes nested raster/GPU wait/readback/unpack/UI. Producer blocked time and queue residence are correlated and must not be summed.");
                        for record in &result.records {
                            if let Some(frame_profile) = record.profile.as_ref() {
                                p.record_frame(
                                    "stream",
                                    record.global_index,
                                    record.timeline,
                                    frame_profile,
                                    record.diagnostics,
                                    record.output,
                                    record.wall,
                                    "FFmpeg handoff",
                                );
                                p.finish_stream_frame_profile(
                                    record.wall,
                                    Duration::ZERO,
                                    frame_profile.total,
                                    record.diagnostics,
                                    record.output,
                                );
                            }
                        }
                        eprintln!(
                            "Frame pipeline: queue capacity {}/{}, Watch {:.1}ms, snapshot {:.1}ms, producer blocked {:.1}ms, queue residence {:.1}ms, consumer waiting {:.1}ms, render {:.1}ms, FFmpeg output {:.1}ms, first-frame latency {:.1}ms",
                            result.stats.queue_high_water,
                            result.stats.queue_capacity,
                            result.stats.producer_watch_ms,
                            result.stats.producer_snapshot_ms,
                            result.stats.producer_blocked_ms,
                            result.stats.queue_residence_ms,
                            result.stats.consumer_waiting_ms,
                            result.stats.render_ms,
                            result.stats.output_ms,
                            result.stats.first_frame_latency_ms,
                        );
                    }
                    result.diagnostics
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error).context("streaming pipelined RGB frames to FFmpeg");
                }
            }
        } else {
            match stream_frames(
                streaming_session,
                range,
                stdin,
                expected_frames.as_deref(),
                &package.watch,
                request.trace_entity_id,
                profile.as_mut(),
                "FFmpeg handoff",
                "FFmpeg final input flush/close",
                None,
            ) {
                Ok(frames) => frames,
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error).context("streaming deterministic RGB frames to FFmpeg");
                }
            }
        };
        if let Some(p) = profile.as_mut() {
            p.finish_render();
        }
        (frames, None)
    };
    let finalize_start = request.profile.then(Instant::now);
    let output = child
        .wait_with_output()
        .context("waiting for FFmpeg export")?;
    ffmpeg_stderr_capture
        .seek(SeekFrom::Start(0))
        .context("rewinding FFmpeg diagnostic capture")?;
    let mut ffmpeg_stderr = String::new();
    ffmpeg_stderr_capture
        .read_to_string(&mut ffmpeg_stderr)
        .context("reading FFmpeg diagnostic capture")?;
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
            ffmpeg_stderr.trim()
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
    let stream_runtime_frames = actual_frames
        .iter()
        .map(|frame| frame.runtime_frame_count)
        .sum();
    let stream_callbacks = actual_frames
        .iter()
        .map(|frame| frame.callback_count as u64)
        .sum();
    let stream_evaluations = actual_frames.iter().map(|frame| frame.vm_evaluations).sum();
    let deterministic_frame_hashes_match = expected_frames
        .as_ref()
        .map(|expected| compare_frame_diagnostics(expected, &actual_frames));
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
        sfx_event_sha1,
        sfx_event_counts,
        event_pass_runtime_frames,
        event_pass_callbacks,
        event_pass_evaluations,
        stream_runtime_frames,
        stream_callbacks,
        stream_evaluations,
        concurrent_sfx_prepass_used,
        rgb_spool_bytes: if concurrent_sfx_prepass_used {
            rgb_spool_bytes
        } else {
            0
        },
        frame_pipeline: frame_pipeline_stats,
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
            let loop_start = profile.as_ref().map(|_| Instant::now());
            let frame_index = range.index(segment_index)?;
            let frame = session.render_global_frame(frame_index)?;
            let diagnostic_start = profile.as_ref().map(|_| Instant::now());
            let diagnostic = frame_diagnostic(
                segment_index,
                frame_index,
                range.time(frame_index),
                &frame,
                watch,
                target_entity_id,
            );
            let diagnostic_time = diagnostic_start.map_or(Duration::ZERO, |start| start.elapsed());
            if let (Some(p), Some(frame_profile), Some(start)) =
                (profile.as_deref_mut(), frame.profile.as_ref(), loop_start)
            {
                p.record_preflight(frame_profile, diagnostic_time, start.elapsed());
            }
            Ok(diagnostic)
        })
        .collect()
}

fn stream_frames(
    mut session: FrameSession<'_>,
    range: FrameRange,
    mut stdin: impl Write,
    expected_frames: Option<&[FrameDiagnostic]>,
    watch: &crate::watch::WatchData,
    target_entity_id: Option<usize>,
    profile: Option<&mut crate::profiling::ProfileCollector>,
    handoff_label: &'static str,
    flush_label: &'static str,
    cancellation: Option<&AtomicU8>,
) -> Result<Vec<FrameDiagnostic>> {
    if expected_frames.is_some_and(|frames| frames.len() != range.frame_count as usize) {
        bail!("determinism validation returned an unexpected number of frames");
    }
    let mut actual_frames = Vec::with_capacity(range.frame_count as usize);
    let mut profile = profile;
    for segment_index in 0..range.frame_count as usize {
        if cancellation.is_some_and(|token| token.load(Ordering::Acquire) != 0) {
            bail!("stream rendering cancelled after concurrent SFX prepass failure");
        }
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
        if let Some(expected) = expected_frames.map(|frames| &frames[segment_index]) {
            if !frame_hashes_match(expected, &diagnostic) {
                bail!("frame {global_index} changed between deterministic passes");
            }
        }
        let diagnostic_time = diagnostic_start.map_or(Duration::ZERO, |start| start.elapsed());
        let handoff_start = profile.as_ref().map(|_| Instant::now());
        stdin
            .write_all(&frame.rgb)
            .with_context(|| format!("writing raw RGB frame {global_index} to export output"))?;
        let handoff_time = handoff_start.map_or(Duration::ZERO, |start| start.elapsed());
        actual_frames.push(diagnostic);
        if let (Some(start), Some(p), Some(frame_profile)) =
            (frame_wall, profile.as_deref_mut(), frame_profile.as_ref())
        {
            let bookkeeping_start = Instant::now();
            p.record_frame(
                "stream",
                global_index,
                range.time(global_index),
                frame_profile,
                diagnostic_time,
                handoff_time,
                start.elapsed(),
                handoff_label,
            );
            let bookkeeping = bookkeeping_start.elapsed();
            p.finish_stream_frame_profile(
                start.elapsed(),
                bookkeeping,
                frame_profile.total,
                diagnostic_time,
                handoff_time,
            );
        }
    }
    let flush_start = profile.as_ref().map(|_| Instant::now());
    stdin.flush().context("flushing raw RGB frames to FFmpeg")?;
    drop(stdin);
    if let (Some(start), Some(p)) = (flush_start, profile.as_deref_mut()) {
        p.record(flush_label, start.elapsed());
    }
    Ok(actual_frames)
}

fn frame_hashes_match(first: &FrameDiagnostic, second: &FrameDiagnostic) -> bool {
    first.raw_draw_sha1 == second.raw_draw_sha1 && first.rgb_sha1 == second.rgb_sha1
}

const FRAME_QUEUE_CAPACITY: usize = 2;

struct PreparedJob {
    local_index: u64,
    global_index: u64,
    timeline: f64,
    ready_at: Instant,
    frame: PreparedFrame,
}

struct PipelineFrameRecord {
    global_index: u64,
    timeline: f64,
    profile: Option<crate::offline::FrameStageProfile>,
    diagnostics: Duration,
    output: Duration,
    wall: Duration,
}

struct PipelineWorkerResult {
    diagnostics: Vec<FrameDiagnostic>,
    records: Vec<PipelineFrameRecord>,
    consumer_waiting: Duration,
    queue_residence: Duration,
    render: Duration,
    output: Duration,
    first_frame_latency: Duration,
}

struct PipelineResult {
    diagnostics: Vec<FrameDiagnostic>,
    stats: FramePipelineStats,
    records: Vec<PipelineFrameRecord>,
}

fn stream_frames_pipelined(
    mut session: FrameSession<'_>,
    range: FrameRange,
    mut stdin: impl Write + Send,
    expected_frames: Option<&[FrameDiagnostic]>,
    watch: &crate::watch::WatchData,
    target_entity_id: Option<usize>,
    profile_enabled: bool,
) -> Result<PipelineResult> {
    if expected_frames.is_some_and(|frames| frames.len() != range.frame_count as usize) {
        bail!("determinism validation returned an unexpected number of frames");
    }
    let resources = session.render_resources();
    let (sender, receiver) = std::sync::mpsc::sync_channel::<PreparedJob>(FRAME_QUEUE_CAPACITY);
    let outstanding = AtomicUsize::new(0);
    let high_water = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let pipeline_start = Instant::now();
    let worker_watch = watch;
    let worker_cancelled = &cancelled;
    let worker_outstanding = &outstanding;

    std::thread::scope(|scope| {
        let worker = scope.spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut records = Vec::with_capacity(range.frame_count as usize);
                let mut diagnostics = Vec::with_capacity(range.frame_count as usize);
                let mut consumer_waiting = Duration::ZERO;
                let mut queue_residence = Duration::ZERO;
                let mut render_total = Duration::ZERO;
                let mut output_total = Duration::ZERO;
                let mut first_frame_latency = Duration::ZERO;
                loop {
                    let wait_start = Instant::now();
                    let job = match receiver.recv() {
                        Ok(job) => job,
                        Err(_) => break,
                    };
                    consumer_waiting += wait_start.elapsed();
                    worker_outstanding.fetch_sub(1, Ordering::AcqRel);
                    if worker_cancelled.load(Ordering::Acquire) {
                        break;
                    }
                    let wall_start = job.ready_at;
                    let render_start = Instant::now();
                    let mut rendered = crate::offline::render_prepared_frame(
                        resources.clone(),
                        profile_enabled,
                        job.frame,
                    )?;
                    let render_time = render_start.elapsed();
                    render_total += render_time;
                    queue_residence += render_start.duration_since(job.ready_at);
                    if let Some(frame_profile) = rendered.profile.as_mut() {
                        frame_profile.total = frame_profile.vm
                            + frame_profile.preparation
                            + frame_profile.backend_wall
                            + frame_profile.runtime_ui
                            + frame_profile.framebuffer_copy;
                    }
                    let diagnostic_start = Instant::now();
                    let diagnostic = frame_diagnostic(
                        job.local_index,
                        job.global_index,
                        job.timeline,
                        &rendered,
                        worker_watch,
                        target_entity_id,
                    );
                    let diagnostics_time = diagnostic_start.elapsed();
                    if let Some(expected) =
                        expected_frames.map(|frames| &frames[job.local_index as usize])
                    {
                        if !frame_hashes_match(expected, &diagnostic) {
                            bail!(
                                "frame {} changed between deterministic passes",
                                job.global_index
                            );
                        }
                    }
                    let output_start = Instant::now();
                    stdin.write_all(&rendered.rgb).with_context(|| {
                        format!("writing raw RGB frame {} to FFmpeg", job.global_index)
                    })?;
                    let output_time = output_start.elapsed();
                    output_total += output_time;
                    if records.is_empty() {
                        first_frame_latency = pipeline_start.elapsed();
                    }
                    records.push(PipelineFrameRecord {
                        global_index: job.global_index,
                        timeline: job.timeline,
                        profile: rendered.profile,
                        diagnostics: diagnostics_time,
                        output: output_time,
                        wall: wall_start.elapsed(),
                    });
                    diagnostics.push(diagnostic);
                }
                stdin
                    .flush()
                    .context("flushing pipelined RGB frames to FFmpeg")?;
                drop(stdin);
                Ok(PipelineWorkerResult {
                    diagnostics,
                    records,
                    consumer_waiting,
                    queue_residence,
                    render: render_total,
                    output: output_total,
                    first_frame_latency,
                })
            }))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("frame render/output worker panicked")));
            if result.is_err() {
                worker_cancelled.store(true, Ordering::Release);
            }
            result
        });

        let mut producer_result: Result<(Duration, Duration, Duration)> =
            Ok((Duration::ZERO, Duration::ZERO, Duration::ZERO));
        for local_index in 0..range.frame_count {
            if cancelled.load(Ordering::Acquire) {
                producer_result = Err(anyhow::anyhow!(
                    "frame pipeline cancelled after worker failure"
                ));
                break;
            }
            let global_index = match range.index(local_index) {
                Ok(index) => index,
                Err(error) => {
                    producer_result = Err(error);
                    break;
                }
            };
            let prepared = match session.prepare_global_frame(global_index) {
                Ok(prepared) => prepared,
                Err(error) => {
                    producer_result =
                        Err(error).context("preparing Watch frame for render pipeline");
                    break;
                }
            };
            let (watch_time, snapshot_time) = prepared.producer_timings();
            let current = producer_result.as_mut().unwrap();
            current.0 += watch_time;
            current.1 += snapshot_time;
            let ready_at = Instant::now();
            let was_full = outstanding.load(Ordering::Acquire) >= FRAME_QUEUE_CAPACITY;
            let send_start = Instant::now();
            outstanding.fetch_add(1, Ordering::AcqRel);
            let send_result = sender.send(PreparedJob {
                local_index,
                global_index,
                timeline: range.time(global_index),
                ready_at,
                frame: prepared,
            });
            let send_time = send_start.elapsed();
            if send_result.is_err() {
                outstanding.fetch_sub(1, Ordering::AcqRel);
                producer_result = Err(anyhow::anyhow!(
                    "frame render/output worker closed the queue"
                ));
                break;
            }
            if was_full {
                producer_result.as_mut().unwrap().2 += send_time;
            }
            high_water.fetch_max(
                outstanding
                    .load(Ordering::Acquire)
                    .min(FRAME_QUEUE_CAPACITY),
                Ordering::AcqRel,
            );
        }
        drop(sender);
        if producer_result.is_err() {
            cancelled.store(true, Ordering::Release);
        }
        let worker_result = worker
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("frame render/output worker panicked")));
        match worker_result {
            Err(error) => Err(error),
            Ok(worker_result) => {
                let (watch_time, snapshot_time, blocked_time) = producer_result?;
                if worker_result.records.len() != range.frame_count as usize {
                    bail!("frame pipeline stopped before all frames were rendered");
                }
                let render_phase = pipeline_start.elapsed();
                Ok(PipelineResult {
                    diagnostics: worker_result.diagnostics,
                    stats: FramePipelineStats {
                        queue_capacity: FRAME_QUEUE_CAPACITY,
                        queue_high_water: high_water.load(Ordering::Acquire),
                        producer_watch_ms: watch_time.as_secs_f64() * 1000.0,
                        producer_snapshot_ms: snapshot_time.as_secs_f64() * 1000.0,
                        producer_blocked_ms: blocked_time.as_secs_f64() * 1000.0,
                        queue_residence_ms: worker_result.queue_residence.as_secs_f64() * 1000.0,
                        consumer_waiting_ms: worker_result.consumer_waiting.as_secs_f64() * 1000.0,
                        render_ms: worker_result.render.as_secs_f64() * 1000.0,
                        output_ms: worker_result.output.as_secs_f64() * 1000.0,
                        first_frame_latency_ms: worker_result.first_frame_latency.as_secs_f64()
                            * 1000.0,
                        render_phase_ms: render_phase.as_secs_f64() * 1000.0,
                    },
                    records: worker_result.records,
                })
            }
        }
    })
}

fn compare_frame_diagnostics(first: &[FrameDiagnostic], second: &[FrameDiagnostic]) -> bool {
    first.len() == second.len()
        && first
            .iter()
            .zip(second)
            .all(|(a, b)| frame_hashes_match(a, b))
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
        runtime_frame_count: frame.report.runtime_profile.frame_count,
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

type AudioRequests = (
    Vec<AudioEffectEvent>,
    Vec<ScheduledEffect>,
    Vec<ScheduledLoopedEffect>,
    Vec<ScheduledLoopedEffectStop>,
);

fn empty_audio_requests() -> AudioRequests {
    (Vec::new(), Vec::new(), Vec::new(), Vec::new())
}

fn hash_audio_requests(requests: &AudioRequests) -> Result<String> {
    let bytes = serde_json::to_vec(requests).context("serializing collected SFX event sequence")?;
    Ok(hex::encode(Sha1::digest(bytes)))
}

fn count_audio_requests(requests: &AudioRequests) -> SfxEventCounts {
    SfxEventCounts {
        audio_events: requests.0.len() as u64,
        scheduled_effects: requests.1.len() as u64,
        loop_starts: requests.2.len() as u64,
        loop_stops: requests.3.len() as u64,
    }
}

fn append_report_events(requests: &mut AudioRequests, report: &crate::watch_runtime::FrameReport) {
    requests.0.extend(report.audio_events.iter().cloned());
    requests.1.extend(report.scheduled_effects.iter().cloned());
    requests
        .2
        .extend(report.scheduled_looped_effects.iter().cloned());
    requests
        .3
        .extend(report.scheduled_looped_effect_stops.iter().cloned());
}

fn collect_audio_requests(frames: &[FrameDiagnostic]) -> AudioRequests {
    let mut requests = empty_audio_requests();
    for frame in frames {
        requests.0.extend(frame.audio_events.iter().cloned());
        requests.1.extend(frame.scheduled_effects.iter().cloned());
        requests
            .2
            .extend(frame.scheduled_looped_effects.iter().cloned());
        requests
            .3
            .extend(frame.scheduled_looped_effect_stops.iter().cloned());
    }
    requests
}

fn collect_event_only_requests(
    mut session: EventSession<'_>,
    range: FrameRange,
    profile_enabled: bool,
    cancellation: &AtomicU8,
) -> Result<EventPassOutput> {
    let pass_start = Instant::now();
    let mut requests = empty_audio_requests();
    let mut profiles = Vec::new();
    let (mut runtime_frames, mut callbacks, mut evaluations) = (0, 0, 0);
    for segment_index in 0..range.frame_count {
        if cancellation.load(Ordering::Acquire) != 0 {
            bail!("SFX event pass cancelled after concurrent render failure");
        }
        let loop_start = Instant::now();
        let frame_index = range.index(segment_index)?;
        let report = session.advance_global_frame(frame_index)?;
        append_report_events(&mut requests, &report);
        runtime_frames += report.runtime_profile.frame_count;
        callbacks += report.callbacks.len() as u64;
        evaluations += report.vm_evaluations;
        if profile_enabled {
            profiles.push((report.runtime_profile.clone(), loop_start.elapsed()));
        }
    }
    Ok(EventPassOutput {
        requests,
        profiles,
        elapsed: pass_start.elapsed(),
        runtime_frames,
        callbacks,
        evaluations,
    })
}

struct EventPassOutput {
    requests: AudioRequests,
    profiles: Vec<(crate::watch_runtime::FrameRuntimeProfile, Duration)>,
    elapsed: Duration,
    runtime_frames: u64,
    callbacks: u64,
    evaluations: u64,
}

struct ConcurrentStreamOutput {
    events: EventPassOutput,
    frames: Vec<FrameDiagnostic>,
    spool: tempfile::NamedTempFile,
    profile: Option<crate::profiling::ProfileCollector>,
}

fn mark_first_failure(cancellation: &AtomicU8, source: u8) {
    let _ = cancellation.compare_exchange(0, source, Ordering::AcqRel, Ordering::Acquire);
}

fn concurrent_event_and_render(
    event_session: EventSession<'_>,
    stream_session: FrameSession<'_>,
    range: FrameRange,
    watch: &crate::watch::WatchData,
    target_entity_id: Option<usize>,
    profile: Option<crate::profiling::ProfileCollector>,
    profile_enabled: bool,
) -> Result<ConcurrentStreamOutput> {
    const EVENT_FAILURE: u8 = 1;
    const RENDER_FAILURE: u8 = 2;
    let cancellation = AtomicU8::new(0);
    std::thread::scope(|scope| {
        let event_worker = scope.spawn(|| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                collect_event_only_requests(event_session, range, profile_enabled, &cancellation)
            }))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("SFX event worker panicked")));
            if result.is_err() {
                mark_first_failure(&cancellation, EVENT_FAILURE);
            }
            result
        });
        let render_worker = scope.spawn(|| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut spool = tempfile::Builder::new()
                    .suffix(".rgb")
                    .tempfile()
                    .context("creating bounded-memory RGB spool for concurrent SFX export")?;
                let mut profile = profile;
                if let Some(profile) = profile.as_mut() {
                    profile.begin_render();
                }
                let frames = stream_frames(
                    stream_session,
                    range,
                    &mut spool,
                    None,
                    watch,
                    target_entity_id,
                    profile.as_mut(),
                    "RGB spool buffer",
                    "RGB spool flush",
                    Some(&cancellation),
                )?;
                if let Some(profile) = profile.as_mut() {
                    profile.finish_render();
                }
                spool
                    .as_file_mut()
                    .seek(SeekFrom::Start(0))
                    .context("rewinding concurrent RGB spool")?;
                Ok((frames, spool, profile))
            }))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("stream render worker panicked")));
            if result.is_err() {
                mark_first_failure(&cancellation, RENDER_FAILURE);
            }
            result
        });

        let events = event_worker
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("SFX event worker panicked")));
        let rendered = render_worker
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("stream render worker panicked")));
        match cancellation.load(Ordering::Acquire) {
            EVENT_FAILURE => Err(events
                .err()
                .unwrap_or_else(|| anyhow::anyhow!("SFX event worker failed without an error"))),
            RENDER_FAILURE => Err(rendered.err().unwrap_or_else(|| {
                anyhow::anyhow!("stream render worker failed without an error")
            })),
            _ => {
                let events = events?;
                let (frames, spool, profile) = rendered?;
                Ok(ConcurrentStreamOutput {
                    events,
                    frames,
                    spool,
                    profile,
                })
            }
        }
    })
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

    struct SlowSink(Duration);

    impl Write for SlowSink {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            std::thread::sleep(self.0);
            Ok(buffer.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct BrokenSink;

    impl Write for BrokenSink {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "simulated FFmpeg stdin closure",
            ))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct PanickingSink;

    impl Write for PanickingSink {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            panic!("simulated output worker panic")
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct DiscardSink;

    impl Write for DiscardSink {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            Ok(buffer.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn no_sfx_production_export_has_no_prepass() {
        assert_eq!(prepass_plan(false, false), PrepassPlan::None);
        assert_eq!(prepass_plan(true, false), PrepassPlan::EventsOnly);
        assert_eq!(prepass_plan(false, true), PrepassPlan::FullValidation);
        assert_eq!(prepass_plan(true, true), PrepassPlan::FullValidation);
    }

    #[test]
    fn explicit_determinism_validation_compares_draw_and_rgb_hashes() {
        let diagnostic = |draw: &str, rgb: &str| FrameDiagnostic {
            output_local_frame_index: 0,
            global_frame_index: 0,
            requested_timeline_time: 0.0,
            runtime_timeline: None,
            runtime_update: [0.0; 4],
            runtime_update_after_callbacks: [0.0; 4],
            timescale: 1.0,
            callback_count: 0,
            vm_evaluations: 0,
            runtime_frame_count: 0,
            runtime_entity_count: 0,
            active_entity_count: 0,
            draw_count: 0,
            raw_draw_sha1: draw.to_owned(),
            rgb_sha1: rgb.to_owned(),
            target_entity_draws: Vec::new(),
            audio_events: Vec::new(),
            scheduled_effects: Vec::new(),
            scheduled_looped_effects: Vec::new(),
            scheduled_looped_effect_stops: Vec::new(),
        };
        let first = [diagnostic("draw-a", "rgb-a")];
        assert!(compare_frame_diagnostics(
            &first,
            &[diagnostic("draw-a", "rgb-a")]
        ));
        assert!(!compare_frame_diagnostics(
            &first,
            &[diagnostic("draw-b", "rgb-a")]
        ));
        assert!(!compare_frame_diagnostics(
            &first,
            &[diagnostic("draw-a", "rgb-b")]
        ));
        assert!(!compare_frame_diagnostics(&first, &[]));
    }

    #[test]
    fn streaming_without_preflight_keeps_known_baumkuchen_frame_hash() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let package =
            crate::formats::load_engine(&repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"))
                .unwrap();
        let level = crate::formats::load_level(&repo.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz")).unwrap();
        let resources = repo.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp");
        let defaults: std::collections::BTreeMap<_, _> =
            package.metadata.resource_defaults().into_iter().collect();
        let session = FrameSession::new_configured(
            &package.watch,
            &package.rom,
            &package.configuration,
            &level,
            &resources,
            defaults.get("skins").unwrap(),
            defaults.get("backgrounds").map(String::as_str),
            defaults.get("effects").map(String::as_str),
            defaults.get("particles").map(String::as_str),
            320,
            180,
            12,
            &[(1, 10.8), (22, 1.0)],
            false,
            crate::render_ui::RendererUiConfig::default(),
            0.0,
            0.25,
            crate::render::RenderBackend::Wgpu,
            true,
            false,
        )
        .unwrap();
        let range = FrameRange {
            first_index: 0,
            frame_count: 1,
            fps: 12,
        };
        let mut output = Vec::new();
        let frames = stream_frames(
            session,
            range,
            &mut output,
            None,
            &package.watch,
            None,
            None,
            "FFmpeg handoff",
            "FFmpeg final input flush/close",
            None,
        )
        .unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(
            frames[0].rgb_sha1,
            "d45f286a15fac6be9b51ded7d0015356b8b92a72"
        );
        assert_eq!(output.len(), 320 * 180 * 3);
    }

    #[test]
    fn bounded_pipeline_matches_sequential_hashes_and_preserves_fifo() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let package =
            crate::formats::load_engine(&repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"))
                .unwrap();
        let level = crate::formats::load_level(&repo.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz")).unwrap();
        let resources = repo.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp");
        let defaults: std::collections::BTreeMap<_, _> =
            package.metadata.resource_defaults().into_iter().collect();
        let skin = defaults.get("skins").unwrap();
        let background = defaults.get("backgrounds").map(String::as_str);
        let effects = defaults.get("effects").map(String::as_str);
        let particles = defaults.get("particles").map(String::as_str);
        let options = [(1, 10.8), (22, 1.0)];
        let make_session = |ui| {
            FrameSession::new_configured(
                &package.watch,
                &package.rom,
                &package.configuration,
                &level,
                &resources,
                skin,
                background,
                effects,
                particles,
                64,
                36,
                12,
                &options,
                true,
                ui,
                0.0,
                10.0,
                crate::render::RenderBackend::Cpu,
                false,
                false,
            )
            .unwrap()
        };
        let range = FrameRange {
            first_index: 0,
            frame_count: 24,
            fps: 12,
        };
        let mut sequential = make_session(crate::render_ui::RendererUiConfig::default());
        let sequential_frames: Vec<_> = (0..range.frame_count)
            .map(|local| {
                let global = range.index(local).unwrap();
                let frame = sequential.render_global_frame(global).unwrap();
                (global, hash_rgb(&frame.rgb), frame.report.vm_evaluations)
            })
            .collect();
        let mut output = Vec::new();
        let pipelined = stream_frames_pipelined(
            make_session(crate::render_ui::RendererUiConfig::default()),
            range,
            &mut output,
            None,
            &package.watch,
            None,
            false,
        )
        .unwrap();
        let pipelined_frames: Vec<_> = pipelined
            .diagnostics
            .iter()
            .map(|frame| {
                (
                    frame.global_frame_index,
                    frame.rgb_sha1.clone(),
                    frame.vm_evaluations,
                )
            })
            .collect();
        assert_eq!(sequential_frames, pipelined_frames);
        assert_eq!(
            pipelined
                .diagnostics
                .iter()
                .map(|frame| frame.output_local_frame_index)
                .collect::<Vec<_>>(),
            (0..range.frame_count).collect::<Vec<_>>()
        );
        assert!(pipelined.stats.queue_high_water > 0);
        assert!(pipelined.stats.queue_high_water <= pipelined.stats.queue_capacity);
        assert_eq!(output.len(), 64 * 36 * 3 * range.frame_count as usize);

        let short_range = FrameRange {
            frame_count: 8,
            ..range
        };
        let slowed = stream_frames_pipelined(
            make_session(crate::render_ui::RendererUiConfig::default()),
            short_range,
            SlowSink(Duration::from_millis(20)),
            None,
            &package.watch,
            None,
            false,
        )
        .unwrap();
        assert_eq!(slowed.stats.queue_high_water, FRAME_QUEUE_CAPACITY);
        assert!(slowed.stats.producer_blocked_ms > 0.0);

        let broken = stream_frames_pipelined(
            make_session(crate::render_ui::RendererUiConfig::default()),
            short_range,
            BrokenSink,
            None,
            &package.watch,
            None,
            false,
        );
        assert!(format!("{:#}", broken.err().unwrap()).contains("simulated FFmpeg stdin closure"));

        let panicked = stream_frames_pipelined(
            make_session(crate::render_ui::RendererUiConfig::default()),
            short_range,
            PanickingSink,
            None,
            &package.watch,
            None,
            false,
        );
        assert!(panicked
            .err()
            .unwrap()
            .to_string()
            .contains("worker panicked"));

        let mut failing_ui = crate::render_ui::RendererUiConfig::default();
        failing_ui.enabled = true;
        failing_ui.primary_metric.provider = crate::render_ui::UiValueProvider::EngineMetric {
            key: "missing".to_owned(),
        };
        let producer_failure = stream_frames_pipelined(
            make_session(failing_ui),
            FrameRange {
                frame_count: 1,
                ..range
            },
            Vec::new(),
            None,
            &package.watch,
            None,
            false,
        );
        assert!(format!("{:#}", producer_failure.err().unwrap())
            .contains("no authoritative Watch source"));
    }

    #[test]
    fn baumkuchen_1080p_pipeline_preserves_known_rgb_sfx_and_workload_hashes() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let package =
            crate::formats::load_engine(&repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"))
                .unwrap();
        let level = crate::formats::load_level(&repo.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz")).unwrap();
        let resources = repo.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp");
        let defaults: std::collections::BTreeMap<_, _> =
            package.metadata.resource_defaults().into_iter().collect();
        let skin = defaults.get("skins").unwrap();
        let background = defaults.get("backgrounds").map(String::as_str);
        let effects = defaults.get("effects").map(String::as_str);
        let particles = defaults.get("particles").map(String::as_str);
        let options = [(1, 10.8), (22, 1.0)];
        let range = FrameRange {
            first_index: 0,
            frame_count: 120,
            fps: 60,
        };
        let session = FrameSession::new_configured(
            &package.watch,
            &package.rom,
            &package.configuration,
            &level,
            &resources,
            skin,
            background,
            effects,
            particles,
            1920,
            1080,
            60,
            &options,
            true,
            crate::render_ui::RendererUiConfig::default(),
            0.0,
            2.0,
            crate::render::RenderBackend::Wgpu,
            true,
            false,
        )
        .unwrap();
        let output = stream_frames_pipelined(
            session,
            range,
            DiscardSink,
            None,
            &package.watch,
            None,
            false,
        )
        .unwrap();
        let rgb_sequence = hash_sequence(
            &output
                .diagnostics
                .iter()
                .map(|frame| frame.rgb_sha1.clone())
                .collect::<Vec<_>>(),
        );
        assert_eq!(rgb_sequence, "6e8363f683b1bae0bc8b8f568633699fad668286");
        assert_eq!(
            output
                .diagnostics
                .iter()
                .map(|frame| frame.runtime_frame_count)
                .sum::<u64>(),
            120
        );
        assert_eq!(
            output
                .diagnostics
                .iter()
                .map(|frame| frame.callback_count as u64)
                .sum::<u64>(),
            10_898
        );
        assert_eq!(
            output
                .diagnostics
                .iter()
                .map(|frame| frame.vm_evaluations)
                .sum::<u64>(),
            12_783_093
        );

        let event_session = EventSession::new_configured(
            &package.watch,
            &package.rom,
            &package.configuration,
            &level,
            &resources,
            skin,
            background,
            effects,
            particles,
            1920,
            1080,
            60,
            &options,
            true,
            true,
            false,
        )
        .unwrap();
        let events =
            collect_event_only_requests(event_session, range, false, &AtomicU8::new(0)).unwrap();
        assert_eq!(
            hash_audio_requests(&events.requests).unwrap(),
            "be53125a6dd38ad4b59dc7fc15b2ca3ca99c1808"
        );
        assert_eq!(
            count_audio_requests(&events.requests),
            SfxEventCounts {
                audio_events: 0,
                scheduled_effects: 615,
                loop_starts: 89,
                loop_stops: 89,
            }
        );
        assert_eq!(events.runtime_frames, 120);
        assert_eq!(events.callbacks, 10_898);
        assert_eq!(events.evaluations, 12_783_093);
    }

    #[test]
    fn concurrent_and_sequential_baumkuchen_event_and_frame_sequences_match() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let package =
            crate::formats::load_engine(&repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"))
                .unwrap();
        let level = crate::formats::load_level(&repo.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz")).unwrap();
        let resources = repo.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp");
        let defaults: std::collections::BTreeMap<_, _> =
            package.metadata.resource_defaults().into_iter().collect();
        let skin = defaults.get("skins").unwrap();
        let background = defaults.get("backgrounds").map(String::as_str);
        let effects = defaults.get("effects").map(String::as_str);
        let particles = defaults.get("particles").map(String::as_str);
        let options = [(1, 10.8), (22, 1.0)];
        let make_event_session = || {
            EventSession::new_configured(
                &package.watch,
                &package.rom,
                &package.configuration,
                &level,
                &resources,
                skin,
                background,
                effects,
                particles,
                64,
                36,
                12,
                &options,
                true,
                false,
                false,
            )
            .unwrap()
        };
        let make_frame_session = || {
            FrameSession::new_configured(
                &package.watch,
                &package.rom,
                &package.configuration,
                &level,
                &resources,
                skin,
                background,
                effects,
                particles,
                64,
                36,
                12,
                &options,
                true,
                crate::render_ui::RendererUiConfig::default(),
                0.0,
                10.0,
                crate::render::RenderBackend::Cpu,
                false,
                false,
            )
            .unwrap()
        };
        let range = FrameRange {
            first_index: 0,
            frame_count: 120,
            fps: 12,
        };
        let sequential_events =
            collect_event_only_requests(make_event_session(), range, false, &AtomicU8::new(0))
                .unwrap();
        let mut sequential_session = make_frame_session();
        let sequential_hashes: Vec<_> = (0..range.frame_count)
            .map(|index| {
                let frame = sequential_session
                    .render_global_frame(range.index(index).unwrap())
                    .unwrap();
                hash_rgb(&frame.rgb)
            })
            .collect();

        let concurrent = concurrent_event_and_render(
            make_event_session(),
            make_frame_session(),
            range,
            &package.watch,
            None,
            None,
            false,
        )
        .unwrap();
        assert_eq!(
            hash_audio_requests(&sequential_events.requests).unwrap(),
            hash_audio_requests(&concurrent.events.requests).unwrap()
        );
        assert_eq!(
            count_audio_requests(&sequential_events.requests),
            count_audio_requests(&concurrent.events.requests)
        );
        assert!(
            count_audio_requests(&concurrent.events.requests).audio_events
                + count_audio_requests(&concurrent.events.requests).scheduled_effects
                + count_audio_requests(&concurrent.events.requests).loop_starts
                + count_audio_requests(&concurrent.events.requests).loop_stops
                > 0
        );
        assert_eq!(
            sequential_events.runtime_frames,
            concurrent.events.runtime_frames
        );
        assert_eq!(sequential_events.callbacks, concurrent.events.callbacks);
        assert_eq!(sequential_events.evaluations, concurrent.events.evaluations);
        let concurrent_hashes: Vec<_> = concurrent
            .frames
            .iter()
            .map(|frame| frame.rgb_sha1.clone())
            .collect();
        assert_eq!(sequential_hashes, concurrent_hashes);
        assert_eq!(
            hash_sequence(&sequential_hashes),
            hash_sequence(&concurrent_hashes)
        );
        assert_eq!(
            fs::metadata(concurrent.spool.path()).unwrap().len(),
            64 * 36 * 3 * 120
        );
    }

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
