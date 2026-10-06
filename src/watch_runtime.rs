//! Generic host for Sonolus WatchData entities.

use crate::{
    formats::LevelData,
    runtime::{
        AudioEffectEvent, DebugEvent, DestroyedParticleEffect, DisplayList, Memory,
        ParticleEffectEvent, ScheduledEffect, ScheduledLoopedEffect, ScheduledLoopedEffectStop,
        VmContext, WatchVm,
    },
    sono_gcc::{SonoGccProgram, WatchExecutionMode},
    watch::{WatchArchetype, WatchData},
};
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const ENTITY_MEMORY: i64 = 4000;
const ENTITY_DATA: i64 = 4001;
const ENTITY_SHARED_MEMORY: i64 = 4002;
const TEMPORARY_MEMORY: i64 = 10000;
const RUNTIME_UPDATE: i64 = 1001;
const RUNTIME_SKIN_TRANSFORM: i64 = 1002;
const RUNTIME_PARTICLE_TRANSFORM: i64 = 1003;
const RUNTIME_UI_CONFIGURATION: i64 = 1007;
const RUNTIME_ENVIRONMENT: i64 = 1000;
const LEVEL_OPTION: i64 = 2002;
// The VM cap is host safety machinery. Keep its old floor, scale callbacks that
// walk level data, and retain a hard ceiling for unusually large inputs.
const MIN_CALLBACK_EVALUATIONS: usize = 5_000_000;
const EVALUATIONS_PER_LEVEL_ENTITY: usize = 4_096;
const MAX_CALLBACK_EVALUATIONS: usize = 50_000_000;
// The largest supplied active wave was 152 entities; classifying it to find
// 107 safe callbacks still lost wall time. Skip scheduler analysis below 192.
const MIN_PARALLEL_STAGE_ENTITIES: usize = 192;
// A/B measurements on the supplied charts showed that batches up to 107
// callbacks cost more to schedule than they saved. Keep these ordered unless
// a contiguous batch exceeds the measured loss range.
const MIN_PARALLEL_BATCH_CALLBACKS: usize = 128;

