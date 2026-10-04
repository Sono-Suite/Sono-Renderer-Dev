//! Generic, explicitly inferred whole-chart boundary discovery.
//! Persistent level controllers have no provable end; finite schedules are
//! evaluated on the actual updateSpawn timeline, never treated as Watch seconds.
use crate::{formats::EffectAssets, offline::EventSession, runtime::AudioEffectEvent};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct ChartEnd {
    pub watch_end: f64,
    pub confidence: &'static str,
    pub policy: &'static str,
    pub finite_entity_count: usize,
    pub persistent_entity_count: usize,
    pub bgm_watch_end: Option<f64>,
    pub sfx_watch_end: f64,
    pub discovery_frames: u64,
    pub discovery_seconds: f64,
    pub discovery_callbacks: usize,
    /// Export-policy threshold, not a Sonolus semantic constant.
    pub persistent_lifetime_threshold: f64,
}

pub(crate) fn discover(
    mut session: EventSession<'_>,
    _fps: u32,
    bgm_end: Option<f64>,
    assets: &EffectAssets,
    bindings: &BTreeMap<i64, String>,
    _progress: Option<&crate::export_progress::ProgressTracker>,
    watch: &crate::watch::WatchData,
) -> Result<ChartEnd> {
    let started = std::time::Instant::now();
    // Conservatively prove that one-time evidence covers all schedules/audio.
    // Unknown graphs are indeterminate, never silently sampled at a lower FPS.
    prove_closed_schedule(watch)?;
    let (scale, offset) = spawn_timeline(watch).context(
        "whole-chart end is indeterminate: updateSpawn is not a supported positive affine Watch timeline; use --duration",
    )?;
    if !scale.is_finite() || scale <= 0.0 || !offset.is_finite() {
        bail!("whole-chart end is indeterminate: non-increasing/nonfinite spawn timeline; use --duration");
    }
    let report = session.schedule_evidence()?;
    let mut durations = BTreeMap::new();
    let mut sfx_end: f64 = 0.0;
    let mut loops = BTreeMap::new();
    for event in &report.audio_events {
        match *event {
            AudioEffectEvent::Play {
                clip_id,
                requested_at,
                ..
            } => {
                if let Some(duration) = clip_duration(clip_id, &mut durations, assets, bindings)? {
                    sfx_end = sfx_end.max(requested_at + duration);
                }
            }
            AudioEffectEvent::StartLoop {
                instance_id,
                clip_id,
                ..
            } => {
                if clip_duration(clip_id, &mut durations, assets, bindings)?.is_some() {
                    loops.insert(instance_id, true);
                }
            }
            AudioEffectEvent::StopLoop {
                instance_id,
                requested_at,
            } => {
                loops.remove(&instance_id);
                sfx_end = sfx_end.max(requested_at);
            }
        }
    }
    for event in &report.scheduled_effects {
        if let Some(duration) = clip_duration(event.clip_id, &mut durations, assets, bindings)? {
            sfx_end = sfx_end.max(event.time + duration);
        }
    }
    for event in &report.scheduled_looped_effects {
        if clip_duration(event.clip_id, &mut durations, assets, bindings)?.is_some() {
            loops.insert(event.instance_id, true);
        }
    }
    for event in &report.scheduled_looped_effect_stops {
        loops.remove(&event.instance_id);
        sfx_end = sfx_end.max(event.end_time);
    }
    let runtime = session.runtime();
    // Stable policy expressed in timeline seconds, independent of output FPS.
    let supported_span = crate::offline::MAX_EXPORT_FRAMES as f64 / 60.0;
    let mut finite = 0;
    let mut persistent = 0;
    let mut schedule_end: f64 = 0.0;
    for entity in &runtime.entities {
        let (start, end) = entity
            .schedule
            .context("whole-chart end is indeterminate: missing schedule; use --duration")?;
        let has_input = watch
            .archetypes
            .get(entity.archetype_index)
            .is_some_and(|a| a.has_input);
        if !has_input && end >= supported_span {
            persistent += 1;
            if start.is_finite() && start < end {
                schedule_end = schedule_end.max((start - offset) / scale);
            }
        } else if end.is_finite() {
            finite += 1;
            if end > start {
                schedule_end = schedule_end.max((end - offset) / scale);
            }
        } else if has_input {
            bail!(
                "whole-chart end is indeterminate: open-ended input entity {}; use --duration",
                entity.id
            );
        } else {
            persistent += 1;
        }
    }
    if finite == 0 && bgm_end.is_none() {
        bail!(
            "whole-chart end is indeterminate: no finite schedule or BGM boundary; use --duration"
        );
    }
    if !loops.is_empty() {
        bail!("whole-chart end is indeterminate: audible SFX loops remain open; use --duration");
    }
    // Preserve the former 60-Hz policy boundary, including the endpoint
    // lifecycle frame, without tying inference to the requested video FPS.
    let schedule_end = if schedule_end > 0.0 {
        (schedule_end * 60.0 - 1e-10).ceil() / 60.0 + 1.0 / 60.0
    } else {
        0.0
    };
    let watch_end =
        (schedule_end.max(bgm_end.unwrap_or(0.0)).max(sfx_end) * 60.0 - 1e-10).ceil() / 60.0;
    if !watch_end.is_finite() || watch_end <= 0.0 {
        bail!("whole-chart end is indeterminate: no positive finite boundary; use --duration");
    }
    Ok(ChartEnd {
        watch_end,
        confidence: "inferred",
        policy: "preprocessed finite schedules plus BGM and known SFX tails; 60-Hz boundary policy, positive affine spawn timeline, no post-preprocess Spawn/audio; persistent non-input activation included, finish not inferred from persistent endpoints",
        finite_entity_count: finite,
        persistent_entity_count: persistent,
        bgm_watch_end: bgm_end,
        sfx_watch_end: sfx_end,
        discovery_frames: 0,
        discovery_seconds: started.elapsed().as_secs_f64(),
        discovery_callbacks: report.callbacks.len(),
        persistent_lifetime_threshold: supported_span,
    })
}

