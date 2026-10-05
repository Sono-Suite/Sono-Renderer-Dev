//! Opt-in bounded execution diagnostics, shared across the authoritative Watch traversal.
use crate::runtime::VmContext;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct TraceFilter {
    pub entities: BTreeSet<usize>,
    pub archetypes: BTreeSet<String>,
    pub callbacks: BTreeSet<String>,
    pub nodes: BTreeSet<usize>,
    pub operations: BTreeSet<String>,
    pub slots: BTreeSet<usize>,
    pub blocks: BTreeSet<i64>,
    pub addresses: BTreeSet<(i64, usize)>,
    pub kinds: BTreeSet<String>,
    pub handles: BTreeSet<i64>,
    pub effect_ids: BTreeSet<i64>,
    /// Particle identity within an effect: (group, copy, entry).
    pub particles: BTreeSet<(usize, u32, usize)>,
    pub spawn_min: Option<f64>,
    pub spawn_max: Option<f64>,
    pub time_min: Option<f64>,
    pub time_max: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct TraceConfig {
    pub capacity: usize,
    /// Filters are ORed; fields within a filter are ANDed. Empty means all events.
    pub filters: Vec<TraceFilter>,
}
impl Default for TraceConfig {
    fn default() -> Self {
        Self {
            capacity: 10000,
            filters: vec![],
        }
    }
}

#[derive(Debug)]
pub struct WatchTrace {
    pub config: TraceConfig,
    events: Vec<Value>,
    sequence: u64,
    dropped: u64,
    pub effect_names: BTreeMap<i64, String>,
    loop_clips: BTreeMap<i64, i64>,
    pub particle_names: BTreeMap<i64, String>,
    particle_origins: BTreeMap<i64, ParticleOrigin>,
}
#[derive(Debug, Clone)]
struct ParticleOrigin {
    entity: Option<usize>,
    archetype: Option<String>,
    callback: Option<String>,
    root: Option<usize>,
    node: usize,
    effect: i64,
    spawned_at: f64,
}
pub type SharedWatchTrace = Arc<Mutex<WatchTrace>>;
impl WatchTrace {
    pub fn new(config: TraceConfig) -> Result<SharedWatchTrace> {
        if config.capacity == 0 || config.capacity > 2_000_000 {
            bail!("trace capacity must be 1..=2000000");
        }
        Ok(Arc::new(Mutex::new(Self {
            config,
            events: vec![],
            sequence: 0,
            dropped: 0,
            effect_names: BTreeMap::new(),
            loop_clips: BTreeMap::new(),
            particle_names: BTreeMap::new(),
            particle_origins: BTreeMap::new(),
        })))
    }
    pub fn record(
        &mut self,
        context: &VmContext,
        node: usize,
        operation: &str,
        kind: &str,
        loops: &[(usize, usize, usize)],
        mut data: Value,
    ) {
        self.sequence += 1;
        if kind == "particle_host" {
            let event = data.get("event");
            let handle = event
                .and_then(|e| e.get("instance_id"))
                .and_then(Value::as_i64);
            let effect = event
                .and_then(|e| e.get("effect_id"))
                .and_then(Value::as_i64)
                .or_else(|| {
                    if operation != "SpawnParticleEffect" {
                        return None;
                    }
                    data.get("arguments")?
                        .as_array()?
                        .first()?
                        .as_str()?
                        .parse::<f64>()
                        .ok()
                        .map(|v| v as i64)
                });
            if let (Some(handle), Some(effect)) = (handle, effect) {
                // Remember an origin even when frame/particle filters hide its
                // host event. Unrelated instances must not exhaust this budget.
                let wanted = self.config.filters.is_empty()
                    || self.config.filters.iter().any(|f| {
                        (f.handles.is_empty() || f.handles.contains(&handle))
                            && (f.effect_ids.is_empty() || f.effect_ids.contains(&effect))
                            && (f.entities.is_empty()
                                || context.entity_id.is_some_and(|id| f.entities.contains(&id)))
                            && (f.archetypes.is_empty()
                                || context
                                    .entity_archetype
                                    .as_ref()
                                    .is_some_and(|name| f.archetypes.contains(name)))
                            && (f.callbacks.is_empty()
                                || context
                                    .callback_name
                                    .as_ref()
                                    .is_some_and(|name| f.callbacks.contains(name)))
                            && f.spawn_min.is_none_or(|v| context.time >= v)
                            && f.spawn_max.is_none_or(|v| context.time <= v)
                    });
                if wanted && self.particle_origins.len() < self.config.capacity {
                    self.particle_origins.insert(
                        handle,
                        ParticleOrigin {
                            entity: context.entity_id,
                            archetype: context.entity_archetype.clone(),
                            callback: context.callback_name.clone(),
                            root: context.callback_node,
                            node,
                            effect,
                            spawned_at: context.time,
                        },
                    );
                }
            }
            let origin = handle.and_then(|h| self.particle_origins.get(&h));
            data["handle"] = json!(handle);
            data["effect_id"] = json!(effect.or(origin.map(|o| o.effect)));
            data["spawned_at"] = json!(origin.map(|o| o.spawned_at));
            data["effect_name"] = json!(data["effect_id"]
                .as_i64()
                .and_then(|id| self.particle_names.get(&id)));
            if operation == "DestroyParticleEffect" {
                if let Some(handle) = handle {
                    self.particle_origins.remove(&handle);
                }
            }
        }
        // Resolve stops even when the matching start is excluded by filters.
        if kind == "audio" {
            if let Some(event) = data.get("event") {
                let handle = event.get("instance_id").and_then(Value::as_i64);
                let clip = event.get("clip_id").and_then(Value::as_i64);
                if let (Some(handle), Some(clip)) = (handle, clip) {
                    if self.loop_clips.len() < self.config.capacity
                        || self.loop_clips.contains_key(&handle)
                    {
                        self.loop_clips.insert(handle, clip);
                    }
                }
                let clip = clip.or_else(|| handle.and_then(|h| self.loop_clips.get(&h).copied()));
                if operation.starts_with("StopLooped") {
                    if let Some(handle) = handle {
                        self.loop_clips.remove(&handle);
                    }
                }
                if let Some(clip) = clip {
                    data["effect_id"] = json!(clip);
                    data["effect_name"] = json!(self.effect_names.get(&clip));
                }
            }
        }
        let matches = self.config.filters.is_empty()
            || self.config.filters.iter().any(|f| {
                (f.entities.is_empty()
                    || context.entity_id.is_some_and(|v| f.entities.contains(&v)))
                    && (f.archetypes.is_empty()
                        || context
                            .entity_archetype
                            .as_ref()
                            .is_some_and(|v| f.archetypes.contains(v)))
                    && (f.callbacks.is_empty()
                        || context
                            .callback_name
                            .as_ref()
                            .is_some_and(|v| f.callbacks.contains(v)))
                    && (f.nodes.is_empty() || f.nodes.contains(&node))
                    && (f.operations.is_empty() || f.operations.contains(operation))
                    && (f.kinds.is_empty() || f.kinds.contains(kind))
                    && (f.handles.is_empty()
                        || data
                            .get("handle")
                            .and_then(Value::as_i64)
                            .is_some_and(|h| f.handles.contains(&h)))
                    && (f.effect_ids.is_empty()
                        || data
                            .get("effect_id")
                            .and_then(Value::as_i64)
                            .is_some_and(|id| f.effect_ids.contains(&id)))
                    && (f.particles.is_empty()
                        || data
                            .get("particle")
                            .and_then(Value::as_array)
                            .filter(|p| p.len() == 3)
                            .is_some_and(|p| {
                                p[0].as_u64()
                                    .zip(p[1].as_u64())
                                    .zip(p[2].as_u64())
                                    .is_some_and(|((g, c), e)| {
                                        f.particles.contains(&(g as usize, c as u32, e as usize))
                                    })
                            }))
                    && f.spawn_min.is_none_or(|v| {
                        data.get("spawned_at")
                            .and_then(Value::as_f64)
                            .is_some_and(|s| s >= v)
                    })
                    && f.spawn_max.is_none_or(|v| {
                        data.get("spawned_at")
                            .and_then(Value::as_f64)
                            .is_some_and(|s| s <= v)
                    })
                    && (f.slots.is_empty()
                        || loops.iter().any(|(_, _, slot)| f.slots.contains(slot)))
                    && (f.blocks.is_empty()
                        || data
                            .get("block")
                            .and_then(Value::as_i64)
                            .is_some_and(|b| f.blocks.contains(&b)))
                    && (f.addresses.is_empty()
                        || data
                            .get("block")
                            .and_then(Value::as_i64)
                            .zip(data.get("index").and_then(Value::as_u64))
                            .is_some_and(|(b, i)| f.addresses.contains(&(b, i as usize))))
                    && f.time_min.is_none_or(|v| context.time >= v)
                    && f.time_max.is_none_or(|v| context.time <= v)
            });
        if !matches {
            return;
        }
        if self.events.len() == self.config.capacity {
            self.dropped += 1;
            return;
        }
        self.events
            .push(json!({"sequence": self.sequence, "time": context.time,
            "entity": context.entity_id, "archetype": context.entity_archetype,
            "callback": context.callback_name, "root": context.callback_node,
            "node":node, "operation":operation, "kind":kind, "loops":loops, "data":data}));
    }
    pub fn record_particle(
        &mut self,
        instance: &crate::runtime::ParticleEffectInstance,
        time: f64,
        operation: &str,
        mut data: Value,
    ) {
        let origin = self.particle_origins.get(&instance.instance_id).cloned();
        let mut context = VmContext::default();
        context.time = time;
        if let Some(origin) = &origin {
            context.entity_id = origin.entity;
            context.entity_archetype = origin.archetype.clone();
            context.callback_name = origin.callback.clone();
            context.callback_node = origin.root;
        }
        data["handle"] = json!(instance.instance_id);
        data["effect_id"] = json!(instance.effect_id);
        data["spawned_at"] = json!(instance.spawned_at);
        data["effect_name"] = json!(self.particle_names.get(&instance.effect_id));
        self.record(
            &context,
            origin.map(|o| o.node).unwrap_or(usize::MAX),
            operation,
            "particle",
            &[],
            data,
        );
    }
    pub fn write_jsonl(&self, path: &Path) -> Result<()> {
        let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
        serde_json::to_writer(
            &mut file,
            &json!({"kind":"trace_header", "config": self.config}),
        )?;
        writeln!(file)?;
        for event in &self.events {
            serde_json::to_writer(&mut file, event)?;
            writeln!(file)?;
        }
        serde_json::to_writer(
            &mut file,
            &json!({"kind":"trace_summary","recorded":self.events.len(),"dropped":self.dropped,"observations":self.sequence}),
        )?;
        writeln!(file)?;
        Ok(())
    }
}
