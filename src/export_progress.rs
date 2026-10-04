//! Structured export events. Consumers need no terminal-output parsing.
use serde::Serialize;
use std::{
    io::{self, IsTerminal, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExportPhase {
    Preparing,
    Validating,
    AudioMixing,
    AudioEncoding,
    Rendering,
    Finalizing,
    Complete,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportProgress {
    pub phase: ExportPhase,
    pub completed_frames: u64,
    pub total_frames: Option<u64>,
    pub percentage: Option<f64>,
    pub elapsed_seconds: f64,
    pub frames_per_second: Option<f64>,
    pub eta_seconds: Option<f64>,
}

pub type ProgressSink = Box<dyn FnMut(ExportProgress) + Send>;

#[derive(Clone)]
pub(crate) struct ProgressTracker(Arc<Mutex<State>>);
struct State {
    sink: ProgressSink,
    start: Instant,
    phase: ExportPhase,
    completed: u64,
    total: Option<u64>,
    // First frame includes startup/pre-roll; omit it from throughput/ETA.
    first_frame: Option<Instant>,
    last_event: Instant,
    render_elapsed: Option<f64>,
}

impl ProgressTracker {
    pub fn new(sink: ProgressSink) -> Self {
        Self(Arc::new(Mutex::new(State {
            sink,
            start: Instant::now(),
            phase: ExportPhase::Preparing,
            completed: 0,
            total: None,
            first_frame: None,
            last_event: Instant::now(),
            render_elapsed: None,
        })))
    }
    pub fn phase(&self, phase: ExportPhase, total: Option<u64>) {
        let mut state = self.0.lock().unwrap();
        state.phase = phase;
        if matches!(phase, ExportPhase::AudioMixing | ExportPhase::Finalizing)
            && state.render_elapsed.is_none()
        {
            state.render_elapsed = state.first_frame.map(|start| start.elapsed().as_secs_f64());
        }
        if total.is_some() {
            state.total = total;
        }
        state.emit();
    }
    pub fn validate_complete(&self) -> anyhow::Result<()> {
        let state = self.0.lock().unwrap();
        anyhow::ensure!(
            state.total == Some(state.completed),
            "export completed-frame count differs from the scheduled total"
        );
        Ok(())
    }
    pub fn pulse(&self) {
        let mut state = self.0.lock().unwrap();
        if state.last_event.elapsed() >= Duration::from_millis(250) {
            state.emit();
        }
    }
    fn frame(&self) {
        let mut state = self.0.lock().unwrap();
        state.completed += 1;
        if state.first_frame.is_none() {
            state.first_frame = Some(Instant::now());
        }
        state.emit();
    }
}

fn estimate(completed: u64, total: Option<u64>, elapsed: f64) -> (Option<f64>, Option<f64>) {
    if completed < 4 || elapsed < 1.0 {
        return (None, None);
    }
    let rate = (completed - 1) as f64 / elapsed;
    (
        Some(rate),
        total.map(|total| total.saturating_sub(completed) as f64 / rate),
    )
}

impl State {
    fn emit(&mut self) {
        self.last_event = Instant::now();
        let (rate, eta) = estimate(
            self.completed,
            self.total,
            self.render_elapsed.unwrap_or_else(|| {
                self.first_frame
                    .map_or(0.0, |start| start.elapsed().as_secs_f64())
            }),
        );
        let event = ExportProgress {
            phase: self.phase,
            completed_frames: self.completed,
            total_frames: self.total,
            percentage: self
                .total
                .filter(|n| *n > 0)
                .map(|n| self.completed as f64 * 100.0 / n as f64),
            elapsed_seconds: self.start.elapsed().as_secs_f64(),
            frames_per_second: rate,
            eta_seconds: if self.phase == ExportPhase::Rendering {
                eta
            } else {
                None
            },
        };
        (self.sink)(event);
    }
}

/// Counts only complete RGB frames successfully written to the export input.
pub(crate) struct ProgressWriter<W> {
    inner: W,
    control: crate::export_control::ExportControl,
    progress: ProgressTracker,
    bytes: usize,
    frame_bytes: usize,
}
impl<W> ProgressWriter<W> {
    pub fn with_control(mut self, control: crate::export_control::ExportControl) -> Self {
        self.control = control;
        self
    }
    pub fn new(inner: W, progress: ProgressTracker, frame_bytes: usize) -> Self {
        Self {
            inner,
            control: Default::default(),
            progress,
            bytes: 0,
            frame_bytes,
        }
    }
}
impl<W: Write> Write for ProgressWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.control.is_cancelled() {
            return Err(io::Error::other(crate::export_control::Cancelled));
        }
        let n = self.inner.write(bytes)?;
        self.bytes += n;
        while self.bytes >= self.frame_bytes {
            self.bytes -= self.frame_bytes;
            self.progress.frame();
        }
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// ASCII, one bounded line on an interactive stderr; sparse lines when redirected.
pub fn terminal_sink() -> ProgressSink {
    let interactive = io::stderr().is_terminal();
    let mut previous_phase = None;
    let mut last = Instant::now();
    Box::new(move |event| {
        let changed = previous_phase != Some(event.phase);
        let final_frame = event.total_frames == Some(event.completed_frames);
        if !changed
            && !final_frame
            && last.elapsed() < Duration::from_millis(if interactive { 250 } else { 5000 })
        {
            return;
        }
        previous_phase = Some(event.phase);
        last = Instant::now();
        let total = event.total_frames.map_or("?".into(), |v| v.to_string());
        let percent = event.percentage.map_or("?".into(), |v| format!("{v:.1}"));
        let rate = event
            .frames_per_second
            .map_or("--".into(), |v| format!("{v:.1}"));
        let eta = event
            .eta_seconds
            .map_or("--".into(), |v| format!("{v:.0}s"));
        let text = format!(
            "{:?} {}/{total} {percent}% | {:.0}s | {rate} fps | ETA {eta}",
            event.phase, event.completed_frames, event.elapsed_seconds
        );
        let mut stderr = io::stderr().lock();
        if interactive {
            // Fits a conventional 80-column Windows console without wrapping.
            let text: String = text.chars().take(78).collect();
            let _ = write!(stderr, "\r{text:<78}");
            if changed || final_frame {
                let _ = writeln!(stderr);
            }
        } else {
            let _ = writeln!(stderr, "{text}");
        }
        let _ = stderr.flush();
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_handoff_does_not_invent_completed_frames() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let output = events.clone();
        let progress = ProgressTracker::new(Box::new(move |e| output.lock().unwrap().push(e)));
        progress.phase(ExportPhase::Rendering, Some(1));
        assert!(ProgressWriter::new(Broken, progress.clone(), 6)
            .write_all(&[0; 6])
            .is_err());
        assert!(progress.validate_complete().is_err());
        assert_eq!(events.lock().unwrap().last().unwrap().completed_frames, 0);
    }
    #[test]
    fn eta_waits_for_samples_and_excludes_first_frame() {
        assert_eq!(estimate(1, Some(20), 100.0), (None, None));
        assert_eq!(estimate(4, Some(20), 0.1), (None, None));
        assert_eq!(estimate(5, Some(20), 2.0), (Some(2.0), Some(7.5)));
        assert_eq!(estimate(20, Some(20), 19.0), (Some(1.0), Some(0.0)));
    }
    #[test]
    fn counts_completed_writes_and_phases_without_counting_partial_frames() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let output = events.clone();
        let progress = ProgressTracker::new(Box::new(move |e| output.lock().unwrap().push(e)));
        progress.phase(ExportPhase::Rendering, Some(2));
        let mut writer = ProgressWriter::new(Vec::new(), progress.clone(), 6);
        writer.write_all(&[0; 5]).unwrap();
        assert_eq!(events.lock().unwrap().last().unwrap().completed_frames, 0);
        writer.write_all(&[0; 7]).unwrap();
        progress.phase(ExportPhase::Finalizing, None);
        progress.phase(ExportPhase::Complete, None);
        let events = events.lock().unwrap();
        assert_eq!(
            events
                .iter()
                .map(|e| e.completed_frames)
                .collect::<Vec<_>>(),
            [0, 1, 2, 2, 2]
        );
        assert_eq!(events.last().unwrap().percentage, Some(100.0));
    }
}