// Traverse all branches, including unreachable branches. False negatives are
// acceptable: they produce an explicit indeterminate result, not a guessed end.
fn prove_closed_schedule(watch: &crate::watch::WatchData) -> Result<()> {
    let mut roots = Vec::new();
    roots.extend(watch.update_spawn);
    for a in &watch.archetypes {
        for callback in [
            &a.spawn_time,
            &a.despawn_time,
            &a.initialize,
            &a.update_sequential,
            &a.update_parallel,
            &a.terminate,
        ] {
            if let Some(value) = callback {
                roots.push(value.as_u64().or_else(|| value.get("index").and_then(serde_json::Value::as_u64))
                    .context("whole-chart end is indeterminate: unsupported callback root; use --duration")? as usize);
            }
        }
    }
    let mut visited = std::collections::BTreeSet::new();
    while let Some(index) = roots.pop() {
        if !visited.insert(index) {
            continue;
        }
        let node = watch
            .nodes
            .get(index)
            .context("whole-chart end is indeterminate: invalid graph node")?;
        if matches!(
            node.func.as_deref(),
            Some(
                "Spawn"
                    | "Play"
                    | "PlayScheduled"
                    | "PlayLooped"
                    | "PlayLoopedScheduled"
                    | "StopLooped"
                    | "StopLoopedScheduled"
            )
        ) {
            bail!("whole-chart end is indeterminate: post-preprocess {} at node {} can add entities/audio; use --duration", node.func.as_deref().unwrap(), index);
        }
        for arg in &node.args {
            roots.push(
                arg.as_u64()
                    .context("whole-chart end is indeterminate: unsupported graph argument")?
                    as usize,
            );
        }
    }
    Ok(())
}

