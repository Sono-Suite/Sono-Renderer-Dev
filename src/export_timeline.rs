//! Shared Watch, media-source, and output-relative clocks for BGM and future MV.
use crate::offline::FrameRange;
use anyhow::{bail, Result};
use serde::Serialize;

pub const AUDIO_RATE: u32 = 44_100;

pub(crate) fn sample_frames(duration: f64) -> u64 {
    (duration * f64::from(AUDIO_RATE) - 1e-8).ceil() as u64
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ExportTimeline {
    pub requested_start: f64,
    pub watch_start: f64,
    pub duration: f64,
    pub media_offset: f64,
    pub audio_sample_frames: u64,
}

impl ExportTimeline {
    /// Preserve the renderer's existing upward alignment to the Watch frame grid.
    pub fn new(requested_start: f64, range: FrameRange, media_offset: f64) -> Result<Self> {
        if !media_offset.is_finite() {
            bail!("media offset must be finite");
        }
        let duration = range.frame_count as f64 / f64::from(range.fps);
        Ok(Self {
            requested_start,
            watch_start: range.start_time(),
            duration,
            media_offset,
            audio_sample_frames: sample_frames(duration),
        })
    }

    pub fn output_to_watch(self, output: f64) -> f64 {
        self.watch_start + output
    }

    pub fn watch_to_output(self, watch: f64) -> f64 {
        watch - self.watch_start
    }

    pub fn watch_to_media(self, watch: f64) -> f64 {
        watch + self.media_offset
    }

    pub fn output_to_media(self, output: f64) -> f64 {
        self.watch_to_media(self.output_to_watch(output))
    }

    /// Exact sample-domain clipping, real leading/trailing silence, zero-based PTS.
    /// The output is bounded even when the desired interval is outside the source.
    pub fn bgm_filter(self) -> String {
        let start = self.output_to_media(0.0);
        let source = (start.max(0.0) * f64::from(AUDIO_RATE)).round() as u64;
        let delay = ((-start).max(0.0).min(self.duration) * f64::from(AUDIO_RATE)).round() as u64;
        let count = self.audio_sample_frames;
        let source_end = source + count.saturating_sub(delay);
        format!("aresample={AUDIO_RATE},atrim=start_sample={source}:end_sample={source_end},asetpts=PTS-STARTPTS,adelay={delay}S:all=1,apad=whole_len={count},atrim=end_sample={count},asetpts=N/SR/TB")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_clock_maps_start_offsets_and_fractional_frame_grid() {
        for (start, offset, media) in [
            (0.0, -1.109, -1.109),
            (0.0, 0.05, 0.05),
            (2.0, -1.109, 0.891),
        ] {
            let t = ExportTimeline::new(start, FrameRange::new(start, 2.0, 12).unwrap(), offset)
                .unwrap();
            assert!((t.output_to_media(0.0) - media).abs() < 1e-10);
            assert_eq!(t.watch_to_output(t.watch_start), 0.0);
            assert_eq!(t.audio_sample_frames, 88_200);
        }
        let t = ExportTimeline::new(1.01, FrameRange::new(1.01, 0.2, 10).unwrap(), 0.0).unwrap();
        assert_eq!(t.watch_start, 1.1);
        assert_eq!(t.audio_sample_frames, 8820);
    }
}
