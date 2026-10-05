use renderer::{runtime::WatchVm, watch::EngineNode};
use serde_json::json;

#[test]
fn sonolus_pre_post_updates_return_the_named_state_for_all_address_forms() {
    for action in ["Increment", "Decrement"] {
        for state in ["Pre", "Post"] {
            for form in ["", "Shifted", "Pointed"] {
                let operation = format!("{action}{state}{form}");
                let args = match form {
                    "" => vec![0, 1],
                    "Shifted" => vec![0, 2, 3, 4], // 2 + 3*4 = 14
                    _ => vec![0, 5, 2],            // pointer pair [10000,12], offset 2
                };
                let nodes: Vec<EngineNode> = serde_json::from_value(json!([
                    {"value":10000}, {"value":14}, {"value":2},
                    {"value":3}, {"value":4}, {"value":0},
                    {"func":operation,"args":args}
                ]))
                .unwrap();
                let mut vm = WatchVm::new(&nodes);
                vm.memory.set(10000, 0, 10000.0);
                vm.memory.set(10000, 1, 12.0);
                vm.memory.set(10000, 14, 5.0);
                let updated = if action == "Increment" { 6.0 } else { 4.0 };
                assert_eq!(
                    vm.execute(6).unwrap(),
                    if state == "Pre" { 5.0 } else { updated },
                    "{operation}"
                );
                assert_eq!(vm.memory.get(10000, 14), updated, "{operation}");
            }
        }
    }
}

#[test]
fn post_increment_run_counter_does_not_consume_an_extra_merge_item() {
    // The scheduler's counter starts at zero and compares the updated count
    // with run width one. A false test must skip the extra-item branch.
    let nodes: Vec<EngineNode> = serde_json::from_value(json!([
        {"value":10000}, {"value":4}, {"value":1},
        {"func":"IncrementPost","args":[0,1]},
        {"func":"Less","args":[3,2]},
        {"value":1}, {"value":2},
        {"func":"If","args":[4,5,6]},
        {"value":99}, {"value":42},
        {"func":"JumpLoop","args":[7,8,9]}
    ]))
    .unwrap();
    let mut vm = WatchVm::new(&nodes);
    assert_eq!(vm.execute(10).unwrap(), 42.0);
    assert_eq!(vm.memory.get(10000, 4), 1.0);
}

#[test]
fn canonical_next_rush_normal_holds_schedule_three_matched_intervals() {
    use renderer::{formats, watch_runtime::WatchRuntime};
    use std::path::Path;
    let dir = Path::new("TestingSuite/Next Sekai Engine/levels/larp 64x");
    if !dir.join("Next RUSH.zip").is_file() {
        eprintln!("canonical local fixture unavailable; primitive regressions still run");
        return;
    }
    for (file, hash) in [
        ("Next RUSH.zip", "0e53198c71b2739def8ce6aeee16d0fb4ae22e01"),
        (
            "larp 64x.json.gz",
            "3ef55933866663dfa52f3725eb1003c3a8bb0cb8",
        ),
    ] {
        use sha1::{Digest, Sha1};
        assert_eq!(
            format!("{:x}", Sha1::digest(std::fs::read(dir.join(file)).unwrap())),
            hash,
            "canonical fixture changed: {file}"
        );
    }
    let engine = formats::load_engine(&dir.join("Next RUSH.zip")).unwrap();
    let level = formats::load_level(&dir.join("larp 64x.json.gz")).unwrap();
    let resources = dir.join("ProSeka Faithful 0.8.4.scp");
    let mut rt = WatchRuntime::new(&engine.watch, &level).unwrap();
    rt.bind_engine_rom(&engine.rom).unwrap();
    rt.bind_engine_option_defaults(&engine.configuration)
        .unwrap();
    rt.bind_engine_option_overrides(&engine.configuration, &[(24, 1.0)])
        .unwrap();
    rt.bind_engine_ui_configuration(&engine.configuration)
        .unwrap();
    rt.bind_effect_clip_names(
        &formats::load_effect_clip_names(&resources, "ProSekaOfficial").unwrap(),
    )
    .unwrap();
    rt.bind_skin_sprite_names(
        &formats::load_skin_sprite_names(&resources, "ProSekaFaithful").unwrap(),
    )
    .unwrap();
    rt.bind_particle_effect_names(
        &formats::load_particle_effect_names(&resources, "NexintWaterMark").unwrap(),
    )
    .unwrap();
    let report = rt.frame(0.0).unwrap();
    assert_eq!(report.scheduled_effects.len(), 93);
    assert_eq!(report.scheduled_looped_effects.len(), 3);
    assert_eq!(report.scheduled_looped_effect_stops.len(), 3);
    for (i, (start, stop)) in report
        .scheduled_looped_effects
        .iter()
        .zip(&report.scheduled_looped_effect_stops)
        .enumerate()
    {
        assert_eq!(start.clip_id, 7);
        assert_eq!(start.instance_id, stop.instance_id);
        assert!((start.start_time - (12.0 + 3.0 * i as f64) / 7.0).abs() < 1e-10);
        assert!((stop.end_time - (15.0 + 3.0 * i as f64) / 7.0).abs() < 1e-10);
    }
    // Scheduling happens once in preprocess, not on every rendered frame.
    let next = rt.frame(1.8).unwrap();
    assert!(next.scheduled_looped_effects.is_empty());
    assert!(next.scheduled_looped_effect_stops.is_empty());
}

