//! Low overhead wall-clock aggregation for opt-in render-video profiling.

use crate::offline::FrameStageProfile;
use std::{collections::BTreeMap, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimingSummary {
    pub total_ms: f64,
    pub mean_ms: f64,
    pub median_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}

pub fn summarize_durations(values: &[Duration]) -> Option<TimingSummary> {
    if values.is_empty() {
        return None;
    }
    let mut ms: Vec<f64> = values.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    ms.sort_by(f64::total_cmp);
    let total_ms = ms.iter().sum();
    let percentile = |p: f64| {
        ms[((ms.len() as f64 * p).ceil() as usize)
            .saturating_sub(1)
            .min(ms.len() - 1)]
    };
    let median_ms = if ms.len() % 2 == 0 {
        (ms[ms.len() / 2 - 1] + ms[ms.len() / 2]) * 0.5
    } else {
        ms[ms.len() / 2]
    };
    Some(TimingSummary {
        total_ms,
        mean_ms: total_ms / ms.len() as f64,
        median_ms,
        p95_ms: percentile(0.95),
        max_ms: *ms.last().unwrap_or(&0.0),
    })
}

#[derive(Debug)]
pub(crate) struct ProfileCollector {
    timings: BTreeMap<&'static str, Vec<Duration>>,
    details: bool,
    frame_count: u64,
    entities: u64,
    skin_draws: u64,
    particles: u64,
    gpu_draws: u64,
    atlas_uploads: u64,
    readback_bytes: u64,
    adapter: Option<String>,
    resolution: (u32, u32),
    backend: &'static str,
    fps: u32,
    render_start: Option<std::time::Instant>,
    pipeline_steady_window: Option<(u64, Duration)>,
    stream_runtime_frames: u64,
    stream_watch_callbacks: u64,
    stream_watch_evaluations: u64,
    stream_function_dispatches: u64,
    stream_memory_entries: u64,
    preflight_runtime_frames: u64,
    preflight_watch_callbacks: u64,
    preflight_watch_evaluations: u64,
    preflight_function_dispatches: u64,
    preflight_memory_entries: u64,
    event_only_runtime_frames: u64,
    event_only_output_frames: u64,
    event_only_watch_callbacks: u64,
    event_only_watch_evaluations: u64,
}
impl ProfileCollector {
    pub(crate) fn new(
        details: bool,
        backend: &'static str,
        resolution: (u32, u32),
        fps: u32,
    ) -> Self {
        Self {
            timings: BTreeMap::new(),
            details,
            frame_count: 0,
            entities: 0,
            skin_draws: 0,
            particles: 0,
            gpu_draws: 0,
            atlas_uploads: 0,
            readback_bytes: 0,
            adapter: None,
            resolution,
            backend,
            fps,
            render_start: None,
            pipeline_steady_window: None,
            stream_runtime_frames: 0,
            stream_watch_callbacks: 0,
            stream_watch_evaluations: 0,
            stream_function_dispatches: 0,
            stream_memory_entries: 0,
            preflight_runtime_frames: 0,
            preflight_watch_callbacks: 0,
            preflight_watch_evaluations: 0,
            preflight_function_dispatches: 0,
            preflight_memory_entries: 0,
            event_only_runtime_frames: 0,
            event_only_output_frames: 0,
            event_only_watch_callbacks: 0,
            event_only_watch_evaluations: 0,
        }
    }
    pub(crate) fn begin_render(&mut self) {
        self.render_start = Some(std::time::Instant::now())
    }
    pub(crate) fn set_pipeline_steady_window(&mut self, frames: u64, wall: Duration) {
        self.pipeline_steady_window = Some((frames, wall));
    }
    pub(crate) fn finish_render(&mut self) {
        if let Some(t) = self.render_start.take() {
            let wall = t.elapsed();
            let frame_walls = self
                .timings
                .get("Total frame wall")
                .map(|values| values.iter().copied().sum::<Duration>())
                .unwrap_or_default();
            let flush = self
                .timings
                .get("FFmpeg final input flush/close")
                .and_then(|values| values.last())
                .copied()
                .unwrap_or_default();
            self.record(
                "Render orchestration residual",
                wall.saturating_sub(frame_walls + flush),
            );
            self.record("Render phase", wall)
        }
    }
    pub(crate) fn record(&mut self, name: &'static str, d: Duration) {
        self.timings.entry(name).or_default().push(d)
    }
    pub(crate) fn record_frame(
        &mut self,
        phase: &str,
        frame: u64,
        time: f64,
        profile: &FrameStageProfile,
        diagnostics: Duration,
        handoff: Duration,
        wall: Duration,
        handoff_label: &'static str,
    ) {
        self.record("Watch VM", profile.vm);
        self.record("Watch runtime frame", profile.runtime.frame);
        self.record("Watch preprocess", profile.runtime.preprocess);
        self.record("Watch update-spawn", profile.runtime.update_spawn);
        self.record("Watch scheduling", profile.runtime.scheduling);
        self.record("Watch activation", profile.runtime.activation);
        self.record("Watch UpdateSequential", profile.runtime.update_sequential);
        self.record("Watch UpdateParallel", profile.runtime.update_parallel);
        self.record(
            "Watch report materialization",
            profile.runtime.report_materialization,
        );
        self.record(
            "Watch event aggregation",
            profile.runtime.stepper_event_aggregation,
        );
        self.record(
            "Callback VM construction",
            profile.runtime.callback_vm_construction,
        );
        self.record(
            "Callback context setup",
            profile.runtime.callback_context_setup,
        );
        self.record(
            "Callback memory setup",
            profile.runtime.callback_memory_setup,
        );
        self.record("Callback evaluator", profile.runtime.callback_execute);
        self.record("Callback commit", profile.runtime.callback_commit);
        self.record("Watch runtime construction", profile.runtime_construction);
        self.record(
            "Atlas identity preparation",
            profile.atlas_identity_preparation,
        );
        if self.frame_count == 0 {
            self.record("First-frame Watch VM", profile.vm);
            self.record("First-frame runtime total", profile.runtime.frame);
            self.record("First-frame preprocess", profile.runtime.preprocess);
            self.record("First-frame UpdateSpawn", profile.runtime.update_spawn);
            self.record("First-frame scheduling", profile.runtime.scheduling);
            self.record("First-frame activation", profile.runtime.activation);
            self.record(
                "First-frame UpdateSequential",
                profile.runtime.update_sequential,
            );
            self.record(
                "First-frame UpdateParallel",
                profile.runtime.update_parallel,
            );
            self.record(
                "First-frame report build",
                profile.runtime.report_materialization,
            );
            self.record(
                "First-frame runtime construction",
                profile.runtime_construction,
            );
            self.record(
                "First-frame callback construction",
                profile.runtime.callback_vm_construction,
            );
            self.record(
                "First-frame callback context",
                profile.runtime.callback_context_setup,
            );
            self.record(
                "First-frame callback memory",
                profile.runtime.callback_memory_setup,
            );
            self.record(
                "First-frame callback setup",
                profile.runtime.callback_vm_construction
                    + profile.runtime.callback_context_setup
                    + profile.runtime.callback_memory_setup,
            );
            self.record(
                "First-frame callback evaluator",
                profile.runtime.callback_execute,
            );
            self.record(
                "First-frame callback commit",
                profile.runtime.callback_commit,
            );
        } else {
            self.record("Steady-state Watch VM", profile.vm);
            self.record(
                "Steady callback context",
                profile.runtime.callback_context_setup,
            );
            self.record(
                "Steady callback memory",
                profile.runtime.callback_memory_setup,
            );
            self.record(
                "Steady-state UpdateSequential",
                profile.runtime.update_sequential,
            );
            self.record(
                "Steady-state UpdateParallel",
                profile.runtime.update_parallel,
            );
            self.record(
                "Steady callback setup",
                profile.runtime.callback_vm_construction
                    + profile.runtime.callback_context_setup
                    + profile.runtime.callback_memory_setup,
            );
            self.record(
                "Steady callback evaluator",
                profile.runtime.callback_execute,
            );
            self.record("Steady callback commit", profile.runtime.callback_commit);
        }
        self.stream_runtime_frames += profile.runtime.frame_count;
        self.stream_watch_callbacks += profile.runtime.callbacks;
        self.stream_watch_evaluations += profile.runtime.evaluations;
        self.stream_function_dispatches += profile.runtime.function_dispatches;
        self.stream_memory_entries += profile.runtime.memory_entries_copied;
        self.record("Draw preparation", profile.preparation);
        if self.backend == "wgpu" {
            self.record("GPU draw preparation", profile.gpu_draw_preparation);
            self.record("GPU target setup", profile.gpu_render_target_setup);
            self.record(
                "GPU initial RGB to RGBA",
                profile.gpu_initial_rgba_conversion,
            );
            self.record(
                "GPU initial framebuffer upload",
                profile.gpu_initial_framebuffer_upload,
            );
            self.record("GPU atlas content hashing", profile.gpu_atlas_hashing);
            self.record("GPU atlas cache/upload", profile.gpu_atlas_cache_upload);
            self.record(
                "GPU readback buffer setup",
                profile.gpu_readback_buffer_setup,
            );
            self.record("GPU encode/submit", profile.gpu_encode_submit);
            self.record("GPU wait/map", profile.gpu_wait_map);
            self.record("GPU readback unpack", profile.framebuffer_unpack);
            self.record("Framebuffer PPM packaging", profile.framebuffer_packaging);
            let gpu_measured = profile.gpu_draw_preparation
                + profile.gpu_render_target_setup
                + profile.gpu_initial_rgba_conversion
                + profile.gpu_initial_framebuffer_upload
                + profile.gpu_atlas_hashing
                + profile.gpu_atlas_cache_upload
                + profile.gpu_readback_buffer_setup
                + profile.gpu_encode_submit
                + profile.gpu_wait_map
                + profile.framebuffer_unpack
                + profile.framebuffer_packaging;
            self.record(
                "GPU backend residual",
                profile.backend_wall.saturating_sub(gpu_measured),
            );
        } else {
            self.record("CPU backend render/composite", profile.backend_wall);
        }
        self.record("Runtime UI", profile.runtime_ui);
        self.record("Final framebuffer RGB copy", profile.framebuffer_copy);
        let frame_measured = profile.vm
            + profile.preparation
            + profile.backend_wall
            + profile.runtime_ui
            + profile.framebuffer_copy;
        self.record(
            "FrameSession residual",
            profile.total.saturating_sub(frame_measured),
        );
        self.record("Frame diagnostics/hash", diagnostics);
        self.record(handoff_label, handoff);
        self.record(
            "Export loop residual",
            wall.saturating_sub(profile.total + diagnostics + handoff),
        );
        self.record("Total frame wall", wall);
        if self.frame_count == 0 {
            self.record("First frame (includes VM init)", wall);
        }
        self.frame_count += 1;
        self.entities += profile.entities as u64;
        self.skin_draws += profile.skin_draws as u64;
        self.particles += profile.particle_draws as u64;
        self.gpu_draws += profile.gpu_draw_calls as u64;
        self.atlas_uploads += profile.atlas_uploads as u64;
        self.readback_bytes += profile.readback_bytes;
        if self.adapter.is_none() {
            self.adapter = profile.adapter.clone()
        }
        if self.details {
            eprintln!("profile frame: {phase} #{frame} t={time:.3}s total={:.2}ms vm={:.2} prep={:.2} encode={:.2} wait/map={:.2} unpack={:.2} output-handoff={:.2}",wall.as_secs_f64()*1e3,profile.vm.as_secs_f64()*1e3,profile.preparation.as_secs_f64()*1e3,profile.gpu_encode_submit.as_secs_f64()*1e3,profile.gpu_wait_map.as_secs_f64()*1e3,profile.framebuffer_unpack.as_secs_f64()*1e3,handoff.as_secs_f64()*1e3)
        } else if self.frame_count % 30 == 0 {
            eprintln!(
                "profile progress: {phase} frames={} latest={:.1}ms draws={} particles={}",
                self.frame_count,
                wall.as_secs_f64() * 1000.0,
                profile.skin_draws,
                profile.particle_draws
            )
        }
    }
    pub(crate) fn record_preflight(
        &mut self,
        profile: &FrameStageProfile,
        diagnostics: Duration,
        loop_wall: Duration,
    ) {
        self.atlas_uploads += profile.atlas_uploads as u64;
        self.record(
            "Atlas identity preparation",
            profile.atlas_identity_preparation,
        );
        self.record("Preflight frame wall", loop_wall);
        self.record("Preflight Watch VM", profile.vm);
        self.record("Preflight frame preparation", profile.preparation);
        if self.backend == "wgpu" {
            self.record(
                "Preflight GPU draw preparation",
                profile.gpu_draw_preparation,
            );
            self.record(
                "Preflight GPU target setup",
                profile.gpu_render_target_setup,
            );
            self.record(
                "Preflight GPU initial RGB to RGBA",
                profile.gpu_initial_rgba_conversion,
            );
            self.record(
                "Preflight GPU initial framebuffer upload",
                profile.gpu_initial_framebuffer_upload,
            );
            self.record(
                "Preflight GPU atlas content hashing",
                profile.gpu_atlas_hashing,
            );
            self.record(
                "Preflight GPU atlas cache/upload",
                profile.gpu_atlas_cache_upload,
            );
            self.record(
                "Preflight GPU readback buffer setup",
                profile.gpu_readback_buffer_setup,
            );
            self.record("Preflight GPU encode/submit", profile.gpu_encode_submit);
            self.record("Preflight GPU wait/map", profile.gpu_wait_map);
            self.record("Preflight GPU readback unpack", profile.framebuffer_unpack);
            self.record(
                "Preflight framebuffer PPM packaging",
                profile.framebuffer_packaging,
            );
            let gpu_measured = profile.gpu_draw_preparation
                + profile.gpu_render_target_setup
                + profile.gpu_initial_rgba_conversion
                + profile.gpu_initial_framebuffer_upload
                + profile.gpu_atlas_hashing
                + profile.gpu_atlas_cache_upload
                + profile.gpu_readback_buffer_setup
                + profile.gpu_encode_submit
                + profile.gpu_wait_map
                + profile.framebuffer_unpack
                + profile.framebuffer_packaging;
            self.record(
                "Preflight GPU backend residual",
                profile.backend_wall.saturating_sub(gpu_measured),
            );
        } else {
            self.record(
                "Preflight CPU backend render/composite",
                profile.backend_wall,
            );
        }
        self.record("Preflight Runtime UI", profile.runtime_ui);
        self.record(
            "Preflight final framebuffer RGB copy",
            profile.framebuffer_copy,
        );
        self.record("Preflight diagnostics/hash", diagnostics);
        let frame_measured = profile.vm
            + profile.preparation
            + profile.backend_wall
            + profile.runtime_ui
            + profile.framebuffer_copy;
        self.record(
            "Preflight FrameSession residual",
            profile.total.saturating_sub(frame_measured),
        );
        self.record(
            "Preflight render-loop residual",
            loop_wall.saturating_sub(profile.total + diagnostics),
        );
        self.record("Preflight Watch runtime frame", profile.runtime.frame);
        self.record("Preflight Watch preprocess", profile.runtime.preprocess);
        self.record("Preflight Watch update-spawn", profile.runtime.update_spawn);
        self.record("Preflight Watch scheduling", profile.runtime.scheduling);
        self.record("Preflight Watch activation", profile.runtime.activation);
        self.record(
            "Preflight Watch UpdateSequential",
            profile.runtime.update_sequential,
        );
        self.record(
            "Preflight Watch UpdateParallel",
            profile.runtime.update_parallel,
        );
        self.record(
            "Preflight report materialization",
            profile.runtime.report_materialization,
        );
        self.record(
            "Preflight event aggregation",
            profile.runtime.stepper_event_aggregation,
        );
        self.record(
            "Preflight callback VM construction",
            profile.runtime.callback_vm_construction,
        );
        self.record(
            "Preflight callback context setup",
            profile.runtime.callback_context_setup,
        );
        self.record(
            "Preflight callback memory setup",
            profile.runtime.callback_memory_setup,
        );
        self.record(
            "Preflight callback evaluator",
            profile.runtime.callback_execute,
        );
        self.record("Preflight callback commit", profile.runtime.callback_commit);
        self.record(
            "Preflight runtime construction",
            profile.runtime_construction,
        );
        self.preflight_runtime_frames += profile.runtime.frame_count;
        self.preflight_watch_callbacks += profile.runtime.callbacks;
        self.preflight_watch_evaluations += profile.runtime.evaluations;
        self.preflight_function_dispatches += profile.runtime.function_dispatches;
        self.preflight_memory_entries += profile.runtime.memory_entries_copied;
        if self.adapter.is_none() {
            self.adapter = profile.adapter.clone();
        }
    }
    pub(crate) fn record_preflight_wrapper(&mut self, total: Duration) {
        let frames = self
            .timings
            .get("Preflight frame wall")
            .map(|values| values.iter().copied().sum::<Duration>())
            .unwrap_or_default();
        self.record("Preflight collector/wrapper", total.saturating_sub(frames));
    }
    pub(crate) fn record_event_only(
        &mut self,
        runtime: &crate::watch_runtime::FrameRuntimeProfile,
        loop_wall: Duration,
    ) {
        self.record("SFX event-only Watch runtime", runtime.frame);
        self.record("SFX event-only Watch preprocess", runtime.preprocess);
        self.record("SFX event-only render-loop wall", loop_wall);
        self.event_only_runtime_frames += runtime.frame_count;
        self.event_only_output_frames += 1;
        self.event_only_watch_callbacks += runtime.callbacks;
        self.event_only_watch_evaluations += runtime.evaluations;
    }
    pub(crate) fn finish_stream_frame_profile(
        &mut self,
        wall: Duration,
        profile_bookkeeping: Duration,
        render: Duration,
        diagnostics: Duration,
        handoff: Duration,
    ) {
        if let Some(values) = self.timings.get_mut("Total frame wall") {
            if let Some(last) = values.last_mut() {
                *last = wall;
            }
        }
        if let Some(values) = self.timings.get_mut("Export loop residual") {
            if let Some(last) = values.last_mut() {
                *last = wall.saturating_sub(render + diagnostics + handoff + profile_bookkeeping);
            }
        }
        self.record("Profile bookkeeping", profile_bookkeeping);
    }
    pub(crate) fn print(&self, total: Duration) {
        eprintln!("\nSono-Renderer profiling\nBackend: {}\nAdapter: {}\nResolution: {}x{}\nStreamed frames: {}\n",self.backend,self.adapter.as_deref().unwrap_or("not reported (CPU backend)"),self.resolution.0,self.resolution.1,self.frame_count);
        eprintln!(
            "{: <28} {:>10} {:>10} {:>10} {:>10} {:>10}",
            "", "total", "mean", "p50", "p95", "max"
        );
        for (name, v) in &self.timings {
            if let Some(s) = summarize_durations(v) {
                eprintln!(
                    "{name:<28} {:>9.3}s {:>9.3}ms {:>9.3}ms {:>9.3}ms {:>9.3}ms",
                    s.total_ms / 1000.0,
                    s.mean_ms,
                    s.median_ms,
                    s.p95_ms,
                    s.max_ms
                )
            }
        }
        let watch_vm_total_ms = self
            .timings
            .get("Watch VM")
            .map(|values| values.iter().map(|d| d.as_secs_f64() * 1000.0).sum::<f64>())
            .unwrap_or_default();
        eprintln!("\nWatch runtime phase shares (callback rows are nested within lifecycle stages; denominator is streamed Watch VM time):");
        for name in [
            "Watch preprocess",
            "Watch update-spawn",
            "Watch scheduling",
            "Watch activation",
            "Watch UpdateSequential",
            "Watch UpdateParallel",
            "Watch report materialization",
            "Watch event aggregation",
            "Callback VM construction",
            "Callback context setup",
            "Callback memory setup",
            "Callback evaluator",
            "Callback commit",
        ] {
            let total_ms = self
                .timings
                .get(name)
                .map(|values| values.iter().map(|d| d.as_secs_f64() * 1000.0).sum::<f64>())
                .unwrap_or_default();
            eprintln!(
                "{name:<34} {total_ms:>9.2}ms {:>6.1}%",
                total_ms / watch_vm_total_ms.max(f64::MIN_POSITIVE) * 100.0
            );
        }
        let n = self.frame_count.max(1) as f64;
        eprintln!("GPU execution timestamp: not collected (no additional query/readback added)\nEntities/frame: {:.1}\nSkin draws/frame: {:.1}\nParticle draws/frame: {:.1}\nGPU draws/frame: {:.1}\nAtlas uploads/cache misses: {}\nReadback/frame: {:.0} bytes\nStreaming runtime frames/callbacks/evaluations: {}/{} / {}\nEvent-only output frames/runtime frames/callbacks/evaluations: {}/{}/{}/{}\nEvent-only prepass GPU frames: 0\nFull-validation runtime frames/callbacks/evaluations: {}/{} / {}",self.entities as f64/n,self.skin_draws as f64/n,self.particles as f64/n,self.gpu_draws as f64/n,self.atlas_uploads,self.readback_bytes as f64/n,self.stream_runtime_frames,self.stream_watch_callbacks,self.stream_watch_evaluations,self.event_only_output_frames,self.event_only_runtime_frames,self.event_only_watch_callbacks,self.event_only_watch_evaluations,self.preflight_runtime_frames,self.preflight_watch_callbacks,self.preflight_watch_evaluations);
        let render = self
            .timings
            .get("Render phase")
            .map(|v| v.iter().map(|d| d.as_secs_f64()).sum::<f64>())
            .unwrap_or_default();
        let throughput = self.frame_count as f64 / render.max(f64::MIN_POSITIVE);
        let realtime = throughput / f64::from(self.fps.max(1));
        let frame_walls = self
            .timings
            .get("Total frame wall")
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let (steady_count, steady_seconds) = self
            .pipeline_steady_window
            .map(|(frames, wall)| (frames as f64, wall.as_secs_f64()))
            .unwrap_or_else(|| {
                let count = frame_walls.len().saturating_sub(1);
                let seconds = frame_walls
                    .iter()
                    .skip(1)
                    .map(Duration::as_secs_f64)
                    .sum::<f64>();
                (count as f64, seconds)
            });
        let steady_fps = steady_count / steady_seconds.max(f64::MIN_POSITIVE);
        eprintln!("Render phase: {render:.3}s\nTotal export: {:.3}s\nEffective throughput (including first frame): {throughput:.2} fps ({realtime:.2}x realtime)\nSteady-state frame throughput (excluding first frame): {steady_fps:.2} fps",total.as_secs_f64());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timing_summary_uses_nearest_rank_and_median() {
        let x: Vec<_> = [1, 2, 3, 4, 100]
            .into_iter()
            .map(Duration::from_millis)
            .collect();
        let s = summarize_durations(&x).unwrap();
        assert_eq!(s.total_ms, 110.0);
        assert_eq!(s.mean_ms, 22.0);
        assert_eq!(s.median_ms, 3.0);
        assert_eq!(s.p95_ms, 100.0);
        assert_eq!(s.max_ms, 100.0)
    }
    #[test]
    fn empty_timing_summary_is_absent() {
        assert_eq!(summarize_durations(&[]), None)
    }
}
