//! Generic host for Sonolus WatchData entities.

use crate::{
    formats::LevelData,
    runtime::{
        AudioEffectEvent, DebugEvent, DestroyedParticleEffect, DisplayList, Memory,
        ParticleEffectEvent, ScheduledEffect, ScheduledLoopedEffect, ScheduledLoopedEffectStop,
        VmContext, WatchVm,
    },
    watch::{WatchArchetype, WatchData},
};
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const ENTITY_MEMORY: i64 = 4000;
const ENTITY_DATA: i64 = 4001;
const ENTITY_SHARED_MEMORY: i64 = 4002;
const TEMPORARY_MEMORY: i64 = 10000;
const RUNTIME_UPDATE: i64 = 1001;
const RUNTIME_SKIN_TRANSFORM: i64 = 1002;
const RUNTIME_ENVIRONMENT: i64 = 1000;
const LEVEL_OPTION: i64 = 2002;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LifecycleStage {
    Preprocess,
    SpawnTime,
    DespawnTime,
    UpdateSpawn,
    Initialize,
    UpdateSequential,
    UpdateParallel,
    Terminate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CallbackRecord {
    pub entity_id: Option<usize>,
    pub archetype: String,
    pub stage: LifecycleStage,
    pub node: usize,
}

#[derive(Debug, Clone)]
pub struct WatchEntity {
    pub id: usize,
    pub archetype_index: usize,
    pub archetype: String,
    pub level_entity_index: Option<usize>,
    pub name: Option<String>,
    pub spawned: bool,
    pub has_entity_data: bool,
    pub has_shared_memory: bool,
    pub entity_data: Vec<f64>,
    pub memory: Memory,
    pub shared_memory: Memory,
    pub schedule: Option<(f64, f64)>,
    pub active: bool,
    pub initialized: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct FrameReport {
    pub timeline: Option<f64>,
    /// Runtime Update block values: current time, delta time, scaled time, reserved slot.
    pub runtime_update: [f64; 4],
    /// Runtime Update block after callbacks have completed for this frame.
    pub runtime_update_after_callbacks: [f64; 4],
    pub timescale: f64,
    pub runtime_entity_count: usize,
    pub active_entity_count: usize,
    pub callbacks: Vec<CallbackRecord>,
    pub spawned: Vec<usize>,
    pub display_list: DisplayList,
    pub scheduled_effects: Vec<ScheduledEffect>,
    pub scheduled_looped_effects: Vec<ScheduledLoopedEffect>,
    pub scheduled_looped_effect_stops: Vec<ScheduledLoopedEffectStop>,
    pub audio_events: Vec<AudioEffectEvent>,
    pub destroyed_particle_effects: Vec<DestroyedParticleEffect>,
    pub particle_events: Vec<ParticleEffectEvent>,
    pub debug_events: Vec<DebugEvent>,
    pub function_counts: BTreeMap<String, u64>,
    pub skin_checks: Vec<(i64, bool)>,
    pub vm_evaluations: u64,
    pub spawn_requests_produced: u64,
    pub runtime_background_quad: [f64; 8],
    /// Runtime Skin Transform matrix as seen after this frame's callbacks.
    pub runtime_skin_transform: [f64; 16],
}

#[derive(Debug, Clone, Copy)]
pub struct WatchDiagnosticTarget {
    pub entity_id: usize,
    pub stage: LifecycleStage,
    pub event_capacity: usize,
}

/// Watch host. Level entities are kept in source order; callback `order` only
/// changes the order within a lifecycle system and ties preserve source order.
pub struct WatchRuntime<'a> {
    watch: &'a WatchData,
    archetype_defs: Vec<WatchArchetype>,
    archetypes: BTreeMap<String, usize>,
    pub entities: Vec<WatchEntity>,
    pub global_memory: Memory,
    pending: Vec<usize>,
    preprocessed: bool,
    pub context: VmContext,
    pub callback_log: Vec<CallbackRecord>,
    frame_display_list: DisplayList,
    frame_scheduled_effects: Vec<ScheduledEffect>,
    frame_scheduled_looped_effects: Vec<ScheduledLoopedEffect>,
    frame_scheduled_looped_effect_stops: Vec<ScheduledLoopedEffectStop>,
    frame_audio_events: Vec<AudioEffectEvent>,
    frame_destroyed_particle_effects: Vec<DestroyedParticleEffect>,
    frame_particle_events: Vec<ParticleEffectEvent>,
    frame_debug_events: Vec<DebugEvent>,
    frame_function_counts: BTreeMap<String, u64>,
    frame_skin_checks: Vec<(i64, bool)>,
    last_frame_time: Option<f64>,
    frame_vm_evaluations: u64,
    frame_spawn_requests: u64,
    diagnostic_target: Option<WatchDiagnosticTarget>,
    trace_draws: bool,
}

impl<'a> WatchRuntime<'a> {
    pub fn new(watch: &'a WatchData, level: &LevelData) -> Result<Self> {
        let mut archetype_defs = watch.archetypes.clone();
        let mut archetypes = BTreeMap::new();
        for (index, archetype) in archetype_defs.iter().enumerate() {
            if archetype.name.is_empty() {
                bail!("Watch archetype at index {index} has an empty name");
            }
            if archetypes.insert(archetype.name.clone(), index).is_some() {
                bail!("duplicate Watch archetype name {:?}", archetype.name);
            }
        }

        // These two Sonolus archetypes are host-defined events. Engines may
        // omit them from EngineWatchData; their named LevelData fields still
        // participate in the automatic timeline maps.
        for (name, fields) in [
            ("#BPM_CHANGE", &["#BEAT", "#BPM"][..]),
            ("#TIMESCALE_CHANGE", &["#BEAT", "#TIMESCALE"][..]),
        ] {
            if !archetypes.contains_key(name) {
                let index = archetype_defs.len();
                archetype_defs.push(WatchArchetype {
                    name: name.to_owned(),
                    imports: fields
                        .iter()
                        .enumerate()
                        .map(|(index, name)| serde_json::json!({"name": name, "index": index}))
                        .collect(),
                    exports: Vec::new(),
                    has_input: false,
                    preprocess: None,
                    spawn_time: None,
                    despawn_time: None,
                    initialize: None,
                    update_sequential: None,
                    update_parallel: None,
                    terminate: None,
                    callbacks: BTreeMap::new(),
                });
                archetypes.insert(name.to_owned(), index);
            }
        }

        let names = collect_entity_names(&level.entities)?;
        let mut runtime = Self {
            watch,
            archetype_defs,
            archetypes,
            entities: Vec::with_capacity(level.entities.len()),
            global_memory: Memory::new(),
            pending: Vec::with_capacity(level.entities.len()),
            preprocessed: false,
            context: VmContext {
                timescale: 1.0,
                ..VmContext::default()
            },
            callback_log: Vec::new(),
            frame_display_list: DisplayList::default(),
            frame_scheduled_effects: Vec::new(),
            frame_scheduled_looped_effects: Vec::new(),
            frame_scheduled_looped_effect_stops: Vec::new(),
            frame_audio_events: Vec::new(),
            frame_destroyed_particle_effects: Vec::new(),
            frame_particle_events: Vec::new(),
            frame_debug_events: Vec::new(),
            frame_function_counts: BTreeMap::new(),
            frame_skin_checks: Vec::new(),
            last_frame_time: None,
            frame_vm_evaluations: 0,
            frame_spawn_requests: 0,
            diagnostic_target: None,
            trace_draws: false,
        };
        for index in 0..16 {
            runtime.global_memory.set(
                RUNTIME_SKIN_TRANSFORM,
                index,
                if index % 5 == 0 { 1.0 } else { 0.0 },
            );
        }

        for (index, source) in level.entities.iter().enumerate() {
            let archetype_name = source
                .archetype
                .as_str()
                .with_context(|| format!("level entity {index} archetype must be a string"))?;
            let archetype_index = *runtime.archetypes.get(archetype_name).ok_or_else(|| {
                anyhow!(
                    "level entity {index} references unknown Watch archetype {archetype_name:?}"
                )
            })?;
            let archetype = &runtime.archetype_defs[archetype_index];
            let entity_data = map_imports(archetype, &source.data, &names, index)?;
            let name = source
                .extra
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned);
            runtime.entities.push(WatchEntity {
                id: index,
                archetype_index,
                archetype: archetype_name.to_owned(),
                level_entity_index: Some(index),
                name,
                spawned: false,
                has_entity_data: true,
                has_shared_memory: true,
                entity_data,
                memory: Memory::new(),
                shared_memory: Memory::new(),
                schedule: None,
                active: false,
                initialized: false,
            });
            runtime.pending.push(index);
        }
        let initial_array = runtime
            .entities
            .iter()
            .flat_map(|entity| (0..32).map(move |slot| entity.entity_data[slot]))
            .collect();
        runtime.context.entity_data_array =
            std::sync::Arc::new(std::sync::RwLock::new(initial_array));
        runtime.context.entity_shared_memory_array =
            std::sync::Arc::new(std::sync::RwLock::new(vec![
                0.0;
                runtime.entities.len() * 32
            ]));
        runtime.context.entity_info_array = std::sync::Arc::new(std::sync::RwLock::new(
            runtime
                .entities
                .iter()
                .flat_map(|entity| [entity.id as f64, entity.archetype_index as f64, 0.0])
                .collect(),
        ));
        runtime.context.bpm_map = runtime.build_bpm_map()?;
        let (time_map, timescale_map) = runtime.build_timescale_maps()?;
        runtime.context.time_map = time_map;
        runtime.context.timescale_map = timescale_map;
        Ok(runtime)
    }

    fn build_bpm_map(&self) -> Result<Vec<(f64, f64)>> {
        let mut changes = Vec::new();
        for entity in &self.entities {
            if entity.archetype != "#BPM_CHANGE" {
                continue;
            }
            let archetype = &self.archetype_defs[entity.archetype_index];
            let beat = imported_value(entity, archetype, "#BEAT")?;
            let bpm = imported_value(entity, archetype, "#BPM")?;
            if !beat.is_finite() || !bpm.is_finite() || bpm <= 0.0 {
                bail!(
                    "#BPM_CHANGE entity {} has invalid beat/BPM ({beat}, {bpm})",
                    entity.id
                );
            }
            changes.push((beat, bpm, entity.id));
        }
        changes.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)));
        Ok(changes
            .into_iter()
            .map(|(beat, bpm, _)| (beat, bpm))
            .collect())
    }

    fn build_timescale_maps(&self) -> Result<(Vec<(f64, f64)>, Vec<(f64, f64)>)> {
        let mut changes = Vec::new();
        for entity in &self.entities {
            if entity.archetype != "#TIMESCALE_CHANGE" {
                continue;
            }
            let archetype = &self.archetype_defs[entity.archetype_index];
            let beat = imported_value(entity, archetype, "#BEAT")?;
            let scale = imported_value(entity, archetype, "#TIMESCALE")?;
            if !beat.is_finite() || !scale.is_finite() {
                bail!(
                    "#TIMESCALE_CHANGE entity {} has invalid beat/timescale ({beat}, {scale})",
                    entity.id
                );
            }
            let time = self.context.beat_to_time(beat)?;
            changes.push((time, scale, entity.id));
        }
        changes.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)));
        let timescale_map = changes
            .iter()
            .map(|(time, scale, _)| (*time, *scale))
            .collect::<Vec<_>>();

        // Scaled time is the integral of the active timescale from time zero.
        // Keep zero as an anchor so both positive and negative timestamps have
        // a defined, deterministic value.
        let scale_at_zero = timescale_map
            .iter()
            .rev()
            .find(|(time, _)| *time <= 0.0)
            .map(|(_, scale)| *scale)
            .unwrap_or(1.0);
        let mut points: Vec<(f64, f64)> = vec![(0.0, 0.0)];
        let mut cursor_time = 0.0;
        let mut cursor_scaled = 0.0;
        let mut scale = scale_at_zero;
        for (time, next_scale, _) in changes.iter().filter(|(time, _, _)| *time > 0.0) {
            cursor_scaled += (*time - cursor_time) * scale;
            points.push((*time, cursor_scaled));
            cursor_time = *time;
            scale = *next_scale;
        }
        cursor_time = 0.0;
        cursor_scaled = 0.0;
        for (time, scale_before, _) in changes.iter().rev().filter(|(time, _, _)| *time < 0.0) {
            cursor_scaled -= (cursor_time - *time) * *scale_before;
            points.push((*time, cursor_scaled));
            cursor_time = *time;
        }
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        Ok((points, timescale_map))
    }

    pub fn resolved_level_entity_count(&self) -> usize {
        self.entities
            .iter()
            .filter(|entity| entity.level_entity_index.is_some())
            .count()
    }

    /// Enable a bounded VM trace for exactly one entity callback invocation.
    pub fn set_diagnostic_target(&mut self, target: Option<WatchDiagnosticTarget>) -> Result<()> {
        if let Some(target) = target {
            if !(1..=4096).contains(&target.event_capacity) {
                bail!("diagnostic event capacity must be in 1..=4096");
            }
            if let Some(entity) = self.entities.get(target.entity_id) {
                if callback(
                    self.archetype_defs[entity.archetype_index].clone(),
                    target.stage,
                )
                .is_none()
                {
                    bail!(
                        "diagnostic entity {} has no {:?} callback",
                        target.entity_id,
                        target.stage
                    );
                }
            }
        }
        self.diagnostic_target = target;
        Ok(())
    }

    /// Capture the evaluated WatchData argument graphs for each executed Draw.
    pub fn set_draw_tracing(&mut self, enabled: bool) {
        self.trace_draws = enabled;
    }

    /// Bind only sprites declared by WatchData and present in the selected
    /// skin's SkinData. Engine-local numeric IDs are retained from WatchData.
    pub fn bind_skin_sprite_names(&mut self, present_names: &BTreeSet<String>) -> Result<()> {
        let bindings = self
            .watch
            .skin
            .get("sprites")
            .and_then(Value::as_array)
            .context("EngineWatchData skin binding has no sprites array")?;
        self.context.skin_sprites.clear();
        for binding in bindings {
            let name = binding
                .get("name")
                .and_then(Value::as_str)
                .context("EngineWatchData skin binding has no string name")?;
            let id = binding
                .get("id")
                .and_then(Value::as_u64)
                .context("EngineWatchData skin binding has no nonnegative ID")?;
            if present_names.contains(name) {
                let id = u32::try_from(id).context("skin sprite ID exceeds u32")?;
                self.context.skin_sprites.insert(id);
            }
        }
        Ok(())
    }

    /// Bind only effect clips declared by EngineWatchData and present in the
    /// selected effect resource. Missing individual clips remain unavailable
    /// to `HasEffectClip` instead of invalidating the resource package.
    pub fn bind_effect_clip_names(&mut self, present_names: &BTreeSet<String>) -> Result<()> {
        self.context.effect_clips = bind_named_ids(
            &self.watch.effect,
            "clips",
            present_names,
            "EngineWatchData effect binding",
        )?;
        Ok(())
    }

    /// Bind only particle effects declared by EngineWatchData and present in
    /// the selected particle resource. Missing individual effects remain
    /// unavailable to `HasParticleEffect`.
    pub fn bind_particle_effect_names(&mut self, present_names: &BTreeSet<String>) -> Result<()> {
        self.context.particle_effects = bind_named_ids(
            &self.watch.particle,
            "effects",
            present_names,
            "EngineWatchData particle binding",
        )?;
        Ok(())
    }

    /// Load the engine's read-only ROM block, encoded as little-endian f32s.
    pub fn bind_engine_rom(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.len() % std::mem::size_of::<f32>() != 0 {
            bail!(
                "Engine Rom length {} is not a whole number of 32-bit values",
                bytes.len()
            );
        }
        let values = bytes
            .chunks_exact(4)
            .map(|chunk| f64::from(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])))
            .collect();
        self.context.engine_rom = std::sync::Arc::new(values);
        Ok(())
    }

    /// Populate Level Option using EngineConfiguration defaults. Callers may
    /// override individual slots afterward when a project supplies selections.
    pub fn bind_engine_option_defaults(&mut self, configuration: &Value) -> Result<()> {
        let options = configuration
            .get("options")
            .and_then(Value::as_array)
            .context("EngineConfiguration has no options array")?;
        for (index, option) in options.iter().enumerate() {
            let value = option.get("def").and_then(Value::as_f64).with_context(|| {
                format!("EngineConfiguration option {index} has no numeric def")
            })?;
            self.global_memory.set(LEVEL_OPTION, index, value);
        }
        Ok(())
    }

    /// Supply the output screen aspect ratio consumed by Runtime Environment.
    pub fn set_screen_aspect_ratio(&mut self, aspect_ratio: f64) -> Result<()> {
        if !aspect_ratio.is_finite() || aspect_ratio <= 0.0 {
            bail!("screen aspect ratio must be finite and positive");
        }
        self.global_memory.set(RUNTIME_ENVIRONMENT, 1, aspect_ratio);
        Ok(())
    }

    /// Initialize the Runtime Background block from BackgroundData. Watch
    /// preprocess/updateSequential callbacks can subsequently modify it.
    pub fn set_runtime_background_quad(&mut self, quad: [[f64; 2]; 4]) -> Result<()> {
        if !quad.iter().flatten().all(|value| value.is_finite()) {
            bail!("Runtime Background quad must contain finite coordinates");
        }
        let mut values = self
            .context
            .runtime_background
            .write()
            .map_err(|_| anyhow!("Runtime Background lock was poisoned"))?;
        for (index, value) in quad.iter().flatten().enumerate() {
            values[index] = *value;
        }
        Ok(())
    }

    /// Execute the one-time preprocessing system in callback order.
    pub fn preprocess(&mut self) -> Result<()> {
        if self.preprocessed {
            return Ok(());
        }
        let mut order: Vec<usize> = (0..self.entities.len()).collect();
        order.sort_by_key(|&id| {
            callback_order(&self.archetype_defs[self.entities[id].archetype_index].preprocess)
        });
        for id in order {
            self.invoke_entity(id, LifecycleStage::Preprocess)?;
        }
        for entity in &mut self.entities {
            if !entity.has_entity_data {
                continue;
            }
            let start = entity.id * 32;
            if let Ok(values) = self.context.entity_data_array.read() {
                for slot in 0..32 {
                    entity.entity_data[slot] = values.get(start + slot).copied().unwrap_or(0.0);
                }
            }
        }
        self.preprocessed = true;
        Ok(())
    }

    /// Advance one deterministic Watch frame at a caller-supplied time.
    pub fn frame(&mut self, time: f64) -> Result<FrameReport> {
        if !time.is_finite() {
            bail!("Watch frame time must be finite");
        }
        let initial_entity_count = self.entities.len();
        self.callback_log.clear();
        self.frame_display_list = DisplayList::default();
        self.frame_scheduled_effects.clear();
        self.frame_scheduled_looped_effects.clear();
        self.frame_scheduled_looped_effect_stops.clear();
        self.frame_audio_events.clear();
        self.frame_destroyed_particle_effects.clear();
        self.frame_particle_events.clear();
        self.frame_debug_events.clear();
        self.frame_function_counts.clear();
        self.frame_skin_checks.clear();
        self.frame_vm_evaluations = 0;
        self.frame_spawn_requests = 0;
        // Watch timestamps may move backwards during a seek. Keep the runtime
        // delta nonnegative; the requested absolute time remains authoritative.
        let delta_time = self
            .last_frame_time
            .filter(|previous| time >= *previous)
            .map(|previous| time - previous)
            .unwrap_or(0.0);
        // Preprocess is the one-time setup pass at the initial runtime state,
        // before advancing the requested playback frame.
        self.preprocess()?;
        self.context.time = time;
        let scaled_time = self.context.scaled_time(time)?;
        self.global_memory.set(RUNTIME_UPDATE, 0, time);
        self.global_memory.set(RUNTIME_UPDATE, 1, delta_time);
        self.global_memory.set(RUNTIME_UPDATE, 2, scaled_time);
        self.global_memory.set(RUNTIME_UPDATE, 3, 0.0);
        self.context.timescale = self.context.time_to_timescale(time)?;
        let timeline = if let Some(node) = self.watch.update_spawn {
            self.invoke_global(LifecycleStage::UpdateSpawn, node)?
        } else {
            time
        };

        self.compute_schedules()?;

        let leaving: Vec<usize> = self
            .entities
            .iter()
            .filter(|e| e.active && !in_spawn_range(e, timeline))
            .map(|e| e.id)
            .collect();
        for id in leaving {
            self.invoke_entity(id, LifecycleStage::Terminate)?;
            self.entities[id].active = false;
            self.set_entity_info_active(id, false);
            self.entities[id].initialized = false;
            self.pending.push(id);
        }

        let entering: Vec<usize> = self
            .pending
            .iter()
            .copied()
            .filter(|&id| in_spawn_range(&self.entities[id], timeline))
            .collect();
        for id in &entering {
            self.entities[*id].active = true;
            self.set_entity_info_active(*id, true);
        }
        self.pending.retain(|id| !entering.contains(id));
        for id in entering {
            self.invoke_entity(id, LifecycleStage::Initialize)?;
            self.entities[id].initialized = true;
        }

        self.run_active_stage(LifecycleStage::UpdateSequential)?;
        self.run_active_stage(LifecycleStage::UpdateParallel)?;
        self.last_frame_time = Some(time);

        let report = FrameReport {
            timeline: Some(timeline),
            runtime_update: [time, delta_time, scaled_time, 0.0],
            runtime_update_after_callbacks: [
                self.global_memory.get(RUNTIME_UPDATE, 0),
                self.global_memory.get(RUNTIME_UPDATE, 1),
                self.global_memory.get(RUNTIME_UPDATE, 2),
                self.global_memory.get(RUNTIME_UPDATE, 3),
            ],
            timescale: self.context.timescale,
            runtime_entity_count: self.entities.len(),
            active_entity_count: self.entities.iter().filter(|entity| entity.active).count(),
            callbacks: self.callback_log.clone(),
            spawned: (initial_entity_count..self.entities.len()).collect(),
            display_list: self.frame_display_list.clone(),
            scheduled_effects: self.frame_scheduled_effects.clone(),
            scheduled_looped_effects: self.frame_scheduled_looped_effects.clone(),
            scheduled_looped_effect_stops: self.frame_scheduled_looped_effect_stops.clone(),
            audio_events: self.frame_audio_events.clone(),
            destroyed_particle_effects: self.frame_destroyed_particle_effects.clone(),
            particle_events: self.frame_particle_events.clone(),
            debug_events: self.frame_debug_events.clone(),
            function_counts: self.frame_function_counts.clone(),
            skin_checks: self.frame_skin_checks.clone(),
            vm_evaluations: self.frame_vm_evaluations,
            spawn_requests_produced: self.frame_spawn_requests,
            runtime_background_quad: self
                .context
                .runtime_background
                .read()
                .map(|values| *values)
                .unwrap_or_default(),
            runtime_skin_transform: std::array::from_fn(|index| {
                self.global_memory.get(RUNTIME_SKIN_TRANSFORM, index)
            }),
        };
        Ok(report)
    }

    fn compute_schedules(&mut self) -> Result<()> {
        let unscheduled: Vec<usize> = self
            .pending
            .iter()
            .copied()
            .filter(|&id| self.entities[id].schedule.is_none())
            .collect();
        let mut spawn_order = unscheduled.clone();
        spawn_order.sort_by_key(|&id| {
            callback_order(&self.archetype_defs[self.entities[id].archetype_index].spawn_time)
        });
        let mut spawn_times = BTreeMap::new();
        for id in spawn_order {
            let archetype = &self.archetype_defs[self.entities[id].archetype_index];
            if archetype.spawn_time.is_some() {
                spawn_times.insert(id, self.invoke_entity(id, LifecycleStage::SpawnTime)?);
            }
        }

        let mut despawn_order = unscheduled.clone();
        despawn_order.sort_by_key(|&id| {
            callback_order(&self.archetype_defs[self.entities[id].archetype_index].despawn_time)
        });
        let mut despawn_times = BTreeMap::new();
        for id in despawn_order {
            let archetype = &self.archetype_defs[self.entities[id].archetype_index];
            if archetype.despawn_time.is_some() {
                despawn_times.insert(id, self.invoke_entity(id, LifecycleStage::DespawnTime)?);
            }
        }

        for id in unscheduled {
            let start = spawn_times.get(&id).copied().unwrap_or(f64::NEG_INFINITY);
            let end = despawn_times.get(&id).copied().unwrap_or(f64::INFINITY);
            if start.is_nan() || end.is_nan() {
                bail!("entity {id} has NaN spawn/despawn time");
            }
            self.entities[id].schedule = Some((start, end));
        }
        Ok(())
    }

    fn run_active_stage(&mut self, stage: LifecycleStage) -> Result<()> {
        let mut active: Vec<usize> = self
            .entities
            .iter()
            .filter(|entity| entity.active && entity.initialized)
            .map(|entity| entity.id)
            .collect();
        active.sort_by_key(|&id| {
            callback_order(&callback(
                self.archetype_defs[self.entities[id].archetype_index].clone(),
                stage,
            ))
        });
        for id in active {
            self.invoke_entity(id, stage)?;
        }
        Ok(())
    }

    fn invoke_entity(&mut self, id: usize, stage: LifecycleStage) -> Result<f64> {
        let archetype_index = self
            .entities
            .get(id)
            .context("entity id out of range")?
            .archetype_index;
        let archetype = &self.archetype_defs[archetype_index];
        let callback = callback(archetype.clone(), stage);
        let Some(value) = callback else {
            return Ok(0.0);
        };
        let node = callback_index(&value)
            .with_context(|| format!("invalid {stage:?} callback for {}", archetype.name))?;
        let record = CallbackRecord {
            entity_id: Some(id),
            archetype: archetype.name.clone(),
            stage,
            node,
        };
        let mut vm = WatchVm::new(&self.watch.nodes);
        vm.context = self.context.clone();
        vm.context.entity_data_array_writable = stage == LifecycleStage::Preprocess;
        vm.context.lifecycle_stage = Some(stage as u8);
        vm.context.entity_id = Some(id);
        vm.context.entity_archetype = Some(archetype.name.clone());
        vm.context.callback_name = Some(format!("{stage:?}"));
        vm.context.callback_node = Some(node);
        vm.set_draw_tracing(self.trace_draws);
        vm.context.has_entity_data = self.entities[id].has_entity_data;
        vm.context.has_entity_shared_memory = self.entities[id].has_shared_memory;
        vm.context.entity_info = (!self.entities[id].spawned).then_some([
            id as f64,
            archetype_index as f64,
            if self.entities[id].active { 1.0 } else { 0.0 },
        ]);
        vm.memory = self.global_memory.clone();
        vm.memory.retain_other_than(TEMPORARY_MEMORY);
        vm.memory.overlay(&self.entities[id].memory);
        vm.memory.retain_other_than(TEMPORARY_MEMORY);
        if let Some(target) = self
            .diagnostic_target
            .filter(|target| target.entity_id == id && target.stage == stage)
        {
            vm.enable_limit_diagnostics(target.event_capacity)?;
        }
        let result = match vm.execute(node) {
            Ok(result) => result,
            Err(error) => {
                let callback_error = error.context(format!(
                    "entity {id} ({}) {stage:?} callback node {node}",
                    archetype.name
                ));
                self.frame_vm_evaluations += vm.evaluation_count() as u64;
                self.frame_spawn_requests += vm.spawn_queue.len() as u64;
                for (name, count) in &vm.function_counts {
                    *self.frame_function_counts.entry(name.clone()).or_default() += count;
                }
                if let Some(trace) = vm.diagnostic_report() {
                    let partial_summary = format!(
                        "Partial frame counters: callbacks_completed={}, vm_evaluations={}, spawn_requests_produced={}, spawned_entities={}, draw_operations={}",
                        self.callback_log.len(),
                        self.frame_vm_evaluations,
                        self.frame_spawn_requests,
                        self.entities.iter().filter(|entity| entity.spawned).count(),
                        self.frame_function_counts.get("Draw").copied().unwrap_or(0)
                    );
                    return Err(anyhow!(
                        "{:#}\n{}\n{}",
                        callback_error,
                        partial_summary,
                        trace
                    ));
                }
                return Err(callback_error);
            }
        };
        let requests = std::mem::take(&mut vm.spawn_queue);
        self.frame_vm_evaluations += vm.evaluation_count() as u64;
        self.frame_spawn_requests += requests.len() as u64;
        if self.entities[id].has_entity_data && stage == LifecycleStage::Preprocess {
            self.entities[id].entity_data = self.entity_data_row(id);
        }
        self.global_memory = vm.memory.clone();
        self.global_memory.retain_other_than(ENTITY_MEMORY);
        self.global_memory.retain_other_than(ENTITY_DATA);
        self.global_memory.retain_other_than(ENTITY_SHARED_MEMORY);
        self.global_memory.retain_other_than(TEMPORARY_MEMORY);
        self.entities[id].memory = vm.memory.clone();
        // Entity Memory is the only per-entity persistent block. Keeping a
        // snapshot of global blocks here lets the next callback overwrite
        // fresh Runtime Update inputs with this entity's stale snapshot.
        self.entities[id].memory.retain_only(ENTITY_MEMORY);
        self.callback_log.push(record);
        self.frame_display_list
            .sprites
            .extend(vm.display_list.sprites);
        self.frame_scheduled_effects.extend(vm.scheduled_effects);
        self.frame_scheduled_looped_effects
            .extend(vm.scheduled_looped_effects);
        self.frame_scheduled_looped_effect_stops
            .extend(vm.scheduled_looped_effect_stops);
        self.frame_audio_events.extend(vm.audio_events);
        self.frame_destroyed_particle_effects
            .extend(vm.destroyed_particle_effects);
        self.frame_particle_events.extend(vm.particle_events);
        self.frame_debug_events.extend(vm.debug_events);
        for (name, count) in vm.function_counts {
            *self.frame_function_counts.entry(name).or_default() += count;
        }
        self.frame_skin_checks.extend(vm.skin_checks);
        for request in requests {
            self.enqueue_spawn(request.archetype_id, request.data)?;
        }
        Ok(result)
    }

    fn invoke_global(&mut self, stage: LifecycleStage, node: usize) -> Result<f64> {
        let mut vm = WatchVm::new(&self.watch.nodes);
        vm.context = self.context.clone();
        vm.context.callback_name = Some(format!("{stage:?}"));
        vm.context.callback_node = Some(node);
        vm.set_draw_tracing(self.trace_draws);
        vm.memory = self.global_memory.clone();
        vm.memory.retain_other_than(TEMPORARY_MEMORY);
        let result = vm
            .execute(node)
            .with_context(|| format!("global {stage:?} callback node {node}"))?;
        let requests = std::mem::take(&mut vm.spawn_queue);
        self.frame_vm_evaluations += vm.evaluation_count() as u64;
        self.frame_spawn_requests += requests.len() as u64;
        self.global_memory = vm.memory;
        self.global_memory.retain_other_than(TEMPORARY_MEMORY);
        self.callback_log.push(CallbackRecord {
            entity_id: None,
            archetype: "<global>".into(),
            stage,
            node,
        });
        self.frame_display_list
            .sprites
            .extend(vm.display_list.sprites);
        self.frame_scheduled_effects.extend(vm.scheduled_effects);
        self.frame_scheduled_looped_effects
            .extend(vm.scheduled_looped_effects);
        self.frame_scheduled_looped_effect_stops
            .extend(vm.scheduled_looped_effect_stops);
        self.frame_audio_events.extend(vm.audio_events);
        self.frame_destroyed_particle_effects
            .extend(vm.destroyed_particle_effects);
        self.frame_particle_events.extend(vm.particle_events);
        self.frame_debug_events.extend(vm.debug_events);
        for (name, count) in vm.function_counts {
            *self.frame_function_counts.entry(name).or_default() += count;
        }
        self.frame_skin_checks.extend(vm.skin_checks);
        for request in requests {
            self.enqueue_spawn(request.archetype_id, request.data)?;
        }
        Ok(result)
    }

    fn enqueue_spawn(&mut self, archetype_id: i64, data: Vec<f64>) -> Result<usize> {
        let archetype_index = usize::try_from(archetype_id)
            .ok()
            .filter(|&index| index < self.watch.archetypes.len())
            .ok_or_else(|| anyhow!("Spawn references unknown archetype id {archetype_id}"))?;
        let id = self.entities.len();
        let mut memory = Memory::new();
        for (slot, value) in data.into_iter().enumerate() {
            memory.set(ENTITY_MEMORY, slot, value);
        }
        self.entities.push(WatchEntity {
            id,
            archetype_index,
            archetype: self.archetype_defs[archetype_index].name.clone(),
            level_entity_index: None,
            name: None,
            spawned: true,
            has_entity_data: false,
            has_shared_memory: false,
            entity_data: Vec::new(),
            memory,
            shared_memory: Memory::new(),
            schedule: None,
            active: false,
            initialized: false,
        });
        self.pending.push(id);
        Ok(id)
    }

    fn entity_data_row(&self, id: usize) -> Vec<f64> {
        let start = id.saturating_mul(32);
        self.context
            .entity_data_array
            .read()
            .ok()
            .map(|values| {
                (0..32)
                    .map(|slot| values.get(start + slot).copied().unwrap_or(0.0))
                    .collect()
            })
            .unwrap_or_else(|| vec![0.0; 32])
    }

    fn set_entity_info_active(&mut self, id: usize, active: bool) {
        if let Ok(mut values) = self.context.entity_info_array.write() {
            let active_slot = id.saturating_mul(3).saturating_add(2);
            if let Some(value) = values.get_mut(active_slot) {
                *value = if active { 1.0 } else { 0.0 };
            }
        }
    }
}