#[test]
fn switch_matches_test_values_and_evaluates_only_selected_consequent() {
    let nodes: Vec<EngineNode> = serde_json::from_value(json!([
        {"value":2.5}, {"value":7}, {"value":10000}, {"value":0},
        {"value":41}, {"value":42},
        {"func":"Set","args":[2,3,4]},
        {"func":"Set","args":[2,3,5]},
        {"func":"Switch","args":[0,1,6,0,7]},
        {"func":"Switch","args":[1,0,6]},
        {"func":"Switch","args":[1]},
        {"func":"Switch","args":[1,1,7,1,6]}
    ]))
    .unwrap();
    let mut vm = WatchVm::new(&nodes);
    assert_eq!(vm.execute(8).unwrap(), 42.0);
    assert_eq!(vm.memory.get(10000, 0), 42.0);
    assert_eq!(vm.execute(9).unwrap(), 0.0);
    assert_eq!(vm.execute(10).unwrap(), 0.0);
    assert_eq!(vm.execute(11).unwrap(), 42.0);
    assert_eq!(vm.memory.get(10000, 0), 42.0);
}

#[test]
fn or_returns_first_nonzero_value_and_short_circuits_side_effects() {
    let nodes: Vec<EngineNode> = serde_json::from_value(json!([
        {"value":0}, {"value":-2.5}, {"value":10000},
        {"value":77}, {"func":"Set","args":[2,0,3]},
        {"func":"Or","args":[0,1,4]},
        {"func":"Or","args":[0,0]},
        {"func":"Or","args":[0,4,1]}
    ]))
    .unwrap();
    let mut vm = WatchVm::new(&nodes);
    assert_eq!(vm.execute(5).unwrap(), -2.5);
    assert_eq!(vm.memory.get(10000, 0), 0.0);
    assert_eq!(vm.execute(6).unwrap(), 0.0);
    assert_eq!(vm.execute(7).unwrap(), 77.0);
    assert_eq!(vm.memory.get(10000, 0), 77.0);
}

#[test]
fn jump_loop_routes_nested_branches_preserves_memory_and_stops_at_last() {
    let nodes: Vec<EngineNode> = serde_json::from_value(json!([
        {"value":10000}, {"value":0}, {"value":1}, {"value":2}, {"value":3},
        {"value":99}, {"func":"Set","args":[0,1,3]},
        {"func":"Execute","args":[6,3]},
        {"func":"Get","args":[0,1]},
        {"func":"IncrementPost","args":[0,1]},
        {"func":"JumpLoop","args":[2,9]},
        {"func":"Execute","args":[10,4]},
        {"func":"JumpLoop","args":[7,5,11,8]},
        {"value":-1}, {"func":"JumpLoop","args":[13,5]},
        {"value":200}, {"func":"JumpLoop","args":[15,5]},
        {"func":"JumpLoop","args":[]}
    ]))
    .unwrap();
    let mut vm = WatchVm::new(&nodes);
    assert_eq!(vm.execute(12).unwrap(), 3.0);
    assert_eq!(vm.memory.get(10000, 0), 3.0);
    assert_eq!(vm.execute(14).unwrap(), 0.0);
    assert_eq!(vm.execute(16).unwrap(), 0.0);
    assert_eq!(vm.execute(17).unwrap(), 0.0);
}

#[test]
fn execution_trace_is_bounded_filters_writes_and_resolves_stops() {
    use renderer::watch_diagnostics::{TraceConfig, TraceFilter, WatchTrace};
    let nodes: Vec<EngineNode> = serde_json::from_value(json!([
        {"value":7}, {"value":2}, {"value":3},
        {"func":"PlayLoopedScheduled","args":[0,1]},
        {"func":"StopLoopedScheduled","args":[3,2]},
        {"value":10000}, {"value":0}, {"func":"Set","args":[5,6,1]}
    ]))
    .unwrap();
    let trace = WatchTrace::new(TraceConfig {
        capacity: 2,
        filters: vec![TraceFilter {
            kinds: ["audio".to_owned(), "memory_write".to_owned()].into(),
            ..Default::default()
        }],
    })
    .unwrap();
    trace.lock().unwrap().effect_names.insert(7, "#HOLD".into());
    let mut vm = WatchVm::new(&nodes);
    vm.context.execution_trace = Some(trace.clone());
    vm.execute(4).unwrap();
    vm.execute(7).unwrap();
    let file = tempfile::NamedTempFile::new().unwrap();
    trace.lock().unwrap().write_jsonl(file.path()).unwrap();
    let events: Vec<serde_json::Value> = std::fs::read_to_string(file.path())
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(events[1]["data"]["effect_name"], "#HOLD");
    assert_eq!(events[2]["data"]["effect_name"], "#HOLD");
    assert_eq!(events.last().unwrap()["recorded"], 2);
    assert_eq!(events.last().unwrap()["dropped"], 1);
    assert_eq!(vm.memory.get(10000, 0), 2.0);
}