fn callback_evaluation_limit(level_entity_count: usize) -> usize {
    MIN_CALLBACK_EVALUATIONS
        .max(level_entity_count.saturating_mul(EVALUATIONS_PER_LEVEL_ENTITY))
        .min(MAX_CALLBACK_EVALUATIONS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
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
    /// Runtime Particle Transform matrix as seen after this frame's callbacks.
    pub runtime_particle_transform: [f64; 16],
    #[serde(skip)]
    pub runtime_profile: FrameRuntimeProfile,
}

#[derive(Debug, Clone, Default)]
pub struct FrameRuntimeProfile {
    pub frame_count: u64,
    pub frame: std::time::Duration,
    pub preprocess: std::time::Duration,
    pub update_spawn: std::time::Duration,
    pub scheduling: std::time::Duration,
    pub activation: std::time::Duration,
    pub update_sequential: std::time::Duration,
    pub update_parallel: std::time::Duration,
    pub report_materialization: std::time::Duration,
    pub callback_vm_construction: std::time::Duration,
    pub callback_context_setup: std::time::Duration,
    pub callback_memory_setup: std::time::Duration,
    pub callback_execute: std::time::Duration,
    pub callback_commit: std::time::Duration,
    pub callbacks: u64,
    pub evaluations: u64,
    pub function_dispatches: u64,
    pub sono_gcc_regions_executed: u64,
    /// Scalar results evaluated by WatchVm and passed into native regions.
    pub sono_gcc_vm_to_native_cut_calls: u64,
    /// Regions that exceeded stack input storage and needed a heap buffer.
    pub sono_gcc_overflow_input_regions: u64,
    pub sono_gcc_region_dispatch: std::time::Duration,
    pub sono_gcc_vm_cut_evaluation: std::time::Duration,
    pub sono_gcc_native_execution: std::time::Duration,
    pub memory_entries_copied: u64,
    /// UpdateParallel callbacks evaluated by the pool.
    pub parallel_callbacks: u64,
    /// Speculative callbacks discarded before ordered compatibility rerun.
    pub parallel_speculative_callbacks: u64,
    /// Callbacks rerun or kept on the ordered compatibility path.
    pub parallel_ordered_callbacks: u64,
    /// Batches dispatched to persistent Watch workers.
    pub parallel_batches: u64,
    /// Sum of worker counts used by dispatched batches.
    pub parallel_worker_slots: u64,
    /// Approximate active worker time summed across all workers.
    pub parallel_worker_busy: std::time::Duration,
    /// Worker slot time available across all dispatched batches.
    pub parallel_worker_slot_time: std::time::Duration,
    /// Coordinator time spent dispatching, waiting, and collecting batches.
    pub parallel_batch_time: std::time::Duration,
    /// Estimated dispatch/wait time after subtracting the longest worker span.
    pub parallel_scheduler_wait: std::time::Duration,
    pub stepper_event_aggregation: std::time::Duration,
}

impl FrameRuntimeProfile {
    pub(crate) fn accumulate(&mut self, other: &Self) {
        self.frame_count += other.frame_count;
        self.frame += other.frame;
        self.preprocess += other.preprocess;
        self.update_spawn += other.update_spawn;
        self.scheduling += other.scheduling;
        self.activation += other.activation;
        self.update_sequential += other.update_sequential;
        self.update_parallel += other.update_parallel;
        self.report_materialization += other.report_materialization;
        self.callback_vm_construction += other.callback_vm_construction;
        self.callback_context_setup += other.callback_context_setup;
        self.callback_memory_setup += other.callback_memory_setup;
        self.callback_execute += other.callback_execute;
        self.callback_commit += other.callback_commit;
        self.callbacks += other.callbacks;
        self.evaluations += other.evaluations;
        self.function_dispatches += other.function_dispatches;
        self.sono_gcc_regions_executed += other.sono_gcc_regions_executed;
        self.sono_gcc_vm_to_native_cut_calls += other.sono_gcc_vm_to_native_cut_calls;
        self.sono_gcc_overflow_input_regions += other.sono_gcc_overflow_input_regions;
        self.sono_gcc_region_dispatch += other.sono_gcc_region_dispatch;
        self.sono_gcc_vm_cut_evaluation += other.sono_gcc_vm_cut_evaluation;
        self.sono_gcc_native_execution += other.sono_gcc_native_execution;
        self.memory_entries_copied += other.memory_entries_copied;
        self.parallel_callbacks += other.parallel_callbacks;
        self.parallel_speculative_callbacks += other.parallel_speculative_callbacks;
        self.parallel_ordered_callbacks += other.parallel_ordered_callbacks;
        self.parallel_batches += other.parallel_batches;
        self.parallel_worker_slots += other.parallel_worker_slots;
        self.parallel_worker_busy += other.parallel_worker_busy;
        self.parallel_worker_slot_time += other.parallel_worker_slot_time;
        self.parallel_batch_time += other.parallel_batch_time;
        self.parallel_scheduler_wait += other.parallel_scheduler_wait;
        self.stepper_event_aggregation += other.stepper_event_aggregation;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WatchDiagnosticTarget {
    pub entity_id: usize,
    pub stage: LifecycleStage,
    pub event_capacity: usize,
}

#[derive(Clone)]
struct ParallelCallbackTask {
    active_index: usize,
    entity_id: usize,
    archetype_name: String,
    node: usize,
    has_entity_data: bool,
    has_shared_memory: bool,
    entity_memory: Memory,
    entity_info: Option<[f64; 3]>,
}

struct ParallelCallbackBatch {
    nodes: Arc<Vec<crate::watch::EngineNode>>,
    tasks: Vec<ParallelCallbackTask>,
    global_memory: Memory,
    context: VmContext,
    stage: LifecycleStage,
    profiling: bool,
    collect_vm_accounting: bool,
    level_entity_count: usize,
    sono_gcc_program: Option<Arc<SonoGccProgram>>,
    export_control: crate::export_control::ExportControl,
    next_task: AtomicUsize,
    stop_after: AtomicUsize,
}

struct ParallelCallbackResult {
    task_index: usize,
    task: ParallelCallbackTask,
    output: crate::runtime::WatchVmCallbackOutput,
    vm_construction: Duration,
    context_setup: Duration,
    memory_setup: Duration,
    execution: Duration,
}

struct ParallelWorkerResult {
    worker_index: usize,
    callbacks: Vec<ParallelCallbackResult>,
    busy: Duration,
}

enum ParallelWorkerMessage {
    Run(
        Arc<ParallelCallbackBatch>,
        mpsc::Sender<ParallelWorkerResult>,
    ),
    Shutdown,
}

struct ParallelWorker {
    sender: SyncSender<ParallelWorkerMessage>,
    handle: Option<JoinHandle<()>>,
}

struct ParallelWorkerPool {
    workers: Vec<ParallelWorker>,
}

impl ParallelWorkerPool {
    fn new(count: usize) -> Result<Self> {
        let mut workers: Vec<ParallelWorker> = Vec::with_capacity(count);
        for worker_index in 0..count {
            let (sender, receiver) = mpsc::sync_channel(1);
            let handle = match thread::Builder::new()
                .name(format!("sono-watch-{worker_index}"))
                .spawn(move || parallel_worker_loop(worker_index, receiver))
            {
                Ok(handle) => handle,
                Err(error) => {
                    for worker in &workers {
                        let _ = worker.sender.send(ParallelWorkerMessage::Shutdown);
                    }
                    for worker in &mut workers {
                        if let Some(handle) = worker.handle.take() {
                            let _ = handle.join();
                        }
                    }
                    return Err(error.into());
                }
            };
            workers.push(ParallelWorker {
                sender,
                handle: Some(handle),
            });
        }
        Ok(Self { workers })
    }

    fn worker_count(&self) -> usize {
        self.workers.len()
    }

    fn run(
        &self,
        batch: Arc<ParallelCallbackBatch>,
        active_workers: usize,
    ) -> Result<(Vec<ParallelCallbackResult>, Duration, Duration, Duration)> {
        let wall_start = std::time::Instant::now();
        let (sender, receiver) = mpsc::channel();
        for (worker_index, worker) in self.workers.iter().take(active_workers).enumerate() {
            worker
                .sender
                .send(ParallelWorkerMessage::Run(batch.clone(), sender.clone()))
                .with_context(|| format!("sending Watch work to worker {worker_index}"))?;
        }
        drop(sender);
        let mut results = Vec::new();
        let mut worker_busy = Duration::ZERO;
        let mut longest_worker = Duration::ZERO;
        let mut worker_ids = BTreeSet::new();
        let mut responses = 0;
        for response in receiver {
            responses += 1;
            if !worker_ids.insert(response.worker_index) {
                bail!(
                    "Watch worker {} returned twice for one batch",
                    response.worker_index
                );
            }
            worker_busy += response.busy;
            longest_worker = longest_worker.max(response.busy);
            results.extend(response.callbacks);
        }
        if responses != active_workers {
            bail!("Watch worker pool returned {responses} of {active_workers} batch results");
        }
        results.sort_by_key(|result| result.task_index);
        let wall = wall_start.elapsed();
        Ok((
            results,
            worker_busy,
            wall,
            wall.saturating_sub(longest_worker),
        ))
    }
}

impl Drop for ParallelWorkerPool {
    fn drop(&mut self) {
        for worker in &self.workers {
            let _ = worker.sender.send(ParallelWorkerMessage::Shutdown);
        }
        for worker in &mut self.workers {
            if let Some(handle) = worker.handle.take() {
                let _ = handle.join();
            }
        }
    }
}

fn parallel_worker_loop(worker_index: usize, receiver: Receiver<ParallelWorkerMessage>) {
    while let Ok(message) = receiver.recv() {
        let ParallelWorkerMessage::Run(batch, sender) = message else {
            break;
        };
        let busy_start = std::time::Instant::now();
        let mut callbacks = Vec::new();
        loop {
            if batch.export_control.is_cancelled() {
                break;
            }
            let task_index = batch.next_task.fetch_add(1, Ordering::Relaxed);
            if task_index >= batch.tasks.len()
                || task_index > batch.stop_after.load(Ordering::Acquire)
            {
                break;
            }
            let task = batch.tasks[task_index].clone();
            let result = execute_parallel_callback(&batch, task_index, task);
            let unexpected_shared_write = result.output.result.is_ok()
                && result.output.memory.differs_outside_overlay(
                    &result.task.entity_memory,
                    &[ENTITY_MEMORY, TEMPORARY_MEMORY],
                );
            if result.output.result.is_err() || unexpected_shared_write {
                batch.stop_after.fetch_min(task_index, Ordering::AcqRel);
            }
            callbacks.push(result);
        }
        let busy = busy_start.elapsed();
        if sender
            .send(ParallelWorkerResult {
                worker_index,
                callbacks,
                busy,
            })
            .is_err()
        {
            break;
        }
    }
}

fn execute_parallel_callback(
    batch: &ParallelCallbackBatch,
    task_index: usize,
    task: ParallelCallbackTask,
) -> ParallelCallbackResult {
    let phase_start = batch.profiling.then(std::time::Instant::now);
    let mut vm = WatchVm::new(batch.nodes.as_slice());
    vm.set_profiling(batch.profiling);
    vm.set_accounting(batch.collect_vm_accounting);
    vm.set_sono_gcc_program(batch.sono_gcc_program.clone());
    if let Some(start) = phase_start {
        let vm_construction = start.elapsed();
        let context_start = batch.profiling.then(std::time::Instant::now);
        vm.set_evaluation_limit(callback_evaluation_limit(batch.level_entity_count));
        vm.context = batch.context.clone();
        vm.context.entity_data_array_writable = false;
        vm.context.lifecycle_stage = Some(batch.stage as u8);
        vm.context.entity_id = Some(task.entity_id);
        vm.context.entity_archetype = Some(task.archetype_name.clone());
        vm.context.callback_name = Some(format!("{:?}", batch.stage));
        vm.context.callback_node = Some(task.node);
        vm.context.has_entity_data = task.has_entity_data;
        vm.context.has_entity_shared_memory = task.has_shared_memory;
        vm.context.entity_info = task.entity_info;
        let context_setup = context_start.map_or(Duration::ZERO, |start| start.elapsed());
        let memory_start = batch.profiling.then(std::time::Instant::now);
        vm.memory = Memory::callback_view(&batch.global_memory, &task.entity_memory);
        vm.memory.retain_other_than(TEMPORARY_MEMORY);
        let memory_setup = memory_start.map_or(Duration::ZERO, |start| start.elapsed());
        let execute_start = batch.profiling.then(std::time::Instant::now);
        let execution = vm.execute(task.node);
        let execution_time = execute_start.map_or(Duration::ZERO, |start| start.elapsed());
        let output = vm.into_callback_output(execution);
        ParallelCallbackResult {
            task_index,
            task,
            output,
            vm_construction,
            context_setup,
            memory_setup,
            execution: execution_time,
        }
    } else {
        vm.set_evaluation_limit(callback_evaluation_limit(batch.level_entity_count));
        vm.context = batch.context.clone();
        vm.context.entity_data_array_writable = false;
        vm.context.lifecycle_stage = Some(batch.stage as u8);
        vm.context.entity_id = Some(task.entity_id);
        vm.context.entity_archetype = Some(task.archetype_name.clone());
        vm.context.callback_name = Some(format!("{:?}", batch.stage));
        vm.context.callback_node = Some(task.node);
        vm.context.has_entity_data = task.has_entity_data;
        vm.context.has_entity_shared_memory = task.has_shared_memory;
        vm.context.entity_info = task.entity_info;
        vm.memory = Memory::callback_view(&batch.global_memory, &task.entity_memory);
        vm.memory.retain_other_than(TEMPORARY_MEMORY);
        let execution = vm.execute(task.node);
        ParallelCallbackResult {
            task_index,
            task,
            output: vm.into_callback_output(execution),
            vm_construction: Duration::ZERO,
            context_setup: Duration::ZERO,
            memory_setup: Duration::ZERO,
            execution: Duration::ZERO,
        }
    }
}

/// Watch host. Level entities are kept in source order; callback `order` only
/// changes the order within a lifecycle system and ties preserve source order.
pub struct WatchRuntime<'a> {
    export_control: crate::export_control::ExportControl,
    watch: &'a WatchData,
    level_entity_count: usize,
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
    profiling: bool,
    collect_vm_accounting: bool,
    execution_mode: WatchExecutionMode,
    parallel_updates_enabled: bool,
    parallel_worker_count: Option<usize>,
    parallel_workers: Option<ParallelWorkerPool>,
    ordered_parallel_callbacks: BTreeMap<(LifecycleStage, usize), bool>,
    sono_gcc_program: Option<Arc<SonoGccProgram>>,
    sono_gcc_compile_error: Option<String>,
    sono_gcc_precompile_deferred: bool,
    sono_gcc_compile_time: Duration,
    sono_gcc_native_load_time: Duration,
    sono_gcc_cache_lookup_time: Duration,
    sono_gcc_graph_identity_time: Duration,
    sono_gcc_disk_validation_time: Duration,
    sono_gcc_metadata_reconstruction_time: Duration,
    sono_gcc_cache_write_time: Duration,
    sono_gcc_cache_key: Option<String>,
    sono_gcc_cache_hit: crate::sono_gcc::CacheHit,
    sono_gcc_cache_write_error: Option<String>,
    sono_gcc_graph_nodes: usize,
    sono_gcc_compiled_operations: u64,
    sono_gcc_eligible_operations: u64,
    sono_gcc_execution_reported: bool,
    frame_profile: FrameRuntimeProfile,
}

impl<'a> WatchRuntime<'a> {
    pub(crate) fn set_export_control(&mut self, control: crate::export_control::ExportControl) {
        self.export_control = control;
        if self.execution_mode == WatchExecutionMode::SonoGcc {
            self.export_control.log(self.execution_mode_status());
        }
    }

    pub fn set_execution_mode(&mut self, mode: WatchExecutionMode) {
        self.set_execution_mode_with_compile_policy(mode, true);
    }

    /// Configure UpdateParallel execution. `worker_count = None` uses the
    /// machine's available parallelism. Disabling it keeps the legacy ordered
    /// callback path and never creates a worker pool.
    pub fn set_parallel_updates(
        &mut self,
        enabled: bool,
        worker_count: Option<usize>,
    ) -> Result<()> {
        if worker_count.is_some_and(|count| !(1..=256).contains(&count)) {
            bail!("Watch worker count must be in 1..=256");
        }
        if self.parallel_updates_enabled != enabled || self.parallel_worker_count != worker_count {
            self.parallel_workers = None;
        }
        self.parallel_updates_enabled = enabled;
        self.parallel_worker_count = worker_count;
        Ok(())
    }

    pub fn parallel_updates_status(&self) -> String {
        if !self.parallel_updates_enabled {
            "ordered single-threaded Watch execution selected".to_owned()
        } else {
            let workers = self.parallel_worker_count.map_or_else(
                || "automatic worker count".to_owned(),
                |count| format!("{count} Watch workers"),
            );
            format!("parallel Watch UpdateParallel enabled ({workers})")
        }
    }

    pub(crate) fn set_execution_mode_with_compile_policy(
        &mut self,
        mode: WatchExecutionMode,
        compile_on_demand: bool,
    ) {
        if self.execution_mode == mode
            && (mode == WatchExecutionMode::Interpreter
                || self.sono_gcc_program.is_some()
                || !compile_on_demand && !self.sono_gcc_precompile_deferred)
        {
            return;
        }
        self.execution_mode = mode;
        self.sono_gcc_program = None;
        self.sono_gcc_compile_error = None;
        self.sono_gcc_precompile_deferred = false;
        self.sono_gcc_compile_time = Duration::ZERO;
        self.sono_gcc_native_load_time = Duration::ZERO;
        self.sono_gcc_cache_lookup_time = Duration::ZERO;
        self.sono_gcc_graph_identity_time = Duration::ZERO;
        self.sono_gcc_disk_validation_time = Duration::ZERO;
        self.sono_gcc_metadata_reconstruction_time = Duration::ZERO;
        self.sono_gcc_cache_write_time = Duration::ZERO;
        self.sono_gcc_cache_key = None;
        self.sono_gcc_cache_hit = crate::sono_gcc::CacheHit::None;
        self.sono_gcc_cache_write_error = None;
        self.sono_gcc_graph_nodes = 0;
        self.sono_gcc_compiled_operations = 0;
        self.sono_gcc_eligible_operations = 0;
        if mode == WatchExecutionMode::SonoGcc {
            if self.context.execution_trace.is_some() || self.trace_draws {
                self.sono_gcc_compile_error = Some(
                    "Watch tracing requires interpreter execution to preserve node diagnostics"
                        .to_owned(),
                );
            } else {
                let result = if compile_on_demand {
                    SonoGccProgram::compile_cached_detailed(&self.watch.nodes)
                } else {
                    let cached = SonoGccProgram::compile_if_process_cached(&self.watch.nodes);
                    self.sono_gcc_precompile_deferred = cached.as_ref().is_ok_and(Option::is_none);
                    cached.and_then(|cached| {
                        cached.ok_or_else(|| {
                            anyhow!("background precompilation has not loaded this graph yet")
                        })
                    })
                };
                match result {
                    Ok(result) => {
                        self.sono_gcc_compile_time = result.timings.compilation;
                        self.sono_gcc_native_load_time = result.timings.native_load;
                        self.sono_gcc_cache_lookup_time = result.timings.cache_lookup;
                        self.sono_gcc_graph_identity_time = result.timings.graph_identity;
                        self.sono_gcc_disk_validation_time = result.timings.disk_validation;
                        self.sono_gcc_metadata_reconstruction_time =
                            result.timings.metadata_reconstruction;
                        self.sono_gcc_cache_write_time = result.timings.cache_write;
                        self.sono_gcc_cache_key = Some(result.cache_key);
                        self.sono_gcc_cache_hit = result.cache_hit;
                        self.sono_gcc_cache_write_error = result.cache_write_error;
                        self.sono_gcc_graph_nodes = result.graph_nodes;
                        self.sono_gcc_compiled_operations = result.compiled_operations;
                        self.sono_gcc_eligible_operations = result.eligible_operations;
                        self.sono_gcc_program = Some(result.program);
                    }
                    Err(error) => self.sono_gcc_compile_error = Some(format!("{error:#}")),
                }
            }
            self.export_control.log(self.execution_mode_status());
        }
    }

    pub fn execution_mode_status(&self) -> String {
        match self.execution_mode {
            WatchExecutionMode::Interpreter => "Sono VM / interpreter selected".to_owned(),
            WatchExecutionMode::SonoGcc => match (&self.sono_gcc_program, &self.sono_gcc_compile_error) {
                (Some(program), _) => format!(
                    "Sono-GCC ready ({}): {} regions, {} / {} eligible native operation nodes across {} graph nodes; compile {:.3}s, cache lookup {:.3}s, DLL load {:.3}s, cache write {:.3}s, graph identity {:.3}s, disk validation {:.3}s, metadata reconstruction {:.3}s; WatchVm executes ordered scalar cuts and retains runtime/effect semantics; other graph paths use Sono VM{} (cache key {})",
                    match self.sono_gcc_cache_hit {
                        crate::sono_gcc::CacheHit::None => "compiled".to_owned(),
                        crate::sono_gcc::CacheHit::Process => "process cache hit".to_owned(),
                        crate::sono_gcc::CacheHit::Persistent => "persistent cache hit".to_owned(),
                    },
                    program.region_count(),
                    self.sono_gcc_compiled_operations,
                    self.sono_gcc_eligible_operations,
                    self.sono_gcc_graph_nodes,
                    self.sono_gcc_compile_time.as_secs_f64(),
                    self.sono_gcc_cache_lookup_time.as_secs_f64(),
                    self.sono_gcc_native_load_time.as_secs_f64(),
                    self.sono_gcc_cache_write_time.as_secs_f64(),
                    self.sono_gcc_graph_identity_time.as_secs_f64(),
                    self.sono_gcc_disk_validation_time.as_secs_f64(),
                    self.sono_gcc_metadata_reconstruction_time.as_secs_f64(),
                    self.sono_gcc_cache_write_error.as_ref().map_or_else(String::new, |error| format!("; cache persistence unavailable: {error}")),
                    self.sono_gcc_cache_key.as_deref().unwrap_or("unknown")
                ),
                (_, Some(error)) if self.sono_gcc_precompile_deferred => format!(
                    "Sono-GCC precompile pending; using Sono VM for this render: {error}"
                ),
                (_, Some(error)) => {
                    format!("Sono-GCC unavailable; falling back to Sono VM: {error}")
                }
                _ => "Sono-GCC selected; native compilation has not completed; using Sono VM".to_owned(),
            },
        }
    }
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

        let level_entity_count = level.entities.len();
        let names = collect_entity_names(&level.entities)?;
        let mut runtime = Self {
            export_control: Default::default(),
            watch,
            level_entity_count,
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
            profiling: false,
            collect_vm_accounting: true,
            execution_mode: WatchExecutionMode::Interpreter,
            parallel_updates_enabled: true,
            parallel_worker_count: None,
            parallel_workers: None,
            ordered_parallel_callbacks: BTreeMap::new(),
            sono_gcc_program: None,
            sono_gcc_compile_error: None,
            sono_gcc_precompile_deferred: false,
            sono_gcc_compile_time: Duration::ZERO,
            sono_gcc_native_load_time: Duration::ZERO,
            sono_gcc_cache_lookup_time: Duration::ZERO,
            sono_gcc_graph_identity_time: Duration::ZERO,
            sono_gcc_disk_validation_time: Duration::ZERO,
            sono_gcc_metadata_reconstruction_time: Duration::ZERO,
            sono_gcc_cache_write_time: Duration::ZERO,
            sono_gcc_cache_key: None,
            sono_gcc_cache_hit: crate::sono_gcc::CacheHit::None,
            sono_gcc_cache_write_error: None,
            sono_gcc_graph_nodes: 0,
            sono_gcc_compiled_operations: 0,
            sono_gcc_eligible_operations: 0,
            sono_gcc_execution_reported: false,
            frame_profile: FrameRuntimeProfile::default(),
        };
        for index in 0..16 {
            runtime.global_memory.set(
                RUNTIME_SKIN_TRANSFORM,
                index,
                if index % 5 == 0 { 1.0 } else { 0.0 },
            );
            runtime.global_memory.set(
                RUNTIME_PARTICLE_TRANSFORM,
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
                if callback_ref(&self.archetype_defs[entity.archetype_index], target.stage)
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

    pub fn set_profiling(&mut self, enabled: bool) {
        self.profiling = enabled;
    }

    pub fn set_vm_accounting(&mut self, enabled: bool) {
        self.collect_vm_accounting = enabled;
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

    /// Bind EngineConfiguration UI visibility pairs into Runtime UI Configuration.
    pub fn bind_engine_ui_configuration(&mut self, configuration: &Value) -> Result<()> {
        let Some(ui) = configuration.get("ui") else {
            return Ok(());
        };
        for (field, index) in [
            ("menuVisibility", 0),
            ("judgmentVisibility", 2),
            ("comboVisibility", 4),
            ("primaryMetricVisibility", 6),
            ("secondaryMetricVisibility", 8),
        ] {
            let visibility = ui
                .get(field)
                .with_context(|| format!("EngineConfiguration ui has no {field}"))?;
            let scale = visibility
                .get("scale")
                .and_then(Value::as_f64)
                .with_context(|| format!("EngineConfiguration ui.{field}.scale is invalid"))?;
            let alpha = visibility
                .get("alpha")
                .and_then(Value::as_f64)
                .with_context(|| format!("EngineConfiguration ui.{field}.alpha is invalid"))?;
            if !scale.is_finite() || !alpha.is_finite() {
                bail!("EngineConfiguration ui.{field} values must be finite");
            }
            self.global_memory
                .set(RUNTIME_UI_CONFIGURATION, index, scale);
            self.global_memory
                .set(RUNTIME_UI_CONFIGURATION, index + 1, alpha);
        }
        Ok(())
    }

    /// Override selected Level Option slots after defaults are bound and before
    /// preprocessing. Indices are the EngineConfiguration option positions.
    pub fn bind_engine_option_overrides(
        &mut self,
        configuration: &Value,
        overrides: &[(usize, f64)],
    ) -> Result<()> {
        let options = configuration
            .get("options")
            .and_then(Value::as_array)
            .context("EngineConfiguration has no options array")?;
        for &(index, value) in overrides {
            if index >= options.len() {
                bail!(
                    "Level Option override index {index} is outside EngineConfiguration options ({} entries)",
                    options.len()
                );
            }
            if !value.is_finite() {
                bail!("Level Option override at index {index} must be finite");
            }
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
            callback_order(
                self.archetype_defs[self.entities[id].archetype_index]
                    .preprocess
                    .as_ref(),
            )
        });
        if self.parallel_scheduling_available() {
            self.run_parallel_entity_stage(&order, LifecycleStage::Preprocess)?;
        } else {
            for id in order {
                self.invoke_entity(id, LifecycleStage::Preprocess)?;
            }
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

    /// Export-boundary evidence only: one-time preprocessing and cached schedules.
    /// This is not a playback frame and never runs initialize/update/terminate.
    pub(crate) fn export_schedule_evidence(&mut self) -> Result<FrameReport> {
        self.preprocess()?;
        // Match frame(0)'s post-preprocess runtime inputs. updateSpawn itself
        // is skipped only after the exporter proves a pure affine clock.
        self.context.time = 0.0;
        self.global_memory.set(RUNTIME_UPDATE, 0, 0.0);
        self.global_memory.set(RUNTIME_UPDATE, 1, 0.0);
        self.global_memory
            .set(RUNTIME_UPDATE, 2, self.context.scaled_time(0.0)?);
        self.global_memory.set(RUNTIME_UPDATE, 3, 0.0);
        self.context.timescale = self.context.time_to_timescale(0.0)?;
        self.compute_schedules()?;
        Ok(FrameReport {
            runtime_entity_count: self.entities.len(),
            callbacks: self.callback_log.clone(),
            scheduled_effects: self.frame_scheduled_effects.clone(),
            scheduled_looped_effects: self.frame_scheduled_looped_effects.clone(),
            scheduled_looped_effect_stops: self.frame_scheduled_looped_effect_stops.clone(),
            audio_events: self.frame_audio_events.clone(),
            ..FrameReport::default()
        })
    }

    /// Advance one deterministic Watch frame at a caller-supplied time.
    pub fn frame(&mut self, time: f64) -> Result<FrameReport> {
        self.export_control.check()?;
        if !time.is_finite() {
            bail!("Watch frame time must be finite");
        }
        self.frame_profile = FrameRuntimeProfile::default();
        let frame_start = self.profiling.then(std::time::Instant::now);
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
        let phase_start = self.profiling.then(std::time::Instant::now);
        self.preprocess()?;
        if let Some(start) = phase_start {
            self.frame_profile.preprocess += start.elapsed();
        }
        self.context.time = time;
        let scaled_time = self.context.scaled_time(time)?;
        self.global_memory.set(RUNTIME_UPDATE, 0, time);
        self.global_memory.set(RUNTIME_UPDATE, 1, delta_time);
        self.global_memory.set(RUNTIME_UPDATE, 2, scaled_time);
        self.global_memory.set(RUNTIME_UPDATE, 3, 0.0);
        self.context.timescale = self.context.time_to_timescale(time)?;
        let phase_start = self.profiling.then(std::time::Instant::now);
        let timeline = if let Some(node) = self.watch.update_spawn {
            self.invoke_global(LifecycleStage::UpdateSpawn, node)?
        } else {
            time
        };
        if let Some(start) = phase_start {
            self.frame_profile.update_spawn += start.elapsed();
        }

        let phase_start = self.profiling.then(std::time::Instant::now);
        self.compute_schedules()?;
        if let Some(start) = phase_start {
            self.frame_profile.scheduling += start.elapsed();
        }

        let phase_start = self.profiling.then(std::time::Instant::now);
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
        if self.parallel_scheduling_available() {
            self.run_parallel_entity_stage(&entering, LifecycleStage::Initialize)?;
            for id in entering {
                self.entities[id].initialized = true;
            }
        } else {
            for id in entering {
                self.invoke_entity(id, LifecycleStage::Initialize)?;
                self.entities[id].initialized = true;
            }
        }
        if let Some(start) = phase_start {
            self.frame_profile.activation += start.elapsed();
        }

        let phase_start = self.profiling.then(std::time::Instant::now);
        self.run_active_stage(LifecycleStage::UpdateSequential)?;
        if let Some(start) = phase_start {
            self.frame_profile.update_sequential += start.elapsed();
        }
        let phase_start = self.profiling.then(std::time::Instant::now);
        self.run_active_stage(LifecycleStage::UpdateParallel)?;
        if let Some(start) = phase_start {
            self.frame_profile.update_parallel += start.elapsed();
        }
        self.last_frame_time = Some(time);

        let materialization_start = self.profiling.then(std::time::Instant::now);
        let mut report = FrameReport {
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
            runtime_particle_transform: std::array::from_fn(|index| {
                self.global_memory.get(RUNTIME_PARTICLE_TRANSFORM, index)
            }),
            runtime_profile: self.frame_profile.clone(),
        };
        if let Some(start) = materialization_start {
            report.runtime_profile.report_materialization += start.elapsed();
        }
        if let Some(start) = frame_start {
            report.runtime_profile.frame += start.elapsed();
            report.runtime_profile.frame_count = 1;
        }
        if self.execution_mode == WatchExecutionMode::SonoGcc && !self.sono_gcc_execution_reported {
            self.export_control.log(format!(
                "Sono-GCC first-frame execution: {} compiled scalar regions ran; all other paths stayed on Sono VM",
                report.runtime_profile.sono_gcc_regions_executed
            ));
            self.sono_gcc_execution_reported = true;
        }
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
            callback_order(
                self.archetype_defs[self.entities[id].archetype_index]
                    .spawn_time
                    .as_ref(),
            )
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
            callback_order(
                self.archetype_defs[self.entities[id].archetype_index]
                    .despawn_time
                    .as_ref(),
            )
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
            callback_order(callback_ref(
                &self.archetype_defs[self.entities[id].archetype_index],
                stage,
            ))
        });
        if stage != LifecycleStage::UpdateSequential && self.parallel_scheduling_available() {
            return self.run_parallel_entity_stage(&active, stage);
        }
        for id in active {
            self.invoke_entity(id, stage)?;
        }
        Ok(())
    }

    fn parallel_worker_limit(&self) -> usize {
        self.parallel_worker_count.unwrap_or_else(|| {
            thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1)
        })
    }

    fn parallel_scheduling_available(&self) -> bool {
        self.parallel_updates_enabled
            && self.parallel_worker_limit() > 1
            && self.context.execution_trace.is_none()
            && !self.trace_draws
            && self.diagnostic_target.is_none()
    }

    fn entity_callback_node(&self, id: usize, stage: LifecycleStage) -> Result<Option<usize>> {
        let entity = self.entities.get(id).context("entity id out of range")?;
        let archetype = self
            .archetype_defs
            .get(entity.archetype_index)
            .context("entity archetype index out of range")?;
        let Some(callback) = callback_ref(archetype, stage) else {
            return Ok(None);
        };
        callback_index(callback)
            .with_context(|| format!("invalid {stage:?} callback for {}", archetype.name))
            .map(Some)
    }

    /// Reject callbacks with shared host state that cannot be isolated safely.
    /// Preprocess additionally permits only per-entity row access; direct array
    /// access and indirect block addresses remain ordered because they may
    /// observe another callback's writes.
    fn callback_requires_ordered_parallel_execution(
        &mut self,
        root: usize,
        stage: LifecycleStage,
    ) -> bool {
        if let Some(&requires_order) = self.ordered_parallel_callbacks.get(&(stage, root)) {
            return requires_order;
        }
        const ORDERED_FUNCTIONS: &[&str] = &[
            "PlayLooped",
            "PlayLoopedScheduled",
            "SpawnParticleEffect",
            "MoveParticleEffect",
            "DestroyParticleEffect",
        ];
        let mut pending = vec![root];
        let mut seen = BTreeSet::new();
        let mut requires_order = false;
        while let Some(node_index) = pending.pop() {
            if !seen.insert(node_index) {
                continue;
            }
            let Some(node) = self.watch.nodes.get(node_index) else {
                requires_order = true;
                break;
            };
            if node
                .func
                .as_deref()
                .is_some_and(|name| ORDERED_FUNCTIONS.contains(&name))
            {
                requires_order = true;
                break;
            }
            if stage == LifecycleStage::Preprocess
                && self.preprocess_node_requires_ordered_execution(node)
            {
                requires_order = true;
                break;
            }
            for argument in &node.args {
                match argument.as_u64() {
                    Some(child) => pending.push(child as usize),
                    None => {
                        requires_order = true;
                        break;
                    }
                }
            }
            if requires_order {
                break;
            }
        }
        self.ordered_parallel_callbacks
            .insert((stage, root), requires_order);
        requires_order
    }

    fn preprocess_node_requires_ordered_execution(&self, node: &crate::watch::EngineNode) -> bool {
        let Some(name) = node.func.as_deref() else {
            return false;
        };
        let reads_address = matches!(name, "Get" | "GetShifted" | "GetPointed");
        let writes_address = matches!(
            name,
            "Set"
                | "SetAdd"
                | "SetSubtract"
                | "SetMultiply"
                | "SetDivide"
                | "SetMod"
                | "SetRem"
                | "SetPower"
                | "SetShifted"
                | "SetAddShifted"
                | "SetSubtractShifted"
                | "SetMultiplyShifted"
                | "SetDivideShifted"
                | "SetModShifted"
                | "SetRemShifted"
                | "SetPowerShifted"
                | "IncrementPre"
                | "IncrementPost"
                | "DecrementPre"
                | "DecrementPost"
                | "IncrementPreShifted"
                | "IncrementPostShifted"
                | "DecrementPreShifted"
                | "DecrementPostShifted"
        );
        if matches!(name, "GetPointed") || name.ends_with("Pointed") || name == "Copy" {
            return true;
        }
        if !reads_address && !writes_address {
            return false;
        }
        let Some(block_node) = node
            .args
            .first()
            .and_then(serde_json::Value::as_u64)
            .and_then(|index| self.watch.nodes.get(index as usize))
        else {
            return true;
        };
        let Some(block_value) = block_node
            .value
            .as_ref()
            .and_then(serde_json::Value::as_f64)
        else {
            return true;
        };
        let Some(block) = block_value
            .is_finite()
            .then_some(block_value)
            .filter(|value| value.fract() == 0.0)
            .map(|value| value as i64)
        else {
            return true;
        };
        if reads_address {
            matches!(block, 4101 | 4102)
        } else {
            matches!(block, 1004 | 4001 | 4002 | 4101 | 4102)
        }
    }

    fn run_parallel_entity_stage(&mut self, active: &[usize], stage: LifecycleStage) -> Result<()> {
        if active.len() < MIN_PARALLEL_STAGE_ENTITIES {
            for &id in active {
                self.invoke_entity(id, stage)?;
            }
            return Ok(());
        }
        let mut position = 0;
        while position < active.len() {
            let mut tasks = Vec::new();
            while position < active.len() {
                let id = active[position];
                let Some(node) = self.entity_callback_node(id, stage)? else {
                    if stage == LifecycleStage::Initialize {
                        self.entities[id].initialized = true;
                    }
                    position += 1;
                    break;
                };
                if self.callback_requires_ordered_parallel_execution(node, stage) {
                    if tasks.is_empty() {
                        if self.profiling {
                            self.frame_profile.parallel_ordered_callbacks += 1;
                        }
                        self.invoke_entity(id, stage)?;
                        position += 1;
                    }
                    break;
                }
                let entity = &self.entities[id];
                tasks.push(ParallelCallbackTask {
                    active_index: position,
                    entity_id: id,
                    archetype_name: entity.archetype.clone(),
                    node,
                    has_entity_data: entity.has_entity_data,
                    has_shared_memory: entity.has_shared_memory,
                    entity_memory: entity.memory.clone(),
                    entity_info: (!entity.spawned).then_some([
                        id as f64,
                        entity.archetype_index as f64,
                        if entity.active { 1.0 } else { 0.0 },
                    ]),
                });
                position += 1;
            }

            if tasks.is_empty() {
                continue;
            }
            if tasks.len() == 1 {
                if self.profiling {
                    self.frame_profile.parallel_ordered_callbacks += 1;
                }
                self.invoke_entity(tasks[0].entity_id, stage)?;
                continue;
            }
            if tasks.len() < MIN_PARALLEL_BATCH_CALLBACKS {
                if self.profiling {
                    self.frame_profile.parallel_ordered_callbacks += tasks.len() as u64;
                }
                for task in tasks {
                    self.invoke_entity(task.entity_id, stage)?;
                }
                continue;
            }
            if self.execute_parallel_callback_batch(tasks, active, stage)? {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Run one contiguous pure callback range concurrently, then commit in
    /// source order. Returns true when a dynamic shared-memory write requires
    /// the remainder of the stage to use the ordered compatibility executor.
    fn execute_parallel_callback_batch(
        &mut self,
        tasks: Vec<ParallelCallbackTask>,
        active: &[usize],
        stage: LifecycleStage,
    ) -> Result<bool> {
        let requested_workers = self.parallel_worker_limit();
        let target_workers = tasks.len().min(requested_workers);
        if self
            .parallel_workers
            .as_ref()
            .is_none_or(|pool| pool.worker_count() < target_workers)
        {
            self.parallel_workers = None;
            self.parallel_workers = Some(
                ParallelWorkerPool::new(target_workers)
                    .context("starting persistent Watch worker pool")?,
            );
        }
        let active_workers = tasks.len().min(
            self.parallel_workers
                .as_ref()
                .map(ParallelWorkerPool::worker_count)
                .unwrap_or(0),
        );
        if active_workers <= 1 {
            for task in tasks {
                if self.profiling {
                    self.frame_profile.parallel_ordered_callbacks += 1;
                }
                self.invoke_entity(task.entity_id, stage)?;
            }
            return Ok(false);
        }

        let batch = Arc::new(ParallelCallbackBatch {
            nodes: self.watch.nodes.clone(),
            tasks,
            global_memory: self.global_memory.clone(),
            context: self.context.clone(),
            stage,
            profiling: self.profiling,
            collect_vm_accounting: self.collect_vm_accounting,
            level_entity_count: self.level_entity_count,
            sono_gcc_program: self.sono_gcc_program.clone(),
            export_control: self.export_control.clone(),
            next_task: AtomicUsize::new(0),
            stop_after: AtomicUsize::new(usize::MAX),
        });
        let (mut results, worker_busy, batch_wall, scheduler_wait) = self
            .parallel_workers
            .as_ref()
            .context("Watch worker pool disappeared")?
            .run(batch.clone(), active_workers)?;
        if self.profiling {
            self.frame_profile.parallel_batches += 1;
            self.frame_profile.parallel_worker_slots += active_workers as u64;
            self.frame_profile.parallel_worker_busy += worker_busy;
            self.frame_profile.parallel_worker_slot_time +=
                batch_wall.saturating_mul(active_workers as u32);
            self.frame_profile.parallel_batch_time += batch_wall;
            self.frame_profile.parallel_scheduler_wait += scheduler_wait;
        }
        self.export_control.check()?;

        for result in results.drain(..) {
            let task_index = result.task_index;
            let task = &batch.tasks[task_index];
            let unexpected_global_write = result.output.result.is_ok()
                && result.output.memory.differs_outside_overlay(
                    &task.entity_memory,
                    &[ENTITY_MEMORY, TEMPORARY_MEMORY],
                );
            if unexpected_global_write {
                self.accumulate_parallel_callback_profile(&result, true);
                let active_index = result.task.active_index;
                if self.profiling {
                    self.frame_profile.parallel_ordered_callbacks +=
                        (active.len() - active_index) as u64;
                }
                self.invoke_entity(result.task.entity_id, stage)?;
                for &id in &active[active_index + 1..] {
                    self.invoke_entity(id, stage)?;
                }
                return Ok(true);
            }
            self.commit_parallel_callback(result, stage)?;
        }
        Ok(false)
    }

    fn accumulate_parallel_callback_profile(
        &mut self,
        callback: &ParallelCallbackResult,
        speculative: bool,
    ) {
        let output = &callback.output;
        self.frame_profile.sono_gcc_regions_executed += output.sono_gcc_regions_executed;
        self.frame_profile.sono_gcc_vm_to_native_cut_calls +=
            output.sono_gcc_vm_to_native_cut_calls;
        self.frame_profile.sono_gcc_overflow_input_regions +=
            output.sono_gcc_overflow_input_regions;
        self.frame_profile.sono_gcc_region_dispatch += output.sono_gcc_profile_times.0;
        self.frame_profile.sono_gcc_vm_cut_evaluation += output.sono_gcc_profile_times.1;
        self.frame_profile.sono_gcc_native_execution += output.sono_gcc_profile_times.2;
        if self.profiling {
            self.frame_profile.callbacks += 1;
            self.frame_profile.callback_vm_construction += callback.vm_construction;
            self.frame_profile.callback_context_setup += callback.context_setup;
            self.frame_profile.callback_memory_setup += callback.memory_setup;
            self.frame_profile.callback_execute += callback.execution;
            self.frame_profile.evaluations += output.evaluations as u64;
            self.frame_profile.function_dispatches += output.function_dispatches;
            self.frame_profile.memory_entries_copied += output.memory.copied_entries();
            self.frame_profile.parallel_callbacks += 1;
            if speculative {
                self.frame_profile.parallel_speculative_callbacks += 1;
            }
        }
    }

    fn commit_parallel_callback(
        &mut self,
        callback: ParallelCallbackResult,
        stage: LifecycleStage,
    ) -> Result<f64> {
        self.accumulate_parallel_callback_profile(&callback, false);
        let task = callback.task;
        let output = callback.output;
        let result = match output.result {
            Ok(value) => value,
            Err(error) => {
                let callback_error = error.context(format!(
                    "entity {} ({}) {stage:?} callback node {}",
                    task.entity_id, task.archetype_name, task.node
                ));
                self.frame_vm_evaluations += output.evaluations as u64;
                self.frame_spawn_requests += output.spawn_queue.len() as u64;
                for (name, count) in output.function_counts {
                    *self.frame_function_counts.entry(name).or_default() += count;
                }
                return Err(callback_error);
            }
        };

        let commit_start = self.profiling.then(std::time::Instant::now);
        let requests = output.spawn_queue;
        self.frame_vm_evaluations += output.evaluations as u64;
        self.frame_spawn_requests += requests.len() as u64;
        if self.entities[task.entity_id].has_entity_data && stage == LifecycleStage::Preprocess {
            self.entities[task.entity_id].entity_data = self.entity_data_row(task.entity_id);
        }
        let (global_memory, entity_memory) = output.memory.into_watch_entity_parts(
            ENTITY_MEMORY,
            &[ENTITY_DATA, ENTITY_SHARED_MEMORY, TEMPORARY_MEMORY],
        );
        self.global_memory = global_memory;
        self.entities[task.entity_id].memory = entity_memory;
        self.callback_log.push(CallbackRecord {
            entity_id: Some(task.entity_id),
            archetype: task.archetype_name,
            stage,
            node: task.node,
        });
        self.frame_display_list
            .sprites
            .extend(output.display_list.sprites);
        self.frame_scheduled_effects
            .extend(output.scheduled_effects);
        self.frame_scheduled_looped_effects
            .extend(output.scheduled_looped_effects);
        self.frame_scheduled_looped_effect_stops
            .extend(output.scheduled_looped_effect_stops);
        self.frame_audio_events.extend(output.audio_events);
        self.frame_destroyed_particle_effects
            .extend(output.destroyed_particle_effects);
        self.frame_particle_events.extend(output.particle_events);
        self.frame_debug_events.extend(output.debug_events);
        for (name, count) in output.function_counts {
            *self.frame_function_counts.entry(name).or_default() += count;
        }
        self.frame_skin_checks.extend(output.skin_checks);
        for request in requests {
            self.enqueue_spawn(request.archetype_id, request.data)?;
        }
        if stage == LifecycleStage::Initialize {
            self.entities[task.entity_id].initialized = true;
        }
        if let Some(start) = commit_start {
            self.frame_profile.callback_commit += start.elapsed();
        }
        Ok(result)
    }

    fn invoke_entity(&mut self, id: usize, stage: LifecycleStage) -> Result<f64> {
        self.export_control.check()?;
        let archetype_index = self
            .entities
            .get(id)
            .context("entity id out of range")?
            .archetype_index;
        let archetype = &self.archetype_defs[archetype_index];
        let Some(value) = callback_ref(archetype, stage).cloned() else {
            if stage == LifecycleStage::Initialize {
                self.entities[id].initialized = true;
            }
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
        if self.profiling {
            self.frame_profile.callbacks += 1;
        }
        let phase_start = self.profiling.then(std::time::Instant::now);
        let mut vm = WatchVm::new(&self.watch.nodes);
        vm.set_profiling(self.profiling);
        vm.set_accounting(self.collect_vm_accounting);
        vm.set_sono_gcc_program(self.sono_gcc_program.clone());
        if let Some(start) = phase_start {
            self.frame_profile.callback_vm_construction += start.elapsed();
        }
        vm.set_evaluation_limit(callback_evaluation_limit(self.level_entity_count));
        let phase_start = self.profiling.then(std::time::Instant::now);
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
        if let Some(target) = self
            .diagnostic_target
            .filter(|target| target.entity_id == id && target.stage == stage)
        {
            vm.enable_limit_diagnostics(target.event_capacity)?;
        }
        if let Some(start) = phase_start {
            self.frame_profile.callback_context_setup += start.elapsed();
        }
        let phase_start = self.profiling.then(std::time::Instant::now);
        vm.memory = Memory::callback_view(&self.global_memory, &self.entities[id].memory);
        vm.memory.retain_other_than(TEMPORARY_MEMORY);
        if let Some(start) = phase_start {
            self.frame_profile.callback_memory_setup += start.elapsed();
        }
        let phase_start = self.profiling.then(std::time::Instant::now);
        let execution = vm.execute(node);
        if self.profiling {
            self.frame_profile.memory_entries_copied += vm.memory.copied_entries();
        }
        self.frame_profile.sono_gcc_regions_executed += vm.sono_gcc_regions_executed();
        self.frame_profile.sono_gcc_vm_to_native_cut_calls += vm.sono_gcc_vm_to_native_cut_calls();
        self.frame_profile.sono_gcc_overflow_input_regions += vm.sono_gcc_overflow_input_regions();
        let (dispatch, cuts, native) = vm.sono_gcc_profile_times();
        self.frame_profile.sono_gcc_region_dispatch += dispatch;
        self.frame_profile.sono_gcc_vm_cut_evaluation += cuts;
        self.frame_profile.sono_gcc_native_execution += native;
        if self.profiling {
            if let Some(start) = phase_start {
                self.frame_profile.callback_execute += start.elapsed();
            }
            self.frame_profile.evaluations += vm.evaluation_count() as u64;
            self.frame_profile.function_dispatches += vm.profile_function_dispatches;
        }
        let result = match execution {
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
        let commit_start = self.profiling.then(std::time::Instant::now);
        let requests = std::mem::take(&mut vm.spawn_queue);
        self.frame_vm_evaluations += vm.evaluation_count() as u64;
        self.frame_spawn_requests += requests.len() as u64;
        if self.entities[id].has_entity_data && stage == LifecycleStage::Preprocess {
            self.entities[id].entity_data = self.entity_data_row(id);
        }
        let (global_memory, entity_memory) = vm.memory.into_watch_entity_parts(
            ENTITY_MEMORY,
            &[ENTITY_DATA, ENTITY_SHARED_MEMORY, TEMPORARY_MEMORY],
        );
        self.global_memory = global_memory;
        // Entity Memory is the only per-entity persistent block. The split
        // keeps the next callback from reapplying this entity's global snapshot.
        self.entities[id].memory = entity_memory;
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
        if stage == LifecycleStage::Initialize {
            self.entities[id].initialized = true;
        }
        if let Some(start) = commit_start {
            self.frame_profile.callback_commit += start.elapsed();
        }
        Ok(result)
    }

    fn invoke_global(&mut self, stage: LifecycleStage, node: usize) -> Result<f64> {
        self.export_control.check()?;
        if self.profiling {
            self.frame_profile.callbacks += 1;
        }
        let phase_start = self.profiling.then(std::time::Instant::now);
        let mut vm = WatchVm::new(&self.watch.nodes);
        vm.set_profiling(self.profiling);
        vm.set_accounting(self.collect_vm_accounting);
        vm.set_sono_gcc_program(self.sono_gcc_program.clone());
        if let Some(start) = phase_start {
            self.frame_profile.callback_vm_construction += start.elapsed();
        }
        vm.set_evaluation_limit(callback_evaluation_limit(self.level_entity_count));
        let phase_start = self.profiling.then(std::time::Instant::now);
        vm.context = self.context.clone();
        vm.context.callback_name = Some(format!("{stage:?}"));
        vm.context.callback_node = Some(node);
        vm.set_draw_tracing(self.trace_draws);
        if let Some(start) = phase_start {
            self.frame_profile.callback_context_setup += start.elapsed();
        }
        let phase_start = self.profiling.then(std::time::Instant::now);
        vm.memory = Memory::fork(&self.global_memory);
        vm.memory.retain_other_than(TEMPORARY_MEMORY);
        if let Some(start) = phase_start {
            self.frame_profile.callback_memory_setup += start.elapsed();
        }
        let phase_start = self.profiling.then(std::time::Instant::now);
        let execution = vm.execute(node);
        if self.profiling {
            self.frame_profile.memory_entries_copied += vm.memory.copied_entries();
        }
        self.frame_profile.sono_gcc_regions_executed += vm.sono_gcc_regions_executed();
        self.frame_profile.sono_gcc_vm_to_native_cut_calls += vm.sono_gcc_vm_to_native_cut_calls();
        self.frame_profile.sono_gcc_overflow_input_regions += vm.sono_gcc_overflow_input_regions();
        let (dispatch, cuts, native) = vm.sono_gcc_profile_times();
        self.frame_profile.sono_gcc_region_dispatch += dispatch;
        self.frame_profile.sono_gcc_vm_cut_evaluation += cuts;
        self.frame_profile.sono_gcc_native_execution += native;
        if self.profiling {
            if let Some(start) = phase_start {
                self.frame_profile.callback_execute += start.elapsed();
            }
            self.frame_profile.evaluations += vm.evaluation_count() as u64;
            self.frame_profile.function_dispatches += vm.profile_function_dispatches;
        }
        let result = execution.with_context(|| format!("global {stage:?} callback node {node}"))?;
        let commit_start = self.profiling.then(std::time::Instant::now);
        let requests = std::mem::take(&mut vm.spawn_queue);
        self.frame_vm_evaluations += vm.evaluation_count() as u64;
        self.frame_spawn_requests += requests.len() as u64;
        self.global_memory = vm.memory.into_global_memory(&[TEMPORARY_MEMORY]);
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
        if let Some(start) = commit_start {
            self.frame_profile.callback_commit += start.elapsed();
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

fn callback_ref(archetype: &WatchArchetype, stage: LifecycleStage) -> Option<&Value> {
    match stage {
        LifecycleStage::Preprocess => archetype.preprocess.as_ref(),
        LifecycleStage::SpawnTime => archetype.spawn_time.as_ref(),
        LifecycleStage::DespawnTime => archetype.despawn_time.as_ref(),
        LifecycleStage::Initialize => archetype.initialize.as_ref(),
        LifecycleStage::UpdateSequential => archetype.update_sequential.as_ref(),
        LifecycleStage::UpdateParallel => archetype.update_parallel.as_ref(),
        LifecycleStage::Terminate => archetype.terminate.as_ref(),
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

fn callback_order(callback: Option<&Value>) -> i64 {
    callback
        .and_then(|v| v.get("order"))
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

fn in_spawn_range(entity: &WatchEntity, timeline: f64) -> bool {
    entity
        .schedule
        .is_some_and(|(start, end)| timeline >= start && timeline < end)
}

#[cfg(test)]
mod ui_configuration_tests {
    use super::*;

    fn fixture(callback: usize, nodes: Value, entities: usize) -> (WatchData, LevelData) {
        let watch = serde_json::from_value(serde_json::json!({
            "archetypes":[{"name":"Worker","updateParallel":{"index":callback}}],
            "nodes":nodes
        }))
        .unwrap();
        let level = serde_json::from_value(serde_json::json!({
            "entities":(0..entities).map(|_| serde_json::json!({
                "archetype":"Worker","data":[]
            })).collect::<Vec<_>>()
        }))
        .unwrap();
        (watch, level)
    }

    #[test]
    fn callback_evaluation_budget_scales_with_level_size_and_has_a_ceiling() {
        assert_eq!(callback_evaluation_limit(0), MIN_CALLBACK_EVALUATIONS);
        assert_eq!(callback_evaluation_limit(1_131), MIN_CALLBACK_EVALUATIONS);
        assert_eq!(callback_evaluation_limit(2_231), 9_138_176);
        assert_eq!(
            callback_evaluation_limit(usize::MAX),
            MAX_CALLBACK_EVALUATIONS
        );
    }

    #[test]
    fn independent_preprocess_callbacks_run_in_parallel_and_commit_in_entity_order() {
        let watch: WatchData = serde_json::from_value(serde_json::json!({
            "archetypes":[{"name":"Worker","preprocess":{"index":5}}],
            "nodes":[
                {"value":4000}, {"value":1}, {"value":4003}, {"value":0},
                {"func":"Get","args":[2,3]},
                {"func":"Set","args":[0,1,4]}
            ]
        }))
        .unwrap();
        let level: LevelData = serde_json::from_value(serde_json::json!({
            "entities":(0..256).map(|_| serde_json::json!({
                "archetype":"Worker","data":[]
            })).collect::<Vec<_>>()
        }))
        .unwrap();

        let mut serial = WatchRuntime::new(&watch, &level).unwrap();
        serial.set_parallel_updates(false, None).unwrap();
        serial.preprocess().unwrap();
        assert!(serial.parallel_workers.is_none());

        let mut parallel = WatchRuntime::new(&watch, &level).unwrap();
        parallel.set_parallel_updates(true, Some(4)).unwrap();
        parallel.set_profiling(true);
        parallel.preprocess().unwrap();

        assert_eq!(parallel.callback_log, serial.callback_log);
        assert_eq!(
            parallel.global_memory.get(4000, 1),
            serial.global_memory.get(4000, 1)
        );
        for id in 0..256 {
            assert_eq!(parallel.entities[id].memory.get(4000, 1), id as f64);
            assert_eq!(
                parallel.entities[id].memory.get(4000, 1),
                serial.entities[id].memory.get(4000, 1)
            );
        }
        assert_eq!(parallel.frame_profile.parallel_callbacks, 256);
        assert_eq!(parallel.frame_profile.parallel_batches, 1);
        assert!(parallel.parallel_workers.is_some());
    }

    #[test]
    fn preprocess_host_array_writes_remain_ordered() {
        let watch: WatchData = serde_json::from_value(serde_json::json!({
            "archetypes":[{"name":"Worker","preprocess":{"index":3}}],
            "nodes":[
                {"value":4001}, {"value":0}, {"value":42},
                {"func":"Set","args":[0,1,2]}
            ]
        }))
        .unwrap();
        let level: LevelData = serde_json::from_value(serde_json::json!({
            "entities":[
                {"archetype":"Worker","data":[]},
                {"archetype":"Worker","data":[]},
                {"archetype":"Worker","data":[]}
            ]
        }))
        .unwrap();
        let mut runtime = WatchRuntime::new(&watch, &level).unwrap();
        runtime.set_profiling(true);
        runtime.preprocess().unwrap();

        assert!(runtime.parallel_workers.is_none());
        assert_eq!(runtime.frame_profile.parallel_callbacks, 0);
        let array = runtime.context.entity_data_array.read().unwrap();
        for entity_id in 0..3 {
            assert_eq!(array[entity_id * 32], 42.0);
            assert!(array[entity_id * 32 + 1..entity_id * 32 + 32]
                .iter()
                .all(|value| *value == 0.0));
        }
    }

    #[test]
    fn parallel_initialize_error_preserves_committed_prefix_and_discards_suffix() {
        let watch: WatchData = serde_json::from_value(serde_json::json!({
            "archetypes":[{"name":"Worker","initialize":{"index":7}}],
            "nodes":[
                {"value":4003}, {"value":0}, {"func":"Get","args":[0,1]},
                {"value":128}, {"func":"Less","args":[2,3]},
                {"value":1}, {"func":"IntentionalTestFailure","args":[]},
                {"func":"If","args":[4,5,6]}
            ]
        }))
        .unwrap();
        let level: LevelData = serde_json::from_value(serde_json::json!({
            "entities":(0..256).map(|_| serde_json::json!({
                "archetype":"Worker","data":[]
            })).collect::<Vec<_>>()
        }))
        .unwrap();

        let mut serial = WatchRuntime::new(&watch, &level).unwrap();
        serial.set_parallel_updates(false, None).unwrap();
        let serial_error = serial.frame(0.0).unwrap_err().to_string();

        let mut parallel = WatchRuntime::new(&watch, &level).unwrap();
        parallel.set_parallel_updates(true, Some(4)).unwrap();
        let parallel_error = parallel.frame(0.0).unwrap_err().to_string();

        assert!(serial_error.contains("entity 128 (Worker) Initialize callback node 7"));
        assert!(parallel_error.contains("entity 128 (Worker) Initialize callback node 7"));
        for id in 0..256 {
            assert_eq!(serial.entities[id].initialized, id < 128);
            assert_eq!(parallel.entities[id].initialized, id < 128);
        }
    }

    #[test]
    fn binds_five_engine_visibility_pairs_to_runtime_ui_block() {
        let watch: WatchData =
            serde_json::from_value(serde_json::json!({"archetypes":[],"nodes":[]})).unwrap();
        let level: LevelData = serde_json::from_value(serde_json::json!({"entities":[]})).unwrap();
        let mut runtime = WatchRuntime::new(&watch, &level).unwrap();
        runtime
            .bind_engine_ui_configuration(&serde_json::json!({"ui":{
                "menuVisibility":{"scale":0.5,"alpha":0.2},
                "judgmentVisibility":{"scale":0.6,"alpha":0.3},
                "comboVisibility":{"scale":0.7,"alpha":0.4},
                "primaryMetricVisibility":{"scale":0.8,"alpha":0.5},
                "secondaryMetricVisibility":{"scale":0.9,"alpha":0.6}
            }}))
            .unwrap();
        assert_eq!(
            (0..10)
                .map(|i| runtime.global_memory.get(RUNTIME_UI_CONFIGURATION, i))
                .collect::<Vec<_>>(),
            [0.5, 0.2, 0.6, 0.3, 0.7, 0.4, 0.8, 0.5, 0.9, 0.6]
        );
        assert!(runtime
            .bind_engine_ui_configuration(
                &serde_json::json!({"ui":{"menuVisibility":{"scale":"bad","alpha":1}}})
            )
            .is_err());
    }

    #[test]
    fn parallel_update_matches_ordered_results_and_single_thread_opt_out_stays_lazy() {
        let (watch, level) = fixture(
            5,
            serde_json::json!([
                {"value":7},
                {"value":1},
                {"func":"Play","args":[0,1]},
                {"value":3},
                {"func":"DebugLog","args":[3]},
                {"func":"Execute","args":[2,4]}
            ]),
            256,
        );
        let mut serial = WatchRuntime::new(&watch, &level).unwrap();
        serial.set_parallel_updates(false, None).unwrap();
        serial.set_profiling(true);
        let serial_report = serial.frame(0.0).unwrap();
        assert!(serial.parallel_workers.is_none());

        let mut parallel = WatchRuntime::new(&watch, &level).unwrap();
        parallel.set_parallel_updates(true, Some(3)).unwrap();
        parallel.set_profiling(true);
        let parallel_report = parallel.frame(0.0).unwrap();

        assert_eq!(parallel_report.callbacks, serial_report.callbacks);
        assert_eq!(parallel_report.audio_events, serial_report.audio_events);
        assert_eq!(parallel_report.debug_events, serial_report.debug_events);
        assert_eq!(
            parallel_report.function_counts,
            serial_report.function_counts
        );
        assert_eq!(parallel_report.vm_evaluations, serial_report.vm_evaluations);
        assert_eq!(parallel_report.display_list, serial_report.display_list);
        assert_eq!(
            parallel_report.runtime_update_after_callbacks,
            serial_report.runtime_update_after_callbacks
        );
        assert!(parallel_report.runtime_profile.parallel_callbacks >= 8);
        assert!(parallel_report.runtime_profile.parallel_batches > 0);
        assert!(parallel_report.runtime_profile.parallel_worker_busy > Duration::ZERO);
        assert!(parallel_report.runtime_profile.parallel_worker_slots <= 3);
    }

    #[test]
    fn dynamic_global_memory_writes_fall_back_to_ordered_execution() {
        let (watch, level) = fixture(
            3,
            serde_json::json!([
                {"value":2000},
                {"value":0},
                {"value":1},
                {"func":"Set","args":[0,1,2]}
            ]),
            256,
        );
        let mut serial = WatchRuntime::new(&watch, &level).unwrap();
        serial.set_parallel_updates(false, None).unwrap();
        let serial_report = serial.frame(0.0).unwrap();

        let mut parallel = WatchRuntime::new(&watch, &level).unwrap();
        parallel.set_parallel_updates(true, Some(3)).unwrap();
        parallel.set_profiling(true);
        let parallel_report = parallel.frame(0.0).unwrap();

        assert_eq!(
            parallel.global_memory.get(2000, 0),
            serial.global_memory.get(2000, 0)
        );
        assert_eq!(parallel_report.callbacks, serial_report.callbacks);
        assert_eq!(
            parallel_report.function_counts,
            serial_report.function_counts
        );
        assert_eq!(parallel_report.vm_evaluations, serial_report.vm_evaluations);
        assert_eq!(parallel_report.display_list, serial_report.display_list);
        assert!(
            parallel_report
                .runtime_profile
                .parallel_speculative_callbacks
                > 0
        );
        assert_eq!(
            parallel_report.runtime_profile.parallel_ordered_callbacks,
            256
        );
    }

    #[test]
    fn shared_loop_ids_are_assigned_in_callback_order_without_a_worker_pool() {
        let (watch, level) = fixture(
            1,
            serde_json::json!([
                {"value":7},
                {"func":"PlayLooped","args":[0]}
            ]),
            5,
        );
        let mut runtime = WatchRuntime::new(&watch, &level).unwrap();
        runtime.set_profiling(true);
        let report = runtime.frame(0.0).unwrap();
        let ids: Vec<_> = report
            .audio_events
            .iter()
            .filter_map(|event| match event {
                AudioEffectEvent::StartLoop { instance_id, .. } => Some(*instance_id),
                _ => None,
            })
            .collect();
        assert_eq!(ids, vec![0, 1, 2, 3, 4]);
        assert_eq!(report.runtime_profile.parallel_batches, 0);
        assert!(runtime.parallel_workers.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn deferred_gcc_selection_uses_vm_when_background_program_is_not_ready() {
        let mut watch: WatchData = serde_json::from_value(serde_json::json!({
            "archetypes":[],
            "nodes":[{"value":1},{"value":2},{"func":"Add","args":[0,1]}]
        }))
        .unwrap();
        Arc::make_mut(&mut watch.nodes).push(
            serde_json::from_value(serde_json::json!({
                "value": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos() as f64
            }))
            .unwrap(),
        );
        let level: LevelData = serde_json::from_value(serde_json::json!({"entities":[]})).unwrap();
        let mut runtime = WatchRuntime::new(&watch, &level).unwrap();
        runtime.set_execution_mode_with_compile_policy(WatchExecutionMode::SonoGcc, false);
        assert!(runtime.sono_gcc_program.is_none());
        assert!(runtime.sono_gcc_precompile_deferred);
        assert!(runtime
            .execution_mode_status()
            .contains("precompile pending; using Sono VM"));

        crate::sono_gcc::precompile_with_progress(&watch.nodes, || {}).unwrap();
        runtime.set_execution_mode_with_compile_policy(WatchExecutionMode::SonoGcc, false);
        assert!(runtime.sono_gcc_program.is_some());
        assert!(!runtime.sono_gcc_precompile_deferred);
        assert_eq!(
            runtime.sono_gcc_cache_hit,
            crate::sono_gcc::CacheHit::Process
        );
    }
}
