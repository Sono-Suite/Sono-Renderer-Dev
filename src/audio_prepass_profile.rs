//! Opt-in, bounded AudioPrepass checkpoints; no callback or scheduling changes.
use crate::watch_runtime::{FrameReport, FrameRuntimeProfile, WatchRuntime};
use serde::Serialize;
use std::{collections::BTreeMap, time::Instant};

#[derive(Debug, Clone, Serialize)]
pub struct AudioPrepassCheckpoint {
    pub watch_start: f64,
    pub watch_end: f64,
    pub wall_seconds: f64,
    pub runtime_frames: u64,
    pub callbacks: u64,
    pub evaluations: u64,
    pub callback_stages: BTreeMap<String, u64>,
    pub stage_seconds: BTreeMap<String, f64>,
    pub memory_entries_copied: u64,
    pub global_memory_entries: usize,
    pub retained_entity_memory_entries: usize,
    pub entities: usize,
    pub spawned_in_interval: usize,
    pub spawned_total: usize,
    pub inactive_entities: usize,
    pub expired_retained_entities: usize,
    pub active_min: usize,
    pub active_max: usize,
    pub active_mean: f64,
    pub active_at_end: BTreeMap<String, usize>,
    pub sfx_events: [usize; 4],
    pub draw_commands: usize,
    pub particle_commands: usize,
    pub live_particle_instances: usize,
}

pub(crate) struct Checkpoints {
    start: f64,
    next: f64,
    wall: Instant,
    runtime: FrameRuntimeProfile,
    samples: usize,
    active_sum: usize,
    active_min: usize,
    active_max: usize,
    spawned: usize,
    sfx: [usize; 4],
    draws: usize,
    particles: usize,
    stages: BTreeMap<String, u64>,
}

impl Checkpoints {
    pub fn new() -> Self {
        Self {
            start: 0.0,
            next: 10.0,
            wall: Instant::now(),
            runtime: Default::default(),
            samples: 0,
            active_sum: 0,
            active_min: usize::MAX,
            active_max: 0,
            spawned: 0,
            sfx: [0; 4],
            draws: 0,
            particles: 0,
            stages: BTreeMap::new(),
        }
    }

    pub fn observe(
        &mut self,
        report: &FrameReport,
        runtime: &WatchRuntime<'_>,
        watch_end: f64,
        final_frame: bool,
    ) -> Option<AudioPrepassCheckpoint> {
        self.runtime.accumulate(&report.runtime_profile);
        self.samples += 1;
        self.active_sum += report.active_entity_count;
        self.active_min = self.active_min.min(report.active_entity_count);
        self.active_max = self.active_max.max(report.active_entity_count);
        self.spawned += report.spawned.len();
        for (sum, count) in self.sfx.iter_mut().zip([
            report.audio_events.len(),
            report.scheduled_effects.len(),
            report.scheduled_looped_effects.len(),
            report.scheduled_looped_effect_stops.len(),
        ]) {
            *sum += count;
        }
        self.draws += report.display_list.sprites.len();
        self.particles += report.particle_events.len();
        for callback in &report.callbacks {
            *self
                .stages
                .entry(format!("{:?}", callback.stage))
                .or_default() += 1;
        }
        if watch_end + 1e-9 < self.next && !final_frame {
            return None;
        }
        let wall_seconds = self.wall.elapsed().as_secs_f64();
        let p = &self.runtime;
        let stage_seconds = [
            ("runtime", p.frame),
            ("preprocess", p.preprocess),
            ("update_spawn", p.update_spawn),
            ("scheduling", p.scheduling),
            ("activation", p.activation),
            ("update_sequential", p.update_sequential),
            ("update_parallel", p.update_parallel),
            ("vm_construction", p.callback_vm_construction),
            ("context_setup", p.callback_context_setup),
            ("memory_setup", p.callback_memory_setup),
            ("execute", p.callback_execute),
            ("commit", p.callback_commit),
            ("report", p.report_materialization),
            ("event_aggregation", p.stepper_event_aggregation),
        ]
        .into_iter()
        .map(|(key, duration)| (key.into(), duration.as_secs_f64()))
        .collect();
        let mut active_at_end = BTreeMap::new();
        for entity in runtime.entities.iter().filter(|e| e.active) {
            *active_at_end.entry(entity.archetype.clone()).or_default() += 1;
        }
        let checkpoint = AudioPrepassCheckpoint {
            watch_start: self.start,
            watch_end,
            wall_seconds,
            runtime_frames: p.frame_count,
            callbacks: p.callbacks,
            evaluations: p.evaluations,
            callback_stages: std::mem::take(&mut self.stages),
            stage_seconds,
            memory_entries_copied: p.memory_entries_copied,
            global_memory_entries: runtime.global_memory.len(),
            retained_entity_memory_entries: runtime.entities.iter().map(|e| e.memory.len()).sum(),
            entities: runtime.entities.len(),
            spawned_in_interval: self.spawned,
            spawned_total: runtime.entities.iter().filter(|e| e.spawned).count(),
            inactive_entities: runtime.entities.iter().filter(|e| !e.active).count(),
            expired_retained_entities: runtime
                .entities
                .iter()
                .filter(|e| {
                    !e.active
                        && e.schedule
                            .is_some_and(|(_, end)| end <= report.timeline.unwrap_or(0.0))
                })
                .count(),
            active_min: self.active_min,
            active_max: self.active_max,
            active_mean: self.active_sum as f64 / self.samples as f64,
            active_at_end,
            sfx_events: self.sfx,
            draw_commands: self.draws,
            particle_commands: self.particles,
            live_particle_instances: runtime
                .context
                .particle_instances
                .read()
                .ok()
                .map_or(0, |state| state.instances().len()),
        };
        self.start = watch_end;
        self.next = (watch_end / 10.0 + 1e-9).floor() * 10.0 + 10.0;
        self.wall = Instant::now();
        self.runtime = Default::default();
        self.samples = 0;
        self.active_sum = 0;
        self.active_min = usize::MAX;
        self.active_max = 0;
        self.spawned = 0;
        self.sfx = [0; 4];
        self.draws = 0;
        self.particles = 0;
        Some(checkpoint)
    }
}