#[test]
fn particle_trace_retains_filtered_spawn_origin_and_selected_particle_identity() {
    use renderer::{
        runtime::{ParticleEffectInstance, VmContext},
        watch_diagnostics::{TraceConfig, TraceFilter, WatchTrace},
    };
    let trace = WatchTrace::new(TraceConfig {
        capacity: 2,
        filters: vec![TraceFilter {
            handles: [42].into(),
            particles: [(1, 2, 3)].into(),
            entities: [9].into(),
            callbacks: ["Terminate".to_owned()].into(),
            spawn_min: Some(2.0),
            spawn_max: Some(2.0),
            ..Default::default()
        }],
    })
    .unwrap();
    let mut context = VmContext::default();
    context.time = 2.0;
    context.entity_id = Some(9);
    context.callback_name = Some("Terminate".into());
    context.callback_node = Some(55);
    let instance = ParticleEffectInstance {
        instance_id: 42,
        effect_id: 7,
        corners: [[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]],
        spawned_at: 2.0,
        duration: 1.0,
        is_looped: false,
    };
    let mut guard = trace.lock().unwrap();
    guard.particle_names.insert(7, "test".into());
    for handle in 100..104 {
        guard.record(
            &context,
            1,
            "SpawnParticleEffect",
            "particle_host",
            &[],
            json!({"event":{"instance_id":handle,"effect_id":7}}),
        );
    }
    guard.record(
        &context,
        66,
        "SpawnParticleEffect",
        "particle_host",
        &[],
        json!({"event":{"instance_id":42,"effect_id":7}}),
    );
    guard.record_particle(
        &instance,
        2.5,
        "ParticleEvaluation",
        json!({"particle":[1,2,3]}),
    );
    guard.record_particle(
        &instance,
        2.5,
        "ParticleEvaluation",
        json!({"particle":[1,2,4]}),
    );
    for _ in 0..2 {
        guard.record_particle(
            &instance,
            2.6,
            "ParticleEvaluation",
            json!({"particle":[1,2,3]}),
        );
    }
    let file = tempfile::NamedTempFile::new().unwrap();
    guard.write_jsonl(file.path()).unwrap();
    let events: Vec<serde_json::Value> = std::fs::read_to_string(file.path())
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(events[1]["entity"], 9);
    assert_eq!(events[1]["node"], 66);
    assert_eq!(events[1]["root"], 55);
    assert_eq!(events[1]["data"]["effect_name"], "test");
    assert_eq!(events[1]["data"]["spawned_at"], 2.0);
    assert_eq!(events.last().unwrap()["recorded"], 2);
    assert_eq!(events.last().unwrap()["dropped"], 1);
}

#[test]
fn particle_host_move_destroy_and_missing_id_preserve_handle_identity() {
    let nodes: Vec<EngineNode> = serde_json::from_value(json!([
        {"value":7},{"value":-1},{"value":1},{"value":0},{"value":2},{"value":99},
        {"func":"SpawnParticleEffect","args":[0,1,1,1,2,2,2,2,1,4,3]},
        {"func":"MoveParticleEffect","args":[3,3,3,3,2,2,2,2,3]},
        {"func":"DestroyParticleEffect","args":[3]},
        {"func":"SpawnParticleEffect","args":[1,1,1,1,2,2,2,2,1,4,3]},
        {"func":"MoveParticleEffect","args":[5,3,3,3,2,2,2,2,3]},
        {"func":"DestroyParticleEffect","args":[5]}
    ]))
    .unwrap();
    let mut vm = WatchVm::new(&nodes);
    vm.context.time = 5.0;
    assert_eq!(vm.execute(9).unwrap(), 0.0);
    assert!(vm
        .context
        .particle_instances
        .read()
        .unwrap()
        .instances()
        .is_empty());
    assert_eq!(vm.execute(6).unwrap(), 0.0);
    assert_eq!(vm.execute(7).unwrap(), 0.0);
    let instance = vm
        .context
        .particle_instances
        .read()
        .unwrap()
        .instances()
        .get(&0)
        .unwrap()
        .clone();
    assert_eq!(
        instance.corners,
        [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]]
    );
    assert_eq!(instance.spawned_at, 5.0);
    assert_eq!(instance.duration, 2.0);
    assert_eq!(vm.execute(10).unwrap(), 0.0);
    assert_eq!(vm.execute(11).unwrap(), 0.0);
    assert_eq!(
        vm.context
            .particle_instances
            .read()
            .unwrap()
            .instances()
            .len(),
        1
    );
    assert_eq!(vm.execute(8).unwrap(), 0.0);
    assert!(vm
        .context
        .particle_instances
        .read()
        .unwrap()
        .instances()
        .is_empty());
    assert_eq!(vm.execute(6).unwrap(), 1.0);
}