fn bind_named_ids(
    binding_root: &Value,
    collection: &str,
    present_names: &BTreeSet<String>,
    label: &str,
) -> Result<BTreeSet<u32>> {
    let bindings = binding_root
        .get(collection)
        .and_then(Value::as_array)
        .with_context(|| format!("{label} has no {collection} array"))?;
    let mut result = BTreeSet::new();
    for (index, binding) in bindings.iter().enumerate() {
        let name = binding
            .get("name")
            .and_then(Value::as_str)
            .with_context(|| format!("{label} entry {index} has no string name"))?;
        let id = binding
            .get("id")
            .and_then(Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .with_context(|| format!("{label} entry {index} has an invalid ID"))?;
        if present_names.contains(name) {
            result.insert(id);
        }
    }
    Ok(result)
}

fn collect_entity_names(
    entities: &[crate::formats::LevelEntity],
) -> Result<BTreeMap<String, usize>> {
    let mut names = BTreeMap::new();
    for (index, entity) in entities.iter().enumerate() {
        if let Some(name) = entity.extra.get("name").and_then(Value::as_str) {
            if names.insert(name.to_owned(), index).is_some() {
                bail!("duplicate level entity name {name:?}");
            }
        }
    }
    Ok(names)
}

fn imported_value(entity: &WatchEntity, archetype: &WatchArchetype, name: &str) -> Result<f64> {
    let index = archetype
        .imports
        .iter()
        .find(|import| import.get("name").and_then(Value::as_str) == Some(name))
        .and_then(|import| import.get("index").and_then(Value::as_u64))
        .with_context(|| format!("{} archetype has no {name} import", archetype.name))?
        as usize;
    entity
        .entity_data
        .get(index)
        .copied()
        .with_context(|| format!("entity {} has no imported {name} value", entity.id))
}

fn map_imports(
    archetype: &WatchArchetype,
    raw_data: &Value,
    names: &BTreeMap<String, usize>,
    entity_index: usize,
) -> Result<Vec<f64>> {
    let data = raw_data
        .as_array()
        .with_context(|| format!("level entity {entity_index} data must be an array"))?;
    enum ImportedValue<'b> {
        Number(f64),
        Reference(&'b str),
    }
    let mut values = BTreeMap::<String, ImportedValue<'_>>::new();
    for item in data {
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .context("entity data entry is missing string name")?;
        let value = if let Some(value) = item.get("value").and_then(Value::as_f64) {
            ImportedValue::Number(value)
        } else if let Some(reference) = item.get("ref").and_then(Value::as_str) {
            ImportedValue::Reference(reference)
        } else {
            bail!(
                "entity {entity_index} data field {name:?} has neither numeric value nor reference"
            );
        };
        if values.insert(name.to_owned(), value).is_some() {
            bail!("entity {entity_index} contains duplicate data field {name:?}");
        }
    }
    let mut result = vec![0.0; 32];
    let mut occupied = BTreeSet::new();
    for import in &archetype.imports {
        let name = import
            .get("name")
            .and_then(Value::as_str)
            .context("archetype import is missing name")?;
        let index = import
            .get("index")
            .and_then(Value::as_u64)
            .context("archetype import is missing a nonnegative index")?
            as usize;
        if index >= result.len() {
            bail!(
                "archetype {} import index {index} exceeds Entity Data block",
                archetype.name
            );
        }
        if !occupied.insert(index) {
            bail!(
                "archetype {} maps multiple imports to Entity Data index {index}",
                archetype.name
            );
        }
        if let Some(value) = values.get(name) {
            result[index] = match value {
                ImportedValue::Number(value) => *value,
                ImportedValue::Reference(reference) => *names.get(*reference).ok_or_else(|| {
                    anyhow!("entity {entity_index} imported reference {name:?} -> {reference:?} does not resolve")
                })? as f64,
            };
        } else if let Some(default) = import.get("def").and_then(Value::as_f64) {
            result[index] = default;
        }
    }
    Ok(result)
}

fn callback(archetype: WatchArchetype, stage: LifecycleStage) -> Option<Value> {
    match stage {
        LifecycleStage::Preprocess => archetype.preprocess,
        LifecycleStage::SpawnTime => archetype.spawn_time,
        LifecycleStage::DespawnTime => archetype.despawn_time,
        LifecycleStage::Initialize => archetype.initialize,
        LifecycleStage::UpdateSequential => archetype.update_sequential,
        LifecycleStage::UpdateParallel => archetype.update_parallel,
        LifecycleStage::Terminate => archetype.terminate,
        LifecycleStage::UpdateSpawn => None,
    }
}

fn callback_index(value: &Value) -> Result<usize> {
    value
        .get("index")
        .and_then(Value::as_u64)
        .map(|index| index as usize)
        .context("callback has no valid node index")
}

fn callback_order(callback: &Option<Value>) -> i64 {
    callback
        .as_ref()
        .and_then(|v| v.get("order"))
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

fn in_spawn_range(entity: &WatchEntity, timeline: f64) -> bool {
    entity
        .schedule
        .is_some_and(|(start, end)| timeline >= start && timeline < end)
}