/// Small symbolic analysis of a side-effect-free affine spawn clock. It does
/// not execute or skip any playback callbacks. Compiler return wrappers are
/// recognized only when an unconditional Break(1, value) is the first statement.
fn spawn_timeline(watch: &crate::watch::WatchData) -> Option<(f64, f64)> {
    fn literal(w: &crate::watch::WatchData, id: usize) -> Option<f64> {
        w.nodes.get(id)?.value.as_ref()?.as_f64()
    }
    fn args(w: &crate::watch::WatchData, id: usize) -> Option<Vec<usize>> {
        w.nodes
            .get(id)?
            .args
            .iter()
            .map(|v| v.as_u64().map(|i| i as usize))
            .collect()
    }
    fn returned(w: &crate::watch::WatchData, id: usize, depth: usize) -> Option<usize> {
        if depth > 256 {
            return None;
        }
        let node = w.nodes.get(id)?;
        let a = args(w, id)?;
        match node.func.as_deref()? {
            "Break" if a.len() == 2 && literal(w, a[0]) == Some(1.0) => Some(a[1]),
            "Execute" if !a.is_empty() => returned(w, a[0], depth + 1),
            "JumpLoop" if a.len() == 2 && literal(w, a[1]) == Some(0.0) => {
                returned(w, a[0], depth + 1)
            }
            _ => None,
        }
    }
    fn affine(w: &crate::watch::WatchData, id: usize, depth: usize) -> Option<(f64, f64)> {
        if depth > 256 {
            return None;
        }
        if let Some(value) = literal(w, id) {
            return Some((0.0, value));
        }
        let node = w.nodes.get(id)?;
        let a = args(w, id)?;
        match node.func.as_deref()? {
            "Block" if a.len() == 1 => affine(w, returned(w, a[0], depth + 1)?, depth + 1),
            "Get"
                if a.len() == 2
                    && literal(w, a[0]) == Some(1001.0)
                    && literal(w, a[1]) == Some(0.0) =>
            {
                Some((1.0, 0.0))
            }
            "Add" | "Subtract" if !a.is_empty() => {
                let mut out = affine(w, a[0], depth + 1)?;
                for arg in &a[1..] {
                    let value = affine(w, *arg, depth + 1)?;
                    let sign = if node.func.as_deref() == Some("Subtract") {
                        -1.0
                    } else {
                        1.0
                    };
                    out.0 += sign * value.0;
                    out.1 += sign * value.1;
                }
                Some(out)
            }
            "Multiply" if !a.is_empty() => {
                let mut out = (0.0, 1.0);
                for arg in a {
                    let v = affine(w, arg, depth + 1)?;
                    if out.0 != 0.0 && v.0 != 0.0 {
                        return None;
                    }
                    out = (out.0 * v.1 + v.0 * out.1, out.1 * v.1);
                }
                Some(out)
            }
            _ => None,
        }
    }
    watch
        .update_spawn
        .map_or(Some((1.0, 0.0)), |root| affine(watch, root, 0))
}

