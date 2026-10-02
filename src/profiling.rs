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
        }
    }
    pub(crate) fn begin_render(&mut self) {
        self.render_start = Some(std::time::Instant::now())
    }
    pub(crate) fn finish_render(&mut self) {
        if let Some(t) = self.render_start.take() {
            self.record("Render phase", t.elapsed())
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
        handoff: Duration,
        wall: Duration,
    ) {
        self.record("Watch VM", profile.vm);
        self.record("Draw preparation", profile.preparation);
        self.record(
            "CPU render-command preparation",
            profile.gpu_draw_preparation,
        );
        if self.backend == "cpu" {
            self.record("CPU raster", profile.cpu_render)
        }
        self.record("GPU encode/submit", profile.gpu_encode_submit);
        self.record("GPU wait/map/readback", profile.gpu_wait_map);
        self.record(
            "Framebuffer unpack/copy",
            profile.framebuffer_unpack + profile.framebuffer_copy,
        );
        self.record("Runtime UI", profile.runtime_ui);
        self.record("FFmpeg handoff", handoff);
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
            eprintln!("profile frame: {phase} #{frame} t={time:.3}s total={:.2}ms vm={:.2} prep={:.2} encode={:.2} wait/map={:.2} unpack={:.2} ffmpeg-write={:.2}",wall.as_secs_f64()*1e3,profile.vm.as_secs_f64()*1e3,profile.preparation.as_secs_f64()*1e3,profile.gpu_encode_submit.as_secs_f64()*1e3,profile.gpu_wait_map.as_secs_f64()*1e3,profile.framebuffer_unpack.as_secs_f64()*1e3,handoff.as_secs_f64()*1e3)
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
    pub(crate) fn record_preflight(&mut self, profile: &FrameStageProfile) {
        self.atlas_uploads += profile.atlas_uploads as u64;
        if self.adapter.is_none() {
            self.adapter = profile.adapter.clone();
        }
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
        let n = self.frame_count.max(1) as f64;
        eprintln!("GPU execution timestamp: not collected (no additional query/readback added)\nEntities/frame: {:.1}\nSkin draws/frame: {:.1}\nParticle draws/frame: {:.1}\nGPU draws/frame: {:.1}\nAtlas uploads/cache misses: {}\nReadback/frame: {:.0} bytes",self.entities as f64/n,self.skin_draws as f64/n,self.particles as f64/n,self.gpu_draws as f64/n,self.atlas_uploads,self.readback_bytes as f64/n);
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
        let steady_count = frame_walls.len().saturating_sub(1);
        let steady_seconds = frame_walls
            .iter()
            .skip(1)
            .map(Duration::as_secs_f64)
            .sum::<f64>();
        let steady_fps = steady_count as f64 / steady_seconds.max(f64::MIN_POSITIVE);
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