fn clip_duration(
    id: i64,
    durations: &mut BTreeMap<i64, Option<f64>>,
    assets: &EffectAssets,
    bindings: &BTreeMap<i64, String>,
) -> Result<Option<f64>> {
    if let Some(duration) = durations.get(&id) {
        return Ok(*duration);
    }
    let duration = bindings
        .get(&id)
        .and_then(|name| assets.clips.get(name))
        .and_then(Option::as_ref)
        .filter(|bytes| !bytes.is_empty())
        .map(|bytes| {
            crate::audio::clip_duration(bytes).with_context(|| {
                format!(
                    "reading SFX clip {:?} duration for whole-chart end inference",
                    bindings.get(&id)
                )
            })
        })
        .transpose()?;
    durations.insert(id, duration);
    Ok(duration)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_chart_schedule_evidence_matches_frame_zero_runtime_inputs() {
        let watch: crate::watch::WatchData = serde_json::from_value(serde_json::json!({
            "archetypes":[{"name":"Note","spawnTime":{"index":0},"despawnTime":{"index":3}}],
            "nodes":[{"value":0},{"value":1001},{"value":2},{"func":"Get","args":[1,2]}]
        }))
        .unwrap();
        let level = serde_json::from_value(
            serde_json::json!({"entities":[{"archetype":"Note","data":[]}]}),
        )
        .unwrap();
        let mut cheap = crate::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();
        let mut playback = crate::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();
        // Host-configured scaled-time anchor, deliberately distinct from zero.
        for runtime in [&mut cheap, &mut playback] {
            runtime.context.time_map = vec![(0.0, 3.0)];
            runtime.context.timescale_map = vec![(0.0, 2.0)];
        }
        cheap.export_schedule_evidence().unwrap();
        playback.frame(0.0).unwrap();
        assert_eq!(cheap.entities[0].schedule, playback.entities[0].schedule);
        assert_eq!(cheap.entities[0].schedule, Some((0.0, 3.0)));
        assert_eq!(cheap.context.timescale, playback.context.timescale);
    }

    #[test]
    fn whole_chart_rejects_dynamic_spawn_audio_and_unknown_clock_without_playback() {
        for function in ["Spawn", "Play", "PlayScheduled", "PlayLooped", "StopLooped"] {
            let watch = serde_json::from_value(serde_json::json!({
                "archetypes":[{"updateParallel":{"index":0}}],
                "nodes":[{"func":function,"args":[]}]
            }))
            .unwrap();
            assert!(prove_closed_schedule(&watch)
                .unwrap_err()
                .to_string()
                .contains("post-preprocess"));
        }
        let watch = serde_json::from_value(serde_json::json!({"updateSpawn":0,"nodes":[{"func":"Random","args":[1,2]},{"value":0},{"value":1}]})).unwrap();
        assert!(spawn_timeline(&watch).is_none());
    }

    #[test]
    fn whole_chart_baumkuchen_discovery_is_fps_independent_and_runs_no_playback_frames() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let package =
            crate::formats::load_engine(&repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"))
                .unwrap();
        let level = crate::formats::load_level(&repo.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz")).unwrap();
        let resources = repo.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp");
        let defaults: BTreeMap<_, _> = package.metadata.resource_defaults().into_iter().collect();
        let effects = defaults.get("effects").unwrap();
        let assets = crate::formats::load_effect_assets_optional(&resources, effects)
            .unwrap()
            .unwrap();
        let bindings = crate::offline::effect_clip_bindings(&package.watch).unwrap();
        let Some(tools) = crate::export_mv::tests::tools() else {
            return;
        };
        let bgm = repo.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/ANIMATION_VIDIO_Baumkuchen_and_Credits_X_Retry_Now_Sub_English_and_jangan_NAKISO_amaamala_1789700599.mp3");
        let bgm_end = crate::export_media::source_duration(&tools, &bgm).unwrap()
            - level.bgm_offset.unwrap_or(0.0);
        for fps in [12, 30, 60] {
            let session = EventSession::new_configured(
                &package.watch,
                &package.rom,
                &package.configuration,
                &level,
                &resources,
                defaults.get("skins").unwrap(),
                defaults.get("backgrounds").map(String::as_str),
                Some(effects),
                defaults.get("particles").map(String::as_str),
                1920,
                1080,
                fps,
                &[(1, 10.8)],
                true,
                false,
                false,
            )
            .unwrap();
            let end = discover(
                session,
                fps,
                Some(bgm_end),
                &assets,
                &bindings,
                None,
                &package.watch,
            )
            .unwrap();
            println!(
                "discovery {fps} FPS: {}",
                serde_json::to_string(&end).unwrap()
            );
            assert!((end.watch_end - 121.95).abs() < 1e-9);
            assert_eq!(end.confidence, "inferred");
            assert_eq!(end.discovery_frames, 0);
            assert_eq!(end.persistent_entity_count, 38);
            assert!(end.discovery_callbacks > 0);
        }
    }
}
