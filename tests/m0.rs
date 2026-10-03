use flate2::{write::GzEncoder, Compression};
use std::io::{Read, Write};

#[test]
fn watch_inventory_keeps_arbitrary_operation_names() {
    let raw = br#"{"archetypes":[{"name":"Unfamiliar","imports":[],"updateParallel":{"index":4}}],"nodes":[{"func":"AlienOperation","args":[0,1]},{"func":"NovelFn","args":[]}] }"#;
    let parsed: serde_json::Value = serde_json::from_slice(raw).unwrap();
    let watch: renderer::watch::WatchData = serde_json::from_value(parsed).unwrap();
    assert_eq!(watch.update_spawn, None);
    let inv = renderer::watch::inventory(&watch);
    assert_eq!(inv.archetypes, vec!["Unfamiliar"]);
    assert!(inv.functions.contains(&"AlienOperation".to_string()));
    assert!(inv.functions.contains(&"NovelFn".to_string()));
}

#[test]
fn z_tuple_order_matches_reported_progressive_sonolus_observations() {
    use renderer::formats::{SkinAssets, SkinSpriteAsset};
    use renderer::runtime::{compare_z_tuples, DisplayList, SpriteDraw};
    use std::cmp::Ordering;
    use std::collections::BTreeMap;

    let identity = [
        [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
    ];
    let skin = SkinAssets {
        width: 2,
        height: 1,
        interpolation: false,
        rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
        sprites: BTreeMap::from([
            (
                "red".to_owned(),
                SkinSpriteAsset {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                    transform: identity,
                },
            ),
            (
                "green".to_owned(),
                SkinSpriteAsset {
                    x: 1,
                    y: 0,
                    width: 1,
                    height: 1,
                    transform: identity,
                },
            ),
        ]),
    };
    let bindings = BTreeMap::from([(0, "red".to_owned()), (1, "green".to_owned())]);
    let quad = [[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]];
    let cases = [
        (
            "BasicOverlap",
            [0.0, 0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0, 0.0],
            [0, 255, 0],
        ),
        (
            "Discriminator1",
            [2.0, -100.0, 0.0, 0.0],
            [1.0, 100.0, 0.0, 0.0],
            [255, 0, 0],
        ),
        (
            "Discriminator2",
            [1.0, 5.0, 0.0, 0.0],
            [1.0, 4.0, 100.0, 0.0],
            [255, 0, 0],
        ),
        (
            "Discriminator3",
            [1.0, 4.0, 100.0, 0.0],
            [1.0, 5.0, 0.0, 0.0],
            [0, 255, 0],
        ),
        (
            "Component2A",
            [1.0, 5.0, 2.0, -100.0],
            [1.0, 5.0, 1.0, 100.0],
            [255, 0, 0],
        ),
        (
            "Component2B",
            [1.0, 5.0, 1.0, 100.0],
            [1.0, 5.0, 2.0, -100.0],
            [0, 255, 0],
        ),
        (
            "Component3A",
            [1.0, 5.0, 2.0, 10.0],
            [1.0, 5.0, 2.0, 9.0],
            [255, 0, 0],
        ),
        (
            "Component3B",
            [1.0, 5.0, 2.0, 9.0],
            [1.0, 5.0, 2.0, 10.0],
            [0, 255, 0],
        ),
    ];

    for (name, red_z, green_z, expected_top) in cases {
        assert_eq!(
            compare_z_tuples(&red_z, &green_z),
            red_z
                .iter()
                .zip(green_z)
                .find_map(|(red, green)| { (red != &green).then(|| red.total_cmp(&green)) })
                .unwrap_or(Ordering::Equal),
            "{name}: comparator should use the first differing component"
        );
        let display_list = DisplayList {
            sprites: vec![
                SpriteDraw {
                    sprite_id: 0,
                    corners: quad,
                    z: red_z,
                    alpha: 1.0,
                    provenance: None,
                    trace: None,
                },
                SpriteDraw {
                    sprite_id: 1,
                    corners: quad,
                    z: green_z,
                    alpha: 1.0,
                    provenance: None,
                    trace: None,
                },
            ],
        };
        let diagnostic_frame = display_list.render_ppm(3, 3).unwrap();
        let expected_sprite_id = if expected_top == [255, 0, 0] { 0 } else { 1 };
        let expected_diagnostic_frame = DisplayList {
            sprites: vec![display_list.sprites[expected_sprite_id].clone()],
        }
        .render_ppm(3, 3)
        .unwrap();
        assert_eq!(
            &diagnostic_frame[(4 * 3)..(5 * 3)],
            &expected_diagnostic_frame[(4 * 3)..(5 * 3)],
            "{name}: diagnostic color renderer must use the same tuple order"
        );
        let frame = display_list
            .render_skin_ppm(3, 3, 1.0, &skin, &bindings)
            .unwrap();
        assert_eq!(&frame[11 + (4 * 3)..11 + (5 * 3)], &expected_top, "{name}");

        let diagnostics = display_list
            .skin_render_diagnostics(3, 3, 1.0, &skin, &bindings)
            .unwrap();
        let expected_order = if expected_top == [255, 0, 0] {
            [1, 0]
        } else {
            [0, 1]
        };
        assert_eq!(
            [
                diagnostics[0].display_list_index,
                diagnostics[1].display_list_index
            ],
            expected_order,
            "{name}: diagnostics must agree with rendered painter order"
        );
        assert_eq!(
            diagnostics[0].z_order_key,
            display_list.sprites[expected_order[0]].z
        );
    }
}

#[test]
fn z_tuple_exact_ties_preserve_stable_draw_submission_order() {
    use renderer::formats::{SkinAssets, SkinSpriteAsset};
    use renderer::runtime::{compare_z_tuples, DisplayList, SpriteDraw};
    use std::cmp::Ordering;
    use std::collections::BTreeMap;

    assert_eq!(
        compare_z_tuples(&[1.0, 2.0, 3.0, 4.0], &[1.0, 2.0, 3.0, 4.0]),
        Ordering::Equal
    );
    let identity = [
        [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
    ];
    let skin = SkinAssets {
        width: 2,
        height: 1,
        interpolation: false,
        rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
        sprites: BTreeMap::from([
            (
                "red".to_owned(),
                SkinSpriteAsset {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                    transform: identity,
                },
            ),
            (
                "green".to_owned(),
                SkinSpriteAsset {
                    x: 1,
                    y: 0,
                    width: 1,
                    height: 1,
                    transform: identity,
                },
            ),
        ]),
    };
    let display_list = DisplayList {
        sprites: [0, 1]
            .into_iter()
            .map(|sprite_id| SpriteDraw {
                sprite_id,
                corners: [[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]],
                z: [1.0, 2.0, 3.0, 4.0],
                alpha: 1.0,
                provenance: None,
                trace: None,
            })
            .collect(),
    };
    let frame = display_list
        .render_skin_ppm(
            3,
            3,
            1.0,
            &skin,
            &BTreeMap::from([(0, "red".to_owned()), (1, "green".to_owned())]),
        )
        .unwrap();
    assert_eq!(&frame[11 + (4 * 3)..11 + (5 * 3)], &[0, 255, 0]);
    let diagnostics = display_list
        .skin_render_diagnostics(
            3,
            3,
            1.0,
            &skin,
            &BTreeMap::from([(0, "red".to_owned()), (1, "green".to_owned())]),
        )
        .unwrap();
    assert_eq!(
        [
            diagnostics[0].display_list_index,
            diagnostics[1].display_list_index
        ],
        [0, 1]
    );
}

#[test]
fn watch_inventory_resolves_memory_block_ids_from_get_nodes() {
    let raw = br#"{"nodes":[{"value":1000},{"value":0},{"func":"Get","args":[0,1]}]}"#;
    let watch: renderer::watch::WatchData = serde_json::from_slice(raw).unwrap();
    assert_eq!(
        renderer::watch::inventory(&watch).memory_blocks,
        vec!["1000"]
    );
}

#[test]
fn watch_data_deserializes_sonolus_update_spawn_camel_case() {
    let watch: renderer::watch::WatchData =
        serde_json::from_slice(br#"{"updateSpawn":7}"#).unwrap();
    assert_eq!(watch.update_spawn, Some(7));
    assert!(serde_json::from_slice::<renderer::watch::WatchData>(
        br#"{"updateSpawn":{"index":7}}"#
    )
    .is_err());
}

#[test]
fn watch_archetype_deserializes_real_lifecycle_field_names() {
    let archetype: renderer::watch::WatchArchetype = serde_json::from_value(serde_json::json!({
        "name": "Example",
        "hasInput": true,
        "spawnTime": {"index": 11, "order": 2},
        "despawnTime": {"index": 12},
        "updateSequential": {"index": 13, "order": -1},
        "updateParallel": {"index": 14},
        "imports": [{"name": "field", "index": 3, "def": 9}]
    }))
    .unwrap();
    assert!(archetype.has_input);
    assert_eq!(archetype.spawn_time.unwrap()["index"], 11);
    assert_eq!(archetype.despawn_time.unwrap()["index"], 12);
    assert_eq!(archetype.update_sequential.unwrap()["order"], -1);
    assert_eq!(archetype.update_parallel.unwrap()["index"], 14);
    assert_eq!(archetype.imports[0]["def"], 9);
}

#[test]
fn engine_resource_defaults_accept_item_and_legacy_metadata() {
    let metadata: renderer::formats::EngineMetadata = serde_json::from_value(serde_json::json!({
        "version": 13,
        "skin": {"name":"skin-a"},
        "background_name":"background-b",
        "effect": {"name":"effect-c"},
        "particle": {"name":"particle-d"}
    }))
    .unwrap();
    let defaults = metadata.resource_defaults();
    assert_eq!(defaults.len(), 4);
    assert!(defaults.contains(&("skins".into(), "skin-a".into())));
    assert!(defaults.contains(&("backgrounds".into(), "background-b".into())));
}

#[test]
fn engine_rom_is_optional_for_zip_and_directory_packages() {
    let dir = tempfile::tempdir().unwrap();
    let zip_path = dir.path().join("engine.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    for (name, contents) in [
        ("engine.json", br#"{"version":13}"#.as_slice()),
        ("EngineConfiguration", br#"{"options":[]}"#.as_slice()),
        ("EngineWatchData", br#"{"nodes":[{"value":1}]}"#.as_slice()),
    ] {
        zip.start_file(name, options).unwrap();
        zip.write_all(contents).unwrap();
    }
    zip.finish().unwrap();
    assert!(renderer::formats::load_engine(&zip_path)
        .unwrap()
        .rom
        .is_empty());

    let package_dir = dir.path().join("directory-engine");
    std::fs::create_dir(&package_dir).unwrap();
    std::fs::write(package_dir.join("engine.json"), br#"{"version":13}"#).unwrap();
    std::fs::write(
        package_dir.join("EngineConfiguration"),
        br#"{"options":[]}"#,
    )
    .unwrap();
    std::fs::write(
        package_dir.join("EngineWatchData"),
        br#"{"nodes":[{"value":1}]}"#,
    )
    .unwrap();
    assert!(renderer::formats::load_engine(&package_dir)
        .unwrap()
        .rom
        .is_empty());
}

#[test]
fn supplied_reference_fixtures_load() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let engine = repo.join("Next Sekai Engine.zip");
    let level = repo.join("Exorcist.zip");
    let scp = repo.join("ProSeka Faithful 0.8.3.scp");
    assert!(
        engine.is_file(),
        "missing engine fixture: {}",
        engine.display()
    );
    assert!(
        level.is_file(),
        "missing level fixture: {}",
        level.display()
    );
    assert!(scp.is_file(), "missing SCP fixture: {}", scp.display());
    let pkg = renderer::formats::load_engine(&engine).unwrap();
    assert!(!pkg.watch.nodes.is_empty());
    assert!(!pkg.rom.is_empty());
    assert!(pkg.watch.update_spawn.is_some());
    assert_eq!(pkg.watch.update_spawn, Some(162902));
    let level_data = renderer::formats::load_level(&level).unwrap();
    assert_eq!(level_data.bgm_offset, Some(0.083));
    assert!(!level_data.entities.is_empty());

    let mut level_archive = zip::ZipArchive::new(std::fs::File::open(&level).unwrap()).unwrap();
    let mut level_gzip = None;
    for index in 0..level_archive.len() {
        let mut entry = level_archive.by_index(index).unwrap();
        if entry.name().ends_with("level.data") {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            level_gzip = Some(bytes);
            break;
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let standalone = dir.path().join("level.json.gz");
    std::fs::write(&standalone, level_gzip.expect("fixture level.data member")).unwrap();
    let standalone_level = renderer::formats::load_level_data(&standalone).unwrap();
    assert_eq!(standalone_level.bgm_offset, Some(0.083));
    assert_eq!(standalone_level.entities.len(), 4130);
    assert!(standalone_level.metadata.is_none());

    assert!(!renderer::formats::inspect_scp(&scp)
        .unwrap()
        .resources
        .is_empty());
    let report = renderer::compatibility::analyze(&engine, &scp, &level).unwrap();
    assert!(report.level_entity_count > 0);
    assert!(!report.level_archetypes.is_empty());
    assert!(report
        .engine
        .support
        .vm_runtime
        .contains("partial Watch VM"));
    assert!(report.resource_integrity_issues.is_empty());
}

#[test]
fn supplied_skin_asset_loader_verifies_and_decodes_atlas() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let skin = renderer::formats::load_skin_assets(
        &repo.join("ProSeka Faithful 0.8.3.scp"),
        "ProSekaFaithful",
    )
    .unwrap();
    assert_eq!((skin.width, skin.height), (4096, 4096));
    assert_eq!(skin.sprites.len(), 316);
    assert_eq!(skin.rgba.len(), 4096 * 4096 * 4);
    assert!(skin.sprites.contains_key("Sekai Stage Lane"));
}

#[test]
fn supplied_scp_resolves_next_sekai_effect_and_particle_names() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let scp = repo.join("ProSeka Faithful 0.8.3.scp");
    let clips = renderer::formats::load_effect_clip_names(&scp, "sekai_q").unwrap();
    let particles = renderer::formats::load_particle_effect_names(&scp, "NexintWaterMark").unwrap();
    let effect_assets = renderer::formats::load_effect_assets(&scp, "sekai_q").unwrap();
    let particle_assets = renderer::formats::load_particle_assets(&scp, "NexintWaterMark").unwrap();
    assert!(clips.contains("#PERFECT"));
    assert!(clips.contains("Sekai Trace"));
    assert!(particles.contains("#NOTE_LINEAR_TAP_GREEN"));
    assert!(particles.contains("Sekai Note Lane Linear"));
    assert!(effect_assets.clips["#PERFECT"]
        .as_ref()
        .unwrap()
        .starts_with(b"RIFF"));
    let effect_bindings = [(3, "#PERFECT".to_owned())].into();
    let mixed = renderer::audio::mix_effects_wav(
        &effect_assets,
        &effect_bindings,
        &[renderer::runtime::AudioEffectEvent::Play {
            clip_id: 3,
            minimum_distance: 0.0,
            requested_at: 0.0,
        }],
        &[],
        &[],
        &[],
        0.0,
        0.1,
    )
    .unwrap()
    .wav
    .unwrap();
    assert_eq!(&mixed[..4], b"RIFF");
    assert_eq!(particle_assets.effects.len(), 22);
    assert!(!particle_assets.rgba.is_empty());
    let effect = particle_assets.effects.values().next().unwrap();
    let particle = effect
        .groups
        .iter()
        .flat_map(|group| &group.particles)
        .find(|particle| particle.duration > 0.0)
        .unwrap();
    let instance = renderer::runtime::ParticleEffectInstance {
        instance_id: 0,
        effect_id: 91,
        corners: [[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]],
        duration: 1.0,
        is_looped: false,
        spawned_at: 0.0,
    };
    let bindings = [(91, effect.name.clone())].into();
    let particle_time = particle.start + particle.duration * 0.5;
    let draws = renderer::particles::render_instances(
        &particle_assets,
        &bindings,
        &[instance],
        particle_time,
    )
    .unwrap();
    assert!(
        !draws.is_empty(),
        "real ParticleData effect should produce a live sprite"
    );
    let mut frame = vec![0; 32 * 32 * 3];
    renderer::runtime::DisplayList::composite_particle_sprites(
        &mut frame,
        32,
        32,
        1.0,
        &particle_assets,
        &draws,
        &[
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ],
    )
    .unwrap();
    assert!(
        frame.iter().any(|byte| *byte != 0),
        "real particle sprite should affect the RGB frame"
    );
}

#[test]
fn effect_data_missing_clip_payload_is_optional_but_malformed_data_fails() {
    use sha1::Digest;

    fn make_scp(path: &std::path::Path, effect_data: &[u8]) {
        let hash = hex::encode(sha1::Sha1::digest(effect_data));
        let manifest = serde_json::json!({
            "item": {"data": {"hash": hash}}
        });
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("sonolus/effects/demo", options).unwrap();
        zip.write_all(manifest.to_string().as_bytes()).unwrap();
        zip.start_file(format!("sonolus/repository/{hash}"), options)
            .unwrap();
        zip.write_all(effect_data).unwrap();
        zip.finish().unwrap();
    }

    let dir = tempfile::tempdir().unwrap();
    let no_payload_path = dir.path().join("no-payload.scp");
    make_scp(
        &no_payload_path,
        br#"{"clips":[{"name":"Sekai Skill","filename":"sekai-skill.wav"}]}"#,
    );
    let assets = renderer::formats::load_effect_assets_optional(&no_payload_path, "demo")
        .unwrap()
        .unwrap();
    assert!(assets.clips["Sekai Skill"].is_none());
    let mixed = renderer::audio::mix_effects_wav(
        &assets,
        &[(8, "Sekai Skill".to_owned())].into(),
        &[renderer::runtime::AudioEffectEvent::Play {
            clip_id: 8,
            minimum_distance: 0.0,
            requested_at: 0.0,
        }],
        &[],
        &[],
        &[],
        0.0,
        0.1,
    )
    .unwrap();
    assert!(mixed.wav.is_none());
    assert_eq!(mixed.warnings.len(), 1);
    assert!(mixed.warnings[0].contains("Sekai Skill"));

    let malformed_path = dir.path().join("malformed.scp");
    make_scp(&malformed_path, br#"{"notClips":[]}"#);
    let error =
        renderer::formats::load_effect_assets_optional(&malformed_path, "demo").unwrap_err();
    assert!(error.to_string().contains("EffectData has no clips array"));
}

#[test]
fn watch_resource_binding_preserves_partial_has_effect_and_particle_availability() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "skin":{"sprites":[]},
        "effect":{"clips":[{"name":"present clip","id":11},{"name":"missing clip","id":12}]},
        "particle":{"effects":[{"name":"present particle","id":21},{"name":"missing particle","id":22}]},
        "archetypes":[],"nodes":[]
    })).unwrap();
    let level: renderer::formats::LevelData =
        serde_json::from_value(serde_json::json!({"entities":[]})).unwrap();
    let mut runtime = renderer::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();
    runtime
        .bind_effect_clip_names(&["present clip".to_owned()].into())
        .unwrap();
    runtime
        .bind_particle_effect_names(&["present particle".to_owned()].into())
        .unwrap();

    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value":11}, {"func":"HasEffectClip","args":[0]},
        {"value":12}, {"func":"HasEffectClip","args":[2]},
        {"value":21}, {"func":"HasParticleEffect","args":[4]},
        {"value":22}, {"func":"HasParticleEffect","args":[6]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    vm.context = runtime.context.clone();
    assert_eq!(vm.execute(1).unwrap(), 1.0);
    assert_eq!(vm.execute(3).unwrap(), 0.0);
    assert_eq!(vm.execute(5).unwrap(), 1.0);
    assert_eq!(vm.execute(7).unwrap(), 0.0);
}

#[test]
fn scp_resource_parsing_does_not_depend_on_filename_extension() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(repo.join("ProSeka Faithful 0.8.3.scp")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let scp_path = dir.path().join("resources.scp");
    let zip_path = dir.path().join("resources.zip");
    std::fs::write(&scp_path, &bytes).unwrap();
    std::fs::write(&zip_path, &bytes).unwrap();

    assert_eq!(
        serde_json::to_value(renderer::formats::inspect_scp(&scp_path).unwrap()).unwrap(),
        serde_json::to_value(renderer::formats::inspect_scp(&zip_path).unwrap()).unwrap()
    );
    assert_eq!(
        renderer::formats::load_skin_sprite_names(&scp_path, "ProSekaFaithful").unwrap(),
        renderer::formats::load_skin_sprite_names(&zip_path, "ProSekaFaithful").unwrap()
    );
}

#[test]
fn supplied_fixture_trace_reaches_a_real_entity_preprocess_callback() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let engine = renderer::formats::load_engine(&repo.join("Next Sekai Engine.zip")).unwrap();
    let level = renderer::formats::load_level(&repo.join("Exorcist.zip")).unwrap();
    let entity_id = level
        .entities
        .iter()
        .position(|entity| {
            let Some(name) = entity.archetype.as_str() else {
                return false;
            };
            engine
                .watch
                .archetypes
                .iter()
                .find(|archetype| archetype.name == name)
                .is_some_and(|archetype| archetype.preprocess.is_some())
        })
        .unwrap();
    let source = &level.entities[entity_id];
    let archetype_name = source.archetype.as_str().unwrap();
    let archetype = engine
        .watch
        .archetypes
        .iter()
        .find(|archetype| archetype.name == archetype_name)
        .unwrap();
    let mut runtime = renderer::watch_runtime::WatchRuntime::new(&engine.watch, &level).unwrap();
    let entity_names: std::collections::BTreeMap<_, _> = level
        .entities
        .iter()
        .enumerate()
        .filter_map(|(index, entity)| {
            entity
                .extra
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(|name| (name, index))
        })
        .collect();
    let mut resolved_references = 0;
    for (source_id, source_entity) in level.entities.iter().enumerate() {
        let source_name = source_entity.archetype.as_str().unwrap();
        let source_archetype = engine
            .watch
            .archetypes
            .iter()
            .find(|archetype| archetype.name == source_name)
            .unwrap();
        for field in source_entity.data.as_array().unwrap() {
            let (Some(name), Some(target)) = (
                field.get("name").and_then(serde_json::Value::as_str),
                field.get("ref").and_then(serde_json::Value::as_str),
            ) else {
                continue;
            };
            let Some(import) = source_archetype.imports.iter().find(|import| {
                import.get("name").and_then(serde_json::Value::as_str) == Some(name)
            }) else {
                continue;
            };
            let slot = import["index"].as_u64().unwrap() as usize;
            let target_id = *entity_names.get(target).unwrap();
            assert_eq!(
                runtime.entities[source_id].entity_data[slot],
                target_id as f64
            );
            resolved_references += 1;
        }
    }
    assert!(
        resolved_references > 0,
        "fixture should exercise named entity references"
    );
    runtime.bind_engine_rom(&engine.rom).unwrap();
    runtime
        .bind_engine_option_defaults(&engine.configuration)
        .unwrap();
    runtime
        .bind_engine_ui_configuration(&engine.configuration)
        .unwrap();
    assert_eq!(runtime.global_memory.get(1007, 0), 1.0);
    assert_eq!(runtime.global_memory.get(1007, 1), 1.0);
    let resources = repo.join("ProSeka Faithful 0.8.3.scp");
    let skin_name = engine.metadata.skin_name.as_deref().unwrap();
    let skin = renderer::formats::load_skin_assets(&resources, skin_name).unwrap();
    let sprite_names = skin.sprites.keys().cloned().collect();
    runtime.bind_skin_sprite_names(&sprite_names).unwrap();
    let effect_name = engine.metadata.effect_name.as_deref().unwrap();
    let effect_names = renderer::formats::load_effect_clip_names(&resources, effect_name).unwrap();
    runtime.bind_effect_clip_names(&effect_names).unwrap();
    let particle_name = engine.metadata.particle_name.as_deref().unwrap();
    let particle_names =
        renderer::formats::load_particle_effect_names(&resources, particle_name).unwrap();
    runtime.bind_particle_effect_names(&particle_names).unwrap();
    let background_name = engine.metadata.background_name.as_deref().unwrap();
    let background =
        renderer::formats::load_background_assets(&resources, background_name).unwrap();
    let background_quad = background
        .data
        .runtime_quad(
            1280.0 / 720.0,
            f64::from(background.width) / f64::from(background.height),
        )
        .unwrap();
    runtime
        .set_runtime_background_quad(background_quad)
        .unwrap();
    assert!(runtime.context.skin_sprites.contains(&70));
    assert!(runtime.context.skin_sprites.contains(&258));
    assert!(runtime.context.effect_clips.contains(&0));
    assert!(!runtime.context.particle_effects.is_empty());
    let resource_check_nodes: Vec<renderer::watch::EngineNode> =
        serde_json::from_value(serde_json::json!([
            {"value": 70},
            {"func": "HasSkinSprite", "args": [0]}
        ]))
        .unwrap();
    let mut resource_check = renderer::runtime::WatchVm::new(&resource_check_nodes);
    resource_check.context = runtime.context.clone();
    assert_eq!(resource_check.execute(1).unwrap(), 1.0);
    runtime
        .bind_engine_option_defaults(&engine.configuration)
        .unwrap();
    let report = runtime.frame(0.0).unwrap();

    for import in &archetype.imports {
        let name = import["name"].as_str().unwrap();
        let slot = import["index"].as_u64().unwrap() as usize;
        if let Some(value) = source
            .data
            .as_array()
            .unwrap()
            .iter()
            .find(|value| value["name"] == name)
        {
            if let Some(number) = value["value"].as_f64() {
                assert_eq!(runtime.entities[entity_id].entity_data[slot], number);
            }
        }
    }
    let preprocess = archetype.preprocess.as_ref().unwrap()["index"]
        .as_u64()
        .unwrap() as usize;
    assert!(report.callbacks.iter().any(|callback| {
        callback.entity_id == Some(entity_id)
            && callback.archetype == archetype_name
            && callback.stage == renderer::watch_runtime::LifecycleStage::Preprocess
            && callback.node == preprocess
    }));
    assert!(runtime.entities[entity_id].has_entity_data);
    assert_eq!(runtime.entities[entity_id].id, entity_id);
    eprintln!(
        "real trace: level entity {entity_id} archetype={archetype_name}, source data={:?}, import defs={:?}, imported row={:?}, preprocess node={preprocess}, runtime id={}, callbacks={}; resolved named references={resolved_references}",
        source.data,
        archetype.imports,
        runtime.entities[entity_id].entity_data,
        runtime.entities[entity_id].id,
        report.callbacks.len()
    );
}

#[test]
fn fixture_import_mutation_changes_only_the_target_entity_slot() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let engine = renderer::formats::load_engine(&repo.join("Next Sekai Engine.zip")).unwrap();
    let level = renderer::formats::load_level(&repo.join("Exorcist.zip")).unwrap();
    let mut candidate = None;
    'entities: for left in 0..level.entities.len() {
        let Some(archetype_name) = level.entities[left].archetype.as_str() else {
            continue;
        };
        let Some(archetype) = engine
            .watch
            .archetypes
            .iter()
            .find(|archetype| archetype.name == archetype_name)
        else {
            continue;
        };
        let value_for = |entity_id: usize, import_name: &str| {
            level.entities[entity_id]
                .data
                .as_array()?
                .iter()
                .find(|entry| {
                    entry.get("name").and_then(serde_json::Value::as_str) == Some(import_name)
                })?
                .get("value")
                .and_then(serde_json::Value::as_f64)
        };
        for right in (left + 1)..level.entities.len() {
            if level.entities[right].archetype.as_str() != Some(archetype_name) {
                continue;
            }
            for import in &archetype.imports {
                let Some(name) = import.get("name").and_then(serde_json::Value::as_str) else {
                    continue;
                };
                let (Some(left_value), Some(right_value)) =
                    (value_for(left, name), value_for(right, name))
                else {
                    continue;
                };
                if left_value != right_value {
                    candidate = Some((
                        left,
                        right,
                        name.to_owned(),
                        import["index"].as_u64().unwrap() as usize,
                        left_value,
                        right_value,
                    ));
                    break 'entities;
                }
            }
        }
    }
    let (left, right, field_name, slot, left_value, right_value) = candidate
        .expect("fixture has two same-archetype entities with different imported numeric values");
    let original = renderer::watch_runtime::WatchRuntime::new(&engine.watch, &level).unwrap();
    assert_eq!(original.entities[left].entity_data[slot], left_value);
    assert_eq!(original.entities[right].entity_data[slot], right_value);

    let mut changed_level = level.clone();
    let field = changed_level.entities[left]
        .data
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry.get("name").and_then(serde_json::Value::as_str) == Some(&field_name))
        .unwrap();
    field["value"] = serde_json::json!(left_value + 123.0);
    let changed =
        renderer::watch_runtime::WatchRuntime::new(&engine.watch, &changed_level).unwrap();
    assert_eq!(changed.entities[left].entity_data[slot], left_value + 123.0);
    assert_eq!(changed.entities[right].entity_data[slot], right_value);
}

#[test]
fn gzip_magic_decodes_json_payload() {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(br#"{"bgmOffset":0,"entities":[]}"#)
        .unwrap();
    let compressed = encoder.finish().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("level.data"), compressed).unwrap();
    let level = renderer::formats::load_level(dir.path()).unwrap();
    assert_eq!(level.bgm_offset, Some(0.0));
}

#[test]
fn direct_gz_level_data_ignores_legacy_item_metadata() {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(br#"{"bgmOffset":1.25,"entities":[]}"#)
        .unwrap();
    let compressed = encoder.finish().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let level_path = dir.path().join("level.json.gz");
    std::fs::write(&level_path, compressed).unwrap();
    std::fs::write(dir.path().join("item.json"), "not json").unwrap();

    let level = renderer::formats::load_level_data(&level_path).unwrap();
    assert_eq!(level.bgm_offset, Some(1.25));
    assert!(level.metadata.is_none());
}

#[test]
fn project_with_accessible_separate_inputs_is_valid_without_cover() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let engine = repo.join("Next Sekai Engine.zip");
    let resources = repo.join("ProSeka Faithful 0.8.3.scp");
    assert!(
        engine.is_file(),
        "missing engine fixture: {}",
        engine.display()
    );
    assert!(
        resources.is_file(),
        "missing SCP fixture: {}",
        resources.display()
    );

    let dir = tempfile::tempdir().unwrap();
    let level_path = dir.path().join("level.json.gz");
    let level_json = serde_json::to_vec(&serde_json::json!({
        "bgmOffset": 0,
        "entities": []
    }))
    .unwrap();
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&level_json).unwrap();
    std::fs::write(&level_path, encoder.finish().unwrap()).unwrap();
    let music = dir.path().join("music.mp3");
    std::fs::write(&music, b"fixture audio input").unwrap();

    let project = renderer::project::ProjectInputs {
        engine_package: engine,
        resource_package: resources,
        level_data: level_path,
        music,
        cover: None,
        custom_mv: None,
    }
    .validate()
    .unwrap();
    assert!(project.cover.is_none());
    assert!(project.custom_mv.is_none());
    assert!(project.level.entities.is_empty());
}

#[test]
fn absent_optional_project_inputs_serialize_as_absent() {
    let inputs = renderer::project::ProjectInputs {
        engine_package: "engine.zip".into(),
        resource_package: "resources.scp".into(),
        level_data: "level.json.gz".into(),
        music: "music.mp3".into(),
        cover: None,
        custom_mv: None,
    };
    let value = serde_json::to_value(inputs).unwrap();
    assert!(value.get("cover").is_none());
    assert!(value.get("custom_mv").is_none());
}

#[test]
fn watch_vm_executes_control_memory_math_and_draw() {
    use renderer::watch::EngineNode as N;
    let nodes: Vec<N> = serde_json::from_value(serde_json::json!([
        {"value": 1000}, {"value": 2}, {"value": 3}, {"func":"Add","args":[1,2]},
        {"func":"Set","args":[0,1,3]}, {"func":"Get","args":[0,1]},
        {"value": 9}, {"func":"If","args":[1,5,6]},
        {"value": 7}, {"value": 0}, {"value": 1}, {"value": 2},
        {"value": 0}, {"value": 1}, {"value": 2}, {"value": 3},
        {"value": 4}, {"value": 5}, {"value": 6}, {"value": 7},
        {"value": 8}, {"value": 0.5},
        {"func":"Draw","args":[10,11,12,13,14,15,16,17,18,19,20]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert_eq!(vm.execute(4).unwrap(), 5.0);
    assert_eq!(vm.memory.get(1000, 2), 5.0);
    assert_eq!(vm.execute(7).unwrap(), 5.0);
    vm.set_draw_tracing(true);
    vm.execute(22).unwrap();
    assert_eq!(vm.display_list.sprites.len(), 1);
    let draw = &vm.display_list.sprites[0];
    assert_eq!(draw.sprite_id, 1);
    assert_eq!(draw.corners[0], [2.0, 0.0]);
    assert_eq!(draw.alpha, 8.0);
    assert_eq!(draw.provenance.as_ref().unwrap().draw_node, 22);
    let trace = draw.trace.as_ref().unwrap();
    assert_eq!(trace.argument_nodes, (10..=20).collect::<Vec<_>>());
    assert_eq!(trace.node_values[&10], 1.0);
    assert_eq!(trace.node_values[&20], 8.0);
}

#[test]
fn watch_subtract_folds_all_operands_left_to_right() {
    use renderer::watch::EngineNode as N;
    let nodes: Vec<N> = serde_json::from_value(serde_json::json!([
        {"value": 10}, {"value": 4}, {"value": 1},
        {"func":"Subtract","args":[0,1,2]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);

    assert_eq!(vm.execute(3).unwrap(), 5.0);
}

#[test]
fn watch_draw_negative_one_is_oracle_verified_noop_and_execution_continues() {
    use renderer::watch::EngineNode as N;
    let nodes: Vec<N> = serde_json::from_value(serde_json::json!([
        {"value":-1},
        {"value":-0.2}, {"value":-0.2}, {"value":-0.2}, {"value":0.2},
        {"value":0.2}, {"value":0.2}, {"value":0.2}, {"value":-0.2},
        {"value":0}, {"value":1},
        {"func":"Draw","args":[0,1,2,3,4,5,6,7,8,9,10]},
        {"value":1},
        {"value":0}, {"value":-0.2}, {"value":0.2}, {"value":-0.2},
        {"value":0.2}, {"value":0.2}, {"value":0.2}, {"value":-0.2},
        {"value":0}, {"value":1},
        {"func":"Draw","args":[12,13,14,15,16,17,18,19,20,21,22]},
        {"func":"Execute","args":[11,23]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);

    assert_eq!(vm.execute(24).unwrap(), 0.0);
    assert_eq!(vm.display_list.sprites.len(), 1);
    assert_eq!(vm.display_list.sprites[0].sprite_id, 1);
}

#[test]
fn watch_draw_negative_one_evaluates_other_arguments_before_noop() {
    use renderer::watch::EngineNode as N;
    let nodes: Vec<N> = serde_json::from_value(serde_json::json!([
        {"value":-1}, {"value":10000}, {"value":3}, {"value":42},
        {"func":"Set","args":[1,2,3]},
        {"value":0}, {"value":0}, {"value":0}, {"value":1},
        {"value":1}, {"value":1}, {"value":0}, {"value":0}, {"value":0},
        {"func":"Draw","args":[0,4,5,6,7,8,9,10,11,12,13]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);

    assert_eq!(vm.execute(14).unwrap(), 0.0);
    assert_eq!(vm.memory.get(10000, 3), 42.0);
    assert!(vm.display_list.sprites.is_empty());
}

#[test]
fn watch_draw_negative_two_keeps_existing_rejection() {
    use renderer::watch::EngineNode as N;
    let nodes: Vec<N> = serde_json::from_value(serde_json::json!([
        {"value":-2}, {"value":-0.2}, {"value":-0.2}, {"value":-0.2},
        {"value":0.2}, {"value":0.2}, {"value":0.2}, {"value":0.2},
        {"value":-0.2}, {"value":0}, {"value":1},
        {"func":"Draw","args":[0,1,2,3,4,5,6,7,8,9,10]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);

    let error = vm.execute(11).unwrap_err();
    assert!(format!("{error:#}").contains("sprite id is outside u32 range"));
    assert!(vm.display_list.sprites.is_empty());
}

#[test]
fn watch_pointed_memory_uses_in_block_pointer_pair_and_offset() {
    use renderer::watch::EngineNode as N;
    let nodes: Vec<N> = serde_json::from_value(serde_json::json!([
        {"value":10000}, {"value":0}, {"value":2},
        {"func":"GetPointed","args":[0,1,2]},
        {"value":5}, {"func":"SetAddPointed","args":[0,1,2,4]},
        {"func":"GetPointed","args":[0,1,2]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    vm.memory.set(10000, 0, 2000.0);
    vm.memory.set(10000, 1, 4.0);
    vm.memory.set(2000, 6, 77.0);

    assert_eq!(vm.execute(3).unwrap(), 77.0);
    assert_eq!(vm.execute(5).unwrap(), 82.0);
    assert_eq!(vm.memory.get(2000, 6), 82.0);
    assert_eq!(vm.execute(6).unwrap(), 82.0);
}

#[test]
fn watch_shifted_and_pointed_memory_variants_apply_documented_updates() {
    use renderer::watch::EngineNode as N;
    let nodes: Vec<N> = serde_json::from_value(serde_json::json!([
        {"value":10000}, {"value":1}, {"value":2}, {"value":3}, {"value":4},
        {"func":"SetModShifted","args":[0,1,2,3,4]},
        {"value":10000}, {"value":0}, {"value":0}, {"value":3},
        {"func":"IncrementPrePointed","args":[6,7,8]},
        {"value":10000}, {"value":2}, {"value":3}, {"value":1},
        {"func":"DecrementPostShifted","args":[11,12,13,14]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    vm.memory.set(10000, 7, 17.0); // shifted slot x + y*s = 1 + 2*3
    vm.memory.set(10000, 0, 10000.0); // pointed target block
    vm.memory.set(10000, 1, 8.0); // pointed target index
    vm.memory.set(10000, 8, 5.0); // pointed target before increment
    vm.memory.set(10000, 5, 5.0); // shifted decrement slot x + y*s = 2 + 3*1

    assert_eq!(vm.execute(5).unwrap(), 1.0); // 17 mod 4
    assert_eq!(vm.memory.get(10000, 7), 1.0);
    assert_eq!(vm.execute(10).unwrap(), 6.0);
    assert_eq!(vm.memory.get(10000, 8), 6.0);
    assert_eq!(vm.execute(15).unwrap(), 5.0); // post-decrement returns old
    assert_eq!(vm.memory.get(10000, 5), 4.0);
}

#[test]
fn watch_do_while_runs_body_before_test_and_returns_zero() {
    use renderer::watch::EngineNode as N;
    let nodes: Vec<N> = serde_json::from_value(serde_json::json!([
        {"value":10000}, {"value":0}, {"value":1},
        {"func":"SetAdd","args":[0,1,2]},
        {"value":10000}, {"value":0}, {"func":"Get","args":[4,5]},
        {"value":3}, {"func":"Less","args":[6,7]},
        {"func":"DoWhile","args":[3,8]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);

    assert_eq!(vm.execute(9).unwrap(), 0.0);
    assert_eq!(vm.memory.get(10000, 0), 3.0);
}

#[test]
fn watch_stream_queries_use_keyed_entries_and_interpolation() {
    use renderer::watch::EngineNode as N;
    let nodes: Vec<N> = serde_json::from_value(serde_json::json!([
        {"value": 7}, {"value": 0.5}, {"func":"StreamHas","args":[0,1]},
        {"value": 7}, {"value": 1}, {"func":"StreamHas","args":[3,4]},
        {"value": 7}, {"value": 2}, {"func":"StreamGetValue","args":[6,7]},
        {"value": 7}, {"value": 1}, {"func":"StreamGetNextKey","args":[9,10]},
        {"value": 7}, {"value": 2}, {"func":"StreamGetPreviousKey","args":[12,13]},
        {"value": 7}, {"value": -1}, {"func":"StreamGetPreviousKey","args":[15,16]},
        {"value": 7}, {"value": 5}, {"func":"StreamGetNextKey","args":[18,19]},
        {"value": 7}, {"value": -1}, {"func":"StreamGetValue","args":[21,22]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    vm.context
        .streams
        .insert((7, 0), vec![(1.0, 10.0), (3.0, 30.0)]);

    assert_eq!(vm.execute(2).unwrap(), 0.0);
    assert_eq!(vm.execute(5).unwrap(), 1.0);
    assert_eq!(vm.execute(8).unwrap(), 20.0);
    assert_eq!(vm.execute(11).unwrap(), 3.0);
    assert_eq!(vm.execute(14).unwrap(), 1.0);
    assert_eq!(vm.execute(17).unwrap(), -1.0);
    assert_eq!(vm.execute(20).unwrap(), 5.0);
    assert_eq!(vm.execute(23).unwrap(), 10.0);
}

fn execute_copy(
    source_block: i64,
    source_index: usize,
    destination_block: i64,
    destination_index: usize,
    count: usize,
    initial: &[(i64, usize, f64)],
) -> (f64, renderer::runtime::Memory) {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value":source_block}, {"value":source_index}, {"value":destination_block},
        {"value":destination_index}, {"value":count},
        {"func":"Copy","args":[0,1,2,3,4]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    for &(block, index, value) in initial {
        vm.memory.set(block, index, value);
    }
    let result = vm.execute(5).unwrap();
    (result, vm.memory)
}

#[test]
fn watch_vm_copy_copies_ranges_and_returns_zero() {
    // Disjoint ranges in one block.
    let (result, memory) = execute_copy(10000, 1, 10000, 4, 2, &[(10000, 1, 7.0), (10000, 2, 8.0)]);
    assert_eq!(result, 0.0);
    assert_eq!(memory.get(10000, 4), 7.0);
    assert_eq!(memory.get(10000, 5), 8.0);

    // Distinct supported blocks.
    let (result, memory) = execute_copy(2000, 0, 10000, 3, 2, &[(2000, 0, 11.0), (2000, 1, 12.0)]);
    assert_eq!(result, 0.0);
    assert_eq!(memory.get(10000, 3), 11.0);
    assert_eq!(memory.get(10000, 4), 12.0);

    // Destination starts before source.
    let initial: Vec<_> = (0..5)
        .map(|index| (10000, index, (index + 1) as f64 * 10.0))
        .collect();
    let (result, memory) = execute_copy(10000, 1, 10000, 0, 4, &initial);
    assert_eq!(result, 0.0);
    assert_eq!(
        (0..5).map(|i| memory.get(10000, i)).collect::<Vec<_>>(),
        [20.0, 30.0, 40.0, 50.0, 50.0]
    );

    // Destination starts inside/after source. Snapshot semantics avoid
    // overwriting values that later source reads still need.
    let initial: Vec<_> = (0..5)
        .map(|index| (10000, index, (index + 1) as f64 * 10.0))
        .collect();
    let (result, memory) = execute_copy(10000, 0, 10000, 1, 4, &initial);
    assert_eq!(result, 0.0);
    assert_eq!(
        (0..5).map(|i| memory.get(10000, i)).collect::<Vec<_>>(),
        [10.0, 10.0, 20.0, 30.0, 40.0]
    );

    // A zero count copies no values and still returns the specified zero.
    let (result, memory) = execute_copy(10000, 1, 10000, 0, 0, &[(10000, 0, 5.0)]);
    assert_eq!(result, 0.0);
    assert_eq!(memory.get(10000, 0), 5.0);

    // Exact Next SEKAI regression: overlapping shift of the two-slot cursor.
    let (result, memory) = execute_copy(
        10000,
        1,
        10000,
        0,
        2,
        &[(10000, 0, 1.0), (10000, 1, 37.0), (10000, 2, 39.0)],
    );
    assert_eq!(result, 0.0);
    assert_eq!(memory.get(10000, 0), 37.0);
    assert_eq!(memory.get(10000, 1), 39.0);
}

#[test]
fn watch_vm_and_returns_last_nonzero_argument() {
    use renderer::watch::EngineNode as N;
    let nodes: Vec<N> = serde_json::from_value(serde_json::json!([
        {"value":2.0}, {"value":7.0}, {"value":9.0},
        {"func":"And","args":[0,1,2]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert_eq!(vm.execute(3).unwrap(), 9.0);
}

#[test]
fn watch_vm_and_short_circuits_after_zero() {
    use renderer::watch::EngineNode as N;
    let nodes: Vec<N> = serde_json::from_value(serde_json::json!([
        {"value":0.0}, {"func":"UnsupportedAfterFalse","args":[]},
        {"func":"And","args":[0,1]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert_eq!(vm.execute(2).unwrap(), 0.0);
}

#[test]
fn watch_host_attaches_callback_and_draw_node_provenance() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "archetypes":[{"name":"Drawable","updateParallel":{"index":11}}],
        "nodes":[
            {"value":7},{"value":-1},{"value":-1},{"value":-1},{"value":1},
            {"value":1},{"value":1},{"value":1},{"value":-1},{"value":0},{"value":1},
            {"func":"Draw","args":[0,1,2,3,4,5,6,7,8,9,10]}
        ]
    }))
    .unwrap();
    let level: renderer::formats::LevelData = serde_json::from_value(serde_json::json!({
        "entities":[{"archetype":"Drawable","data":[]}]
    }))
    .unwrap();
    let mut runtime = renderer::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();
    let report = runtime.frame(0.0).unwrap();
    let provenance = report.display_list.sprites[0].provenance.as_ref().unwrap();
    assert_eq!(provenance.entity_id, Some(0));
    assert_eq!(provenance.archetype.as_deref(), Some("Drawable"));
    assert_eq!(provenance.callback.as_deref(), Some("UpdateParallel"));
    assert_eq!(provenance.callback_node, Some(11));
    assert_eq!(provenance.draw_node, 11);
}

#[test]
fn level_option_override_is_visible_during_preprocess_and_empty_override_preserves_default() {
    fn run(overrides: &[(usize, f64)]) -> f64 {
        let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
            "archetypes":[{"name":"Initialization","preprocess":{"index":3}}],
            "nodes":[
                {"value":2002}, {"value":0}, {"func":"Get","args":[0,1]},
                {"func":"DebugLog","args":[2]}
            ]
        }))
        .unwrap();
        let level: renderer::formats::LevelData = serde_json::from_value(serde_json::json!({
            "entities":[{"archetype":"Initialization","data":[]}]
        }))
        .unwrap();
        let configuration = serde_json::json!({"options":[{"def":6.0}]});
        let mut runtime = renderer::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();
        runtime.bind_engine_option_defaults(&configuration).unwrap();
        assert_eq!(runtime.global_memory.get(2002, 0), 6.0);
        runtime
            .bind_engine_option_overrides(&configuration, overrides)
            .unwrap();
        let report = runtime.frame(0.0).unwrap();
        match &report.debug_events[0] {
            renderer::runtime::DebugEvent::Log { value, .. } => *value,
            event => panic!("expected preprocess DebugLog, got {event:?}"),
        }
    }

    assert_eq!(run(&[]), 6.0);
    assert_eq!(run(&[(0, 10.8)]), 10.8);
}

#[test]
fn level_option_override_rejects_out_of_range_index() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "archetypes":[], "nodes":[]
    }))
    .unwrap();
    let level: renderer::formats::LevelData =
        serde_json::from_value(serde_json::json!({"entities":[]})).unwrap();
    let configuration = serde_json::json!({"options":[{"def":6.0}]});
    let mut runtime = renderer::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();
    runtime.bind_engine_option_defaults(&configuration).unwrap();
    assert!(runtime
        .bind_engine_option_overrides(&configuration, &[(1, 10.8)])
        .unwrap_err()
        .to_string()
        .contains("outside EngineConfiguration options"));
}

#[test]
fn skin_rasterizer_samples_bound_sprite_and_composites_alpha() {
    use renderer::formats::{SkinAssets, SkinSpriteAsset};
    use renderer::runtime::{DisplayList, SpriteDraw};
    use std::collections::BTreeMap;

    let identity = [
        [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
    ];
    let runtime_identity = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let skin = SkinAssets {
        width: 1,
        height: 1,
        interpolation: false,
        rgba: vec![255, 0, 0, 128],
        sprites: BTreeMap::from([(
            "half-red".to_owned(),
            SkinSpriteAsset {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                transform: identity,
            },
        )]),
    };
    let display_list = DisplayList {
        sprites: vec![SpriteDraw {
            sprite_id: 7,
            corners: [[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]],
            z: [0.0; 4],
            alpha: 0.5,
            provenance: None,
            trace: None,
        }],
    };
    let ppm = display_list
        .render_skin_ppm(
            2,
            2,
            1.0,
            &skin,
            &BTreeMap::from([(7, "half-red".to_owned())]),
        )
        .unwrap();
    let rgb = display_list
        .render_skin_rgb_with_runtime_transform_and_background(
            2,
            2,
            1.0,
            &skin,
            &BTreeMap::from([(7, "half-red".to_owned())]),
            &runtime_identity,
            None,
        )
        .unwrap();
    assert_eq!(&ppm[..11], b"P6\n2 2\n255\n");
    let pixels = &ppm[11..];
    assert_eq!(rgb, pixels);
    assert_eq!(pixels, &[64, 0, 0, 64, 0, 0, 64, 0, 0, 64, 0, 0]);

    let diagnostic = display_list
        .skin_render_diagnostics(
            2,
            2,
            1.0,
            &skin,
            &BTreeMap::from([(7, "half-red".to_owned())]),
        )
        .unwrap()
        .remove(0);
    assert_eq!(diagnostic.display_list_index, 0);
    assert_eq!(diagnostic.render_order, 0);
    assert_eq!(diagnostic.sprite_name, "half-red");
    assert_eq!(diagnostic.sprite_transform, identity);
    assert_eq!(diagnostic.atlas_nontransparent_pixels, 1);
    assert_eq!(diagnostic.atlas_alpha_range, [128, 128]);
    assert_eq!(diagnostic.nontransparent_quad_pixels, 4);
    assert_eq!(diagnostic.sampled_alpha_range, Some([128, 128]));
    assert_eq!(
        diagnostic.sampled_rgb_range,
        Some([[255, 255], [0, 0], [0, 0]])
    );
    assert_eq!(diagnostic.input_corners, display_list.sprites[0].corners);
    assert_eq!(
        diagnostic.transformed_corners,
        display_list.sprites[0].corners
    );
    assert_eq!(
        diagnostic.screen_pixel_corners,
        [[0.0, 2.0], [0.0, 0.0], [2.0, 0.0], [2.0, 2.0]]
    );
    assert_eq!(diagnostic.atlas_rect, [0, 0, 1, 1]);
    assert_eq!(
        diagnostic.atlas_corner_samples,
        [[-0.5, 0.5], [-0.5, -0.5], [0.5, -0.5], [0.5, 0.5]]
    );
    assert_eq!(diagnostic.unclipped_pixel_bounds, [0, 0, 2, 2]);
    assert_eq!(diagnostic.clipped_pixel_bounds, [0, 0, 2, 2]);

    let uncovered = DisplayList::default();
    let uncovered_ppm = uncovered
        .render_skin_ppm(2, 2, 1.0, &skin, &BTreeMap::new())
        .unwrap();
    let uncovered_rgb = uncovered
        .render_skin_rgb_with_runtime_transform_and_background(
            2,
            2,
            1.0,
            &skin,
            &BTreeMap::new(),
            &[
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            None,
        )
        .unwrap();
    assert_eq!(uncovered_rgb, &uncovered_ppm[11..]);
    assert!(uncovered_rgb.iter().all(|byte| *byte == 0));
}

#[test]
fn runtime_skin_transform_precedes_sprite_transform_and_preserves_draw() {
    use renderer::formats::{SkinAssets, SkinSpriteAsset};
    use renderer::runtime::{DisplayList, SpriteDraw};
    use std::collections::BTreeMap;

    let sprite_transform = [
        [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
    ];
    let skin = SkinAssets {
        width: 1,
        height: 1,
        interpolation: false,
        rgba: vec![255, 255, 255, 255],
        sprites: BTreeMap::from([(
            "white".to_owned(),
            SkinSpriteAsset {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                transform: sprite_transform,
            },
        )]),
    };
    let draw = SpriteDraw {
        sprite_id: 0,
        corners: [[-0.5, -0.5], [-0.5, 0.5], [0.5, 0.5], [0.5, -0.5]],
        z: [0.0; 4],
        alpha: 1.0,
        provenance: None,
        trace: None,
    };
    let display_list = DisplayList {
        sprites: vec![draw.clone()],
    };
    let bindings = BTreeMap::from([(0, "white".to_owned())]);
    let matrix = [
        2.0, 0.0, 0.1, 0.25, 0.0, 2.0, 0.2, -0.25, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let diagnostic = display_list
        .skin_render_diagnostics_with_runtime_transform(100, 100, 1.0, &skin, &bindings, &matrix)
        .unwrap()
        .remove(0);
    assert_eq!(diagnostic.input_corners, draw.corners);
    assert_eq!(
        diagnostic.runtime_transformed_corners,
        [[-0.65, -1.05], [-0.65, 0.95], [1.35, 0.95], [1.35, -1.05]]
    );
    assert_eq!(
        diagnostic.transformed_corners,
        diagnostic.runtime_transformed_corners
    );
    assert_eq!(display_list.sprites[0], draw);
}

#[test]
fn watch_runtime_skin_transform_starts_identity_and_accepts_preprocess_write() {
    let level: renderer::formats::LevelData = serde_json::from_value(serde_json::json!({
        "entities":[{"archetype":"Initialization","data":[]}]
    }))
    .unwrap();
    let identity_watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "archetypes":[{"name":"Initialization"}], "nodes":[]
    }))
    .unwrap();
    let report = renderer::watch_runtime::WatchRuntime::new(&identity_watch, &level)
        .unwrap()
        .frame(0.0)
        .unwrap();
    assert_eq!(
        report.runtime_skin_transform,
        [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]
    );

    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "archetypes":[{"name":"Initialization","preprocess":{"index":3}}],
        "nodes":[{"value":1002},{"value":7},{"value":1.5},{"func":"Set","args":[0,1,2]}]
    }))
    .unwrap();
    let report = renderer::watch_runtime::WatchRuntime::new(&watch, &level)
        .unwrap()
        .frame(0.0)
        .unwrap();
    assert_eq!(report.runtime_skin_transform[7], 1.5);
    assert_eq!(report.runtime_skin_transform[0], 1.0);
}

#[test]
fn skin_rasterizer_preserves_sonolus_bottom_left_corner_and_texture_orientation() {
    use renderer::formats::{SkinAssets, SkinSpriteAsset};
    use renderer::runtime::{DisplayList, SpriteDraw};
    use std::collections::BTreeMap;

    let identity = [
        [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
    ];
    let skin = SkinAssets {
        width: 2,
        height: 2,
        interpolation: false,
        // Top row red/green; bottom row blue/yellow.
        rgba: vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
        ],
        sprites: BTreeMap::from([(
            "quadrants".to_owned(),
            SkinSpriteAsset {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
                transform: identity,
            },
        )]),
    };
    let display_list = DisplayList {
        sprites: vec![SpriteDraw {
            sprite_id: 0,
            corners: [[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]],
            z: [0.0; 4],
            alpha: 1.0,
            provenance: None,
            trace: None,
        }],
    };
    let ppm = display_list
        .render_skin_ppm(
            2,
            2,
            1.0,
            &skin,
            &BTreeMap::from([(0, "quadrants".to_owned())]),
        )
        .unwrap();
    assert_eq!(&ppm[11..], &[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0]);
}

#[test]
fn skin_rasterizer_maps_corner_marked_texture_through_skewed_bilinear_quad() {
    use renderer::formats::{SkinAssets, SkinSpriteAsset};
    use renderer::runtime::{DisplayList, SpriteDraw};
    use std::collections::BTreeMap;

    let identity = [
        [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
    ];
    let mut rgba = Vec::with_capacity(8 * 8 * 4);
    for y in 0..8 {
        for x in 0..8 {
            let color = match (x >= 4, y >= 4) {
                (false, false) => [0, 255, 0], // texture top-left
                (true, false) => [0, 0, 255],  // texture top-right
                (false, true) => [255, 0, 0],  // texture bottom-left
                (true, true) => [255, 255, 0], // texture bottom-right
            };
            rgba.extend_from_slice(&[color[0], color[1], color[2], 255]);
        }
    }
    let skin = SkinAssets {
        width: 8,
        height: 8,
        interpolation: false,
        rgba,
        sprites: BTreeMap::from([(
            "corner-marked".to_owned(),
            SkinSpriteAsset {
                x: 0,
                y: 0,
                width: 8,
                height: 8,
                transform: identity,
            },
        )]),
    };

    // An asymmetric, convex trapezoid exercises bilinear interpolation over
    // all four supplied corners; its lower-left extent is clipped by the view.
    let quad = [[-1.3, -1.2], [-0.8, 1.2], [1.1, 0.9], [0.5, -0.6]];
    let display_list = DisplayList {
        sprites: vec![SpriteDraw {
            sprite_id: 4,
            corners: quad,
            z: [0.0; 4],
            alpha: 1.0,
            provenance: None,
            trace: None,
        }],
    };
    let ppm = display_list
        .render_skin_ppm(
            128,
            128,
            1.0,
            &skin,
            &BTreeMap::from([(4, "corner-marked".to_owned())]),
        )
        .unwrap();
    let header = b"P6\n128 128\n255\n";
    assert!(ppm.starts_with(header));
    let rgb = &ppm[header.len()..];
    let pixel_at_uv = |u: f64, v: f64| {
        let p00 = quad[0]; // bottom-left
        let p10 = quad[3]; // bottom-right
        let p01 = quad[1]; // top-left
        let p11 = quad[2]; // top-right
        let point = [
            p00[0]
                + u * (p10[0] - p00[0])
                + v * (p01[0] - p00[0])
                + u * v * (p11[0] - p10[0] - p01[0] + p00[0]),
            p00[1]
                + u * (p10[1] - p00[1])
                + v * (p01[1] - p00[1])
                + u * v * (p11[1] - p10[1] - p01[1] + p00[1]),
        ];
        let x = (((point[0] + 1.0) * 0.5 * 128.0).floor() as usize).min(127);
        let y = (((1.0 - point[1]) * 0.5 * 128.0).floor() as usize).min(127);
        let offset = (y * 128 + x) * 3;
        [rgb[offset], rgb[offset + 1], rgb[offset + 2]]
    };

    // Independent colored corners establish both Sonolus's BL/TL/TR/BR order
    // and the atlas's top-down row orientation through a skewed destination.
    assert_eq!(pixel_at_uv(0.2, 0.2), [255, 0, 0]); // BL
    assert_eq!(pixel_at_uv(0.2, 0.8), [0, 255, 0]); // TL
    assert_eq!(pixel_at_uv(0.8, 0.8), [0, 0, 255]); // TR
    assert_eq!(pixel_at_uv(0.8, 0.2), [255, 255, 0]); // BR

    // With atlas interpolation enabled, the center sample blends the four
    // differently colored quadrants. This also exercises the trapezoid's
    // bilinear interior rather than a pair of triangle-specific mappings.
    let interpolated_skin = SkinAssets {
        interpolation: true,
        ..skin.clone()
    };
    let interpolated = display_list
        .render_skin_ppm(
            128,
            128,
            1.0,
            &interpolated_skin,
            &BTreeMap::from([(4, "corner-marked".to_owned())]),
        )
        .unwrap();
    let center_world = [
        (quad[0][0] + quad[1][0] + quad[2][0] + quad[3][0]) / 4.0,
        (quad[0][1] + quad[1][1] + quad[2][1] + quad[3][1]) / 4.0,
    ];
    let center_x = (((center_world[0] + 1.0) * 0.5 * 128.0).floor() as usize).min(127);
    let center_y = (((1.0 - center_world[1]) * 0.5 * 128.0).floor() as usize).min(127);
    let center_offset = header.len() + (center_y * 128 + center_x) * 3;
    let center = &interpolated[center_offset..center_offset + 3];
    assert!((100..=155).contains(&center[0]), "center red={}", center[0]);
    assert!(
        (100..=155).contains(&center[1]),
        "center green={}",
        center[1]
    );
    assert!((45..=85).contains(&center[2]), "center blue={}", center[2]);

    let nonblack = rgb
        .chunks_exact(3)
        .filter(|pixel| *pixel != [0, 0, 0])
        .count();
    assert!(
        nonblack > 5_000,
        "skewed quad coverage was only {nonblack} pixels"
    );
    assert_eq!(
        &rgb[0..3],
        &[0, 0, 0],
        "outside clipped quad stays untouched"
    );
}

#[test]
fn watch_vm_lerp_uses_endpoints_then_fraction() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 10}, {"value": 20}, {"value": 0.25}, {"func":"Lerp","args":[0,1,2]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert_eq!(vm.execute(3).unwrap(), 12.5);
}

#[test]
fn background_fit_modes_produce_documented_centered_runtime_quads() {
    let data = renderer::formats::BackgroundData {
        aspect_ratio: Some(1.0),
        fit: "contain".to_owned(),
        color: "#000000".to_owned(),
        scale_x: None,
        scale_y: None,
    };
    let contain = data.runtime_quad(2.0, 1.0).unwrap();
    assert_eq!(
        contain,
        [[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]]
    );

    let mut data = data.clone();
    data.fit = "cover".to_owned();
    assert_eq!(
        data.runtime_quad(2.0, 1.0).unwrap(),
        [[-2.0, -2.0], [-2.0, 2.0], [2.0, 2.0], [2.0, -2.0]]
    );
    data.fit = "width".to_owned();
    assert_eq!(
        data.runtime_quad(2.0, 1.0).unwrap(),
        [[-2.0, -2.0], [-2.0, 2.0], [2.0, 2.0], [2.0, -2.0]]
    );
    data.fit = "height".to_owned();
    data.scale_x = Some(2.0);
    data.scale_y = Some(0.5);
    assert_eq!(
        data.runtime_quad(2.0, 1.0).unwrap(),
        [[-2.0, -0.5], [-2.0, 0.5], [2.0, 0.5], [2.0, -0.5]]
    );
}

#[test]
fn runtime_background_block_reads_initialized_quad_and_allows_documented_writes() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 1004}, {"value": 0}, {"func": "Get", "args": [0, 1]},
        {"value": 0.25}, {"func": "Set", "args": [0, 1, 3]},
        {"func": "Get", "args": [0, 1]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    vm.context.lifecycle_stage = Some(renderer::watch_runtime::LifecycleStage::Preprocess as u8);
    vm.context.runtime_background = std::sync::Arc::new(std::sync::RwLock::new([
        -1.0, -1.0, -1.0, 1.0, 1.0, 1.0, 1.0, -1.0,
    ]));
    assert_eq!(vm.execute(2).unwrap(), -1.0);
    vm.execute(4).unwrap();
    assert_eq!(vm.execute(5).unwrap(), 0.25);

    vm.context.lifecycle_stage =
        Some(renderer::watch_runtime::LifecycleStage::UpdateParallel as u8);
    assert!(vm.execute(4).is_err());
}

#[test]
fn background_renders_first_and_applies_data_color_and_configuration_mask() {
    let background = renderer::formats::BackgroundAssets {
        width: 1,
        height: 1,
        rgba: vec![0, 0, 255, 255],
        data: renderer::formats::BackgroundData {
            aspect_ratio: Some(1.0),
            fit: "cover".to_owned(),
            color: "#000000".to_owned(),
            scale_x: None,
            scale_y: None,
        },
        configuration: renderer::formats::BackgroundConfiguration {
            blur: 0.0,
            mask: "#FF000080".to_owned(),
        },
    };
    let identity = std::array::from_fn(|row| {
        std::array::from_fn(|column| if row == column { 1.0 } else { 0.0 })
    });
    let runtime_identity = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let skin = renderer::formats::SkinAssets {
        width: 1,
        height: 1,
        interpolation: false,
        rgba: vec![255, 255, 255, 255],
        sprites: std::collections::BTreeMap::from([(
            "white".to_owned(),
            renderer::formats::SkinSpriteAsset {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                transform: identity,
            },
        )]),
    };
    let draws = renderer::runtime::DisplayList {
        sprites: vec![renderer::runtime::SpriteDraw {
            sprite_id: 0,
            corners: [[-0.5, -0.5], [-0.5, 0.5], [0.5, 0.5], [0.5, -0.5]],
            z: [0.0; 4],
            alpha: 0.5,
            provenance: None,
            trace: None,
        }],
    };
    let ppm = draws
        .render_skin_ppm_with_background(
            2,
            2,
            1.0,
            &skin,
            &std::collections::BTreeMap::from([(0, "white".to_owned())]),
            Some((
                &background,
                [[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]],
            )),
        )
        .unwrap();
    let rgb = draws
        .render_skin_rgb_with_runtime_transform_and_background(
            2,
            2,
            1.0,
            &skin,
            &std::collections::BTreeMap::from([(0, "white".to_owned())]),
            &runtime_identity,
            Some((
                &background,
                [[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]],
            )),
        )
        .unwrap();
    assert_eq!(rgb, &ppm[11..]);
    assert_eq!(&ppm[11..14], &[192, 128, 191]);
}

#[test]
fn supplied_horizon_background_resource_resolves_and_decodes() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let engine =
        renderer::formats::load_engine(&repo.join("TestingSuite/Horizon/Horizon.zip")).unwrap();
    let name = engine.metadata.background_name.as_deref().unwrap();
    let background = renderer::formats::load_background_assets(
        &repo.join("TestingSuite/Horizon/project(1).scp"),
        name,
    )
    .unwrap();
    assert!(background.width > 0 && background.height > 0);
    assert_eq!(background.data.fit, "cover");
    assert_eq!(background.data.color, "#23043c");
    assert_eq!(background.configuration.blur, 0.0);
}

#[test]
fn watch_vm_unlerp_uses_range_then_value_and_clamps_only_when_requested() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 10}, {"value": 20}, {"value": 15}, {"value": 5}, {"value": 25},
        {"func":"Unlerp","args":[0,1,2]},
        {"func":"Unlerp","args":[0,1,3]},
        {"func":"Unlerp","args":[0,1,4]},
        {"func":"UnlerpClamped","args":[0,1,3]},
        {"func":"UnlerpClamped","args":[0,1,4]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert_eq!(vm.execute(5).unwrap(), 0.5);
    assert_eq!(vm.execute(6).unwrap(), -0.5);
    assert_eq!(vm.execute(7).unwrap(), 1.5);
    assert_eq!(vm.execute(8).unwrap(), 0.0);
    assert_eq!(vm.execute(9).unwrap(), 1.0);
}

#[test]
fn watch_vm_ease_in_out_quad_matches_sonolus_piecewise_curve_without_clamping() {
    let samples = [
        (0.0, 0.0),
        (0.25, 0.125),
        (0.49, 0.4802),
        (0.5, 0.5),
        (0.75, 0.875),
        (1.0, 1.0),
        (-0.5, 0.5),
        (1.5, 0.5),
    ];

    for (input, expected) in samples {
        let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
            {"value":input}, {"func":"EaseInOutQuad","args":[0]}
        ]))
        .unwrap();
        let mut vm = renderer::runtime::WatchVm::new(&nodes);
        let actual = vm.execute(1).unwrap();
        assert!(
            (actual - expected).abs() < 1e-12,
            "EaseInOutQuad({input}) = {actual}, expected {expected}"
        );
    }
}

#[test]
fn watch_vm_implements_all_documented_sonolus_easing_curves() {
    // Reference samples at x=0.25 and x=0.75 calculated from the
    // specification-linked easings.net equations. Two interior points also
    // exercise both halves of each InOut/OutIn curve.
    let cases = [
        ("EaseInSine", 0.07612046748871326, 0.6173165676349102),
        ("EaseInQuad", 0.0625, 0.5625),
        ("EaseInCubic", 0.015625, 0.421875),
        ("EaseInQuart", 0.00390625, 0.31640625),
        ("EaseInQuint", 0.0009765625, 0.2373046875),
        ("EaseInExpo", 0.005524271728019903, 0.1767766952966369),
        ("EaseInCirc", 0.031754163448145745, 0.3385621722338523),
        ("EaseInBack", -0.0641365625, 0.1825903124999997),
        ("EaseInElastic", -0.005524271728019903, 0.08838834764831845),
        ("EaseOutSine", 0.3826834323650898, 0.9238795325112867),
        ("EaseOutQuad", 0.4375, 0.9375),
        ("EaseOutCubic", 0.578125, 0.984375),
        ("EaseOutQuart", 0.68359375, 0.99609375),
        ("EaseOutQuint", 0.7626953125, 0.9990234375),
        ("EaseOutExpo", 0.8232233047033631, 0.99447572827198),
        ("EaseOutCirc", 0.6614378277661477, 0.9682458365518543),
        ("EaseOutBack", 0.8174096875000002, 1.0641365625),
        ("EaseOutElastic", 0.9116116523516816, 1.00552427172802),
        ("EaseInOutSine", 0.1464466094067262, 0.8535533905932737),
        ("EaseInOutQuad", 0.125, 0.875),
        ("EaseInOutCubic", 0.0625, 0.9375),
        ("EaseInOutQuart", 0.03125, 0.96875),
        ("EaseInOutQuint", 0.015625, 0.984375),
        ("EaseInOutExpo", 0.015625, 0.984375),
        ("EaseInOutCirc", 0.0669872981077807, 0.9330127018922193),
        ("EaseInOutBack", -0.04384875000000002, 1.04384875),
        ("EaseInOutElastic", -0.007812500000000023, 1.0078125),
        ("EaseOutInSine", 0.3535533905932737, 0.6464466094067263),
        ("EaseOutInQuad", 0.375, 0.625),
        ("EaseOutInCubic", 0.4375, 0.5625),
        ("EaseOutInQuart", 0.46875, 0.53125),
        ("EaseOutInQuint", 0.484375, 0.515625),
        ("EaseOutInExpo", 0.484375, 0.515625),
        ("EaseOutInCirc", 0.4330127018922193, 0.5669872981077807),
        ("EaseOutInBack", 0.54384875, 0.45615125),
        ("EaseOutInElastic", 0.5078125, 0.4921875),
    ];

    for (name, at_quarter, at_three_quarters) in cases {
        for (input, expected) in [
            (0.0, 0.0),
            (0.25, at_quarter),
            (0.75, at_three_quarters),
            (1.0, 1.0),
        ] {
            let nodes: Vec<renderer::watch::EngineNode> =
                serde_json::from_value(serde_json::json!([
                    {"value":input}, {"func":name,"args":[0]}
                ]))
                .unwrap();
            let mut vm = renderer::runtime::WatchVm::new(&nodes);
            let actual = vm.execute(1).unwrap();
            assert!(
                (actual - expected).abs() < 1e-12,
                "{name}({input}) = {actual}, expected {expected}"
            );
        }
    }
}

#[test]
fn watch_vm_easing_extrapolates_without_undocumented_clamping() {
    for (name, input, expected) in [
        ("EaseInQuad", -0.5, 0.25),
        ("EaseOutQuad", 1.5, 0.75),
        ("EaseInOutQuad", -0.5, 0.5),
        ("EaseOutInQuad", 1.5, 2.5),
    ] {
        let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
            {"value":input}, {"func":name,"args":[0]}
        ]))
        .unwrap();
        let mut vm = renderer::runtime::WatchVm::new(&nodes);
        let actual = vm.execute(1).unwrap();
        assert_eq!(actual, expected, "{name}({input})");
    }
}

#[test]
fn watch_vm_play_scheduled_emits_timeline_effect_and_returns_zero() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 7}, {"value": 2.0}, {"value": 0.25},
        {"func":"PlayScheduled","args":[0,1,2]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    vm.context.time = 1.0;
    assert_eq!(vm.execute(3).unwrap(), 0.0);
    assert_eq!(vm.scheduled_effects.len(), 1);
    assert_eq!(
        vm.scheduled_effects[0],
        renderer::runtime::ScheduledEffect {
            clip_id: 7,
            time: 2.0,
            minimum_distance: 0.25,
            requested_at: 1.0,
            has_required_lead_time: true,
        }
    );

    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 7}, {"value": 1.25}, {"value": 0.25},
        {"func":"PlayScheduled","args":[0,1,2]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    vm.context.time = 1.0;
    assert_eq!(vm.execute(3).unwrap(), 0.0);
    assert!(!vm.scheduled_effects[0].has_required_lead_time);
}

#[test]
fn watch_vm_immediate_audio_emits_commands_and_shares_loop_handles() {
    let play_nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 12}, {"value": 0.25}, {"func":"Play","args":[0,1]},
        {"value": 13}, {"func":"PlayLooped","args":[3]}
    ]))
    .unwrap();
    let mut play_vm = renderer::runtime::WatchVm::new(&play_nodes);
    play_vm.context.time = 1.5;
    assert_eq!(play_vm.execute(2).unwrap(), 0.0);
    let instance_id = play_vm.execute(4).unwrap();
    assert_eq!(instance_id, 0.0);
    assert_eq!(
        play_vm.audio_events,
        vec![
            renderer::runtime::AudioEffectEvent::Play {
                clip_id: 12,
                minimum_distance: 0.25,
                requested_at: 1.5,
            },
            renderer::runtime::AudioEffectEvent::StartLoop {
                instance_id: 0,
                clip_id: 13,
                requested_at: 1.5,
            }
        ]
    );

    let stop_nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": instance_id}, {"func":"StopLooped","args":[0]}
    ]))
    .unwrap();
    let mut stop_vm = renderer::runtime::WatchVm::new(&stop_nodes);
    stop_vm.context = play_vm.context.clone();
    stop_vm.context.time = 2.0;
    assert_eq!(stop_vm.execute(1).unwrap(), 0.0);
    assert_eq!(
        stop_vm.audio_events,
        vec![renderer::runtime::AudioEffectEvent::StopLoop {
            instance_id: 0,
            requested_at: 2.0,
        }]
    );
}

#[test]
fn watch_vm_play_looped_scheduled_returns_unique_ids_and_retains_schedule() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 7}, {"value": 3.0},
        {"func":"PlayLoopedScheduled","args":[0,1]},
        {"value": 7}, {"value": 4.0},
        {"func":"PlayLoopedScheduled","args":[3,4]},
        {"value": 9}, {"value": 2.25},
        {"func":"PlayLoopedScheduled","args":[6,7]},
        {"value": 10000}, {"value": 0}, {"func":"Set","args":[9,10,2]},
        {"value": 1}, {"func":"Set","args":[9,12,5]},
        {"value": 2}, {"func":"Set","args":[9,14,8]},
        {"func":"Execute","args":[11,13,15]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    vm.context.time = 2.0;

    let final_id = vm.execute(16).unwrap();
    let events = &vm.scheduled_looped_effects;
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].clip_id, 7);
    assert_eq!(events[0].start_time, 3.0);
    assert_eq!(events[0].requested_at, 2.0);
    assert!(events[0].has_required_lead_time);
    assert_eq!(events[1].clip_id, 7);
    assert_eq!(events[1].start_time, 4.0);
    assert_eq!(events[2].clip_id, 9);
    assert_eq!(events[2].start_time, 2.25);
    assert!(!events[2].has_required_lead_time);

    assert_ne!(events[0].instance_id, events[1].instance_id);
    assert_ne!(events[0].instance_id, events[2].instance_id);
    assert_ne!(events[1].instance_id, events[2].instance_id);
    assert_eq!(vm.memory.get(10000, 0), events[0].instance_id as f64);
    assert_eq!(vm.memory.get(10000, 1), events[1].instance_id as f64);
    assert_eq!(vm.memory.get(10000, 2), events[2].instance_id as f64);
    assert_eq!(final_id, events[2].instance_id as f64);
}

#[test]
fn watch_vm_play_looped_scheduled_ids_are_shared_between_callback_vms() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 7}, {"value": 3.0},
        {"func":"PlayLoopedScheduled","args":[0,1]}
    ]))
    .unwrap();
    let mut first = renderer::runtime::WatchVm::new(&nodes);
    let first_id = first.execute(2).unwrap();

    let mut second = renderer::runtime::WatchVm::new(&nodes);
    second.context = first.context.clone();
    let second_id = second.execute(2).unwrap();

    assert_ne!(first_id, second_id);
    assert_eq!(first.scheduled_looped_effects[0].clip_id, 7);
    assert_eq!(second.scheduled_looped_effects[0].clip_id, 7);
}

#[test]
fn watch_vm_stop_looped_scheduled_targets_instance_and_returns_zero() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 7}, {"value": 3.0},
        {"func":"PlayLoopedScheduled","args":[0,1]},
        {"value": 7}, {"value": 4.0},
        {"func":"PlayLoopedScheduled","args":[3,4]},
        {"value": 4.0}, {"value": 1.0}, {"func":"Add","args":[6,7]},
        {"func":"StopLoopedScheduled","args":[2,8]},
        {"value": 4.25},
        {"func":"StopLoopedScheduled","args":[5,10]},
        {"func":"Execute","args":[9,11]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    vm.context.time = 4.0;

    assert_eq!(vm.execute(12).unwrap(), 0.0);
    assert_eq!(vm.function_counts.get("Add"), Some(&1));
    assert_eq!(vm.scheduled_looped_effects.len(), 2);
    let [loop_a, loop_b] = vm.scheduled_looped_effects.as_slice() else {
        panic!("expected two scheduled loop instances");
    };
    assert_ne!(loop_a.instance_id, loop_b.instance_id);
    assert_eq!(loop_a.clip_id, 7);
    assert_eq!(loop_b.clip_id, 7);

    let [stop_a, stop_b] = vm.scheduled_looped_effect_stops.as_slice() else {
        panic!("expected two scheduled loop stops");
    };
    assert_eq!(stop_a.instance_id, loop_a.instance_id);
    assert_eq!(stop_a.end_time, 5.0);
    assert_eq!(stop_a.requested_at, 4.0);
    assert!(stop_a.has_required_lead_time);
    assert_eq!(stop_b.instance_id, loop_b.instance_id);
    assert_ne!(stop_a.instance_id, stop_b.instance_id);
    assert_eq!(stop_b.end_time, 4.25);
    assert_eq!(stop_b.requested_at, 4.0);
    assert!(!stop_b.has_required_lead_time);
}

#[test]
fn watch_vm_stop_looped_scheduled_accepts_shared_id_from_another_callback_vm() {
    let play_nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 7}, {"value": 3.0},
        {"func":"PlayLoopedScheduled","args":[0,1]}
    ]))
    .unwrap();
    let mut play_vm = renderer::runtime::WatchVm::new(&play_nodes);
    let loop_id = play_vm.execute(2).unwrap();

    let stop_nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": loop_id}, {"value": 8.0},
        {"func":"StopLoopedScheduled","args":[0,1]}
    ]))
    .unwrap();
    let mut stop_vm = renderer::runtime::WatchVm::new(&stop_nodes);
    stop_vm.context = play_vm.context.clone();
    assert_eq!(stop_vm.execute(2).unwrap(), 0.0);
    assert_eq!(
        stop_vm.scheduled_looped_effect_stops[0].instance_id as f64,
        loop_id
    );
}

#[test]
fn watch_vm_destroy_particle_effect_emits_instance_command_and_returns_zero() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 17},
        {"func":"DestroyParticleEffect","args":[0]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);

    assert_eq!(vm.execute(1).unwrap(), 0.0);
    assert_eq!(
        vm.destroyed_particle_effects,
        vec![renderer::runtime::DestroyedParticleEffect { particle_id: 17 }]
    );

    for invalid in [1.5, f64::NAN, f64::INFINITY] {
        let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(
            serde_json::json!([{"value": invalid}, {"func":"DestroyParticleEffect","args":[0]}]),
        )
        .unwrap();
        let mut vm = renderer::runtime::WatchVm::new(&nodes);
        assert!(vm.execute(1).is_err(), "accepted invalid id {invalid}");
    }

    let no_args: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"func":"DestroyParticleEffect","args":[]}
    ]))
    .unwrap();
    assert!(renderer::runtime::WatchVm::new(&no_args)
        .execute(0)
        .is_err());
}

#[test]
fn watch_host_reports_destroy_particle_effect_from_entity_callback() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "archetypes":[{"name":"Thing","updateParallel":{"index":1}}],
        "nodes":[{"value": 29}, {"func":"DestroyParticleEffect","args":[0]}]
    }))
    .unwrap();
    let level: renderer::formats::LevelData = serde_json::from_value(serde_json::json!({
        "entities":[{"archetype":"Thing","data":[]}]
    }))
    .unwrap();
    let mut runtime = renderer::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();

    let report = runtime.frame(0.0).unwrap();
    assert_eq!(report.destroyed_particle_effects.len(), 1);
    assert_eq!(report.destroyed_particle_effects[0].particle_id, 29);
}

#[test]
fn watch_vm_spawn_particle_effect_returns_unique_ids_and_can_be_destroyed() {
    fn spawn_nodes(effect_id: f64, looped: f64) -> Vec<renderer::watch::EngineNode> {
        serde_json::from_value(serde_json::json!([
            {"value": effect_id},
            {"value": -1.0}, {"value": -0.5},
            {"value": -1.0}, {"value": 0.5},
            {"value": 1.0}, {"value": 0.5},
            {"value": 1.0}, {"value": -0.5},
            {"value": 2.5}, {"value": looped},
            {"func":"SpawnParticleEffect","args":[0,1,2,3,4,5,6,7,8,9,10]}
        ]))
        .unwrap()
    }

    let first_nodes = spawn_nodes(7.0, 0.0);
    let mut spawner = renderer::runtime::WatchVm::new(&first_nodes);
    spawner.context.time = 3.25;
    assert_eq!(spawner.execute(11).unwrap(), 0.0);
    let first = spawner.particle_events[0].clone();
    assert!(matches!(
        &first,
        renderer::runtime::ParticleEffectEvent::Spawn(instance)
            if instance.instance_id == 0
                && instance.effect_id == 7
                && instance.corners == [[-1.0, -0.5], [-1.0, 0.5], [1.0, 0.5], [1.0, -0.5]]
                && instance.duration == 2.5
                && !instance.is_looped
                && instance.spawned_at == 3.25
    ));

    let second_nodes = spawn_nodes(8.0, 1.0);
    let mut second_spawner = renderer::runtime::WatchVm::new(&second_nodes);
    second_spawner.context = spawner.context.clone();
    assert_eq!(second_spawner.execute(11).unwrap(), 1.0);
    assert!(matches!(
        &second_spawner.particle_events[0],
        renderer::runtime::ParticleEffectEvent::Spawn(instance)
            if instance.instance_id == 1 && instance.effect_id == 8 && instance.is_looped
    ));
    {
        let state = second_spawner.context.particle_instances.read().unwrap();
        assert_eq!(state.instances().len(), 2);
        assert!(state.instances().contains_key(&0));
        assert!(state.instances().contains_key(&1));
    }

    let destroy_nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(
        serde_json::json!([{"value": 0}, {"func":"DestroyParticleEffect","args":[0]}]),
    )
    .unwrap();
    let mut destroyer = renderer::runtime::WatchVm::new(&destroy_nodes);
    destroyer.context = spawner.context.clone();
    assert_eq!(destroyer.execute(1).unwrap(), 0.0);
    assert_eq!(
        destroyer.particle_events,
        vec![renderer::runtime::ParticleEffectEvent::Destroy { instance_id: 0 }]
    );
    let state = destroyer.context.particle_instances.read().unwrap();
    assert!(!state.instances().contains_key(&0));
    assert!(state.instances().contains_key(&1));
    drop(state);

    // A fresh run starts from the same allocator state and produces the same ID.
    let fresh_nodes = spawn_nodes(7.0, 0.0);
    let mut fresh = renderer::runtime::WatchVm::new(&fresh_nodes);
    fresh.context.time = 3.25;
    assert_eq!(fresh.execute(11).unwrap(), 0.0);
    assert_eq!(fresh.particle_events, vec![first]);
}

#[test]
fn watch_vm_move_particle_effect_updates_live_instance_and_emits_corners() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 3},
        {"value": 0}, {"value": 1}, {"value": 2}, {"value": 3},
        {"value": 4}, {"value": 5}, {"value": 6}, {"value": 7},
        {"value": 30}, {"value": 1},
        {"func":"SpawnParticleEffect","args":[0,1,2,3,4,5,6,7,8,9,10]},
        {"value": 10}, {"value": 11}, {"value": 12}, {"value": 13},
        {"value": 14}, {"value": 15}, {"value": 16}, {"value": 17},
        {"func":"MoveParticleEffect","args":[11,12,13,14,15,16,17,18,19]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert_eq!(vm.execute(20).unwrap(), 0.0);
    let instance_id = 0;
    let expected_corners = [[10.0, 11.0], [12.0, 13.0], [14.0, 15.0], [16.0, 17.0]];
    let instances = vm.context.particle_instances.read().unwrap();
    assert_eq!(
        instances.instances()[&instance_id].corners,
        expected_corners
    );
    assert!(matches!(
        vm.particle_events.last(),
        Some(renderer::runtime::ParticleEffectEvent::Move { instance_id: id, corners })
            if *id == instance_id && *corners == expected_corners
    ));
}

#[test]
fn watch_vm_move_particle_effect_requires_documented_argument_count() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 1}, {"func":"MoveParticleEffect","args":[0]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert!(format!("{:#}", vm.execute(1).unwrap_err()).contains("requires 9 arguments"));
}

#[test]
fn watch_vm_spawn_particle_effect_rejects_invalid_arity_and_identifier() {
    for values in [
        vec![1.0; 10],
        vec![1.0; 12],
        vec![1.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
    ] {
        let mut nodes: Vec<renderer::watch::EngineNode> = values
            .iter()
            .map(|value| serde_json::json!({"value": value}))
            .collect::<Vec<_>>()
            .into_iter()
            .map(serde_json::from_value)
            .collect::<Result<_, _>>()
            .unwrap();
        let last = nodes.len();
        nodes.push(
            serde_json::from_value(serde_json::json!({
                "func":"SpawnParticleEffect",
                "args":(0..last).collect::<Vec<_>>()
            }))
            .unwrap(),
        );
        let mut vm = renderer::runtime::WatchVm::new(&nodes);
        assert!(
            vm.execute(last).is_err(),
            "accepted spawn arguments {values:?}"
        );
        assert!(vm
            .context
            .particle_instances
            .read()
            .unwrap()
            .instances()
            .is_empty());
    }

    for invalid_id in [f64::NAN, f64::INFINITY] {
        let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
            {"value": 9000}, {"value": 0}, {"func":"Get","args":[0,1]},
            {"value": 0}, {"value": 0}, {"value": 0}, {"value": 0},
            {"value": 0}, {"value": 0}, {"value": 0}, {"value": 0},
            {"value": 1}, {"value": 0},
            {"func":"SpawnParticleEffect","args":[2,3,4,5,6,7,8,9,10,11,12]}
        ]))
        .unwrap();
        let mut vm = renderer::runtime::WatchVm::new(&nodes);
        vm.memory.set(9000, 0, invalid_id);
        assert!(vm.execute(13).is_err(), "accepted effect id {invalid_id}");
        assert!(vm
            .context
            .particle_instances
            .read()
            .unwrap()
            .instances()
            .is_empty());
    }
}

#[test]
fn watch_vm_spawn_particle_effect_missing_sentinel_is_noop() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": -1},
        {"value": -1.0}, {"value": -0.5},
        {"value": -1.0}, {"value": 0.5},
        {"value": 1.0}, {"value": 0.5},
        {"value": 1.0}, {"value": -0.5},
        {"value": 2.5}, {"value": 0},
        {"func":"SpawnParticleEffect","args":[0,1,2,3,4,5,6,7,8,9,10]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);

    assert_eq!(vm.execute(11).unwrap(), 0.0);
    assert!(vm.particle_events.is_empty());
    assert!(vm
        .context
        .particle_instances
        .read()
        .unwrap()
        .instances()
        .is_empty());
}

#[test]
fn watch_host_reports_particle_spawn_events_from_callbacks() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "archetypes":[{"name":"Thing","updateParallel":{"index":11}}],
        "nodes":[
            {"value": 4},
            {"value": -1.0}, {"value": -1.0},
            {"value": -1.0}, {"value": 1.0},
            {"value": 1.0}, {"value": 1.0},
            {"value": 1.0}, {"value": -1.0},
            {"value": 0.5}, {"value": 0},
            {"func":"SpawnParticleEffect","args":[0,1,2,3,4,5,6,7,8,9,10]}
        ]
    }))
    .unwrap();
    let level: renderer::formats::LevelData = serde_json::from_value(serde_json::json!({
        "entities":[{"archetype":"Thing","data":[]}]
    }))
    .unwrap();
    let mut runtime = renderer::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();

    let report = runtime.frame(0.0).unwrap();
    assert!(matches!(
        &report.particle_events[..],
        [renderer::runtime::ParticleEffectEvent::Spawn(instance)]
            if instance.instance_id == 0 && instance.effect_id == 4
    ));
}

#[test]
fn watch_host_reports_scheduled_effect_commands_from_entity_callbacks() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "archetypes":[{"name":"Thing","updateParallel":{"index":3}}],
        "nodes":[
            {"value": 12}, {"value": 0.75}, {"value": 0.1},
            {"func":"PlayScheduled","args":[0,1,2]}
        ]
    }))
    .unwrap();
    let level: renderer::formats::LevelData = serde_json::from_value(serde_json::json!({
        "entities":[{"archetype":"Thing","data":[]}]
    }))
    .unwrap();
    let mut runtime = renderer::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();
    let report = runtime.frame(0.0).unwrap();
    assert_eq!(report.scheduled_effects.len(), 1);
    assert_eq!(report.scheduled_effects[0].clip_id, 12);
    assert_eq!(report.scheduled_effects[0].time, 0.75);
    assert!(report.scheduled_effects[0].has_required_lead_time);
}

#[test]
fn watch_host_includes_immediate_audio_commands_in_frame_report() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "archetypes":[{"name":"Thing","updateParallel":{"index":2}}],
        "nodes":[
            {"value":7}, {"value":0.1}, {"func":"Play","args":[0,1]}
        ]
    }))
    .unwrap();
    let level: renderer::formats::LevelData = serde_json::from_value(serde_json::json!({
        "entities":[{"archetype":"Thing","data":[]}]
    }))
    .unwrap();
    let mut runtime = renderer::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();
    let report = runtime.frame(0.0).unwrap();
    assert_eq!(
        report.audio_events,
        vec![renderer::runtime::AudioEffectEvent::Play {
            clip_id: 7,
            minimum_distance: 0.1,
            requested_at: 0.0,
        }]
    );
}

#[test]
fn watch_host_reports_scheduled_loop_stops_from_entity_callbacks() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "archetypes":[{"name":"Thing","updateParallel":{"index":4}}],
        "nodes":[
            {"value": 17}, {"value": 3.0},
            {"func":"PlayLoopedScheduled","args":[0,1]},
            {"value": 5.0},
            {"func":"StopLoopedScheduled","args":[2,3]}
        ]
    }))
    .unwrap();
    let level: renderer::formats::LevelData = serde_json::from_value(serde_json::json!({
        "entities":[{"archetype":"Thing","data":[]}]
    }))
    .unwrap();
    let mut runtime = renderer::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();
    let report = runtime.frame(0.0).unwrap();
    assert_eq!(report.scheduled_looped_effects.len(), 1);
    assert_eq!(report.scheduled_looped_effect_stops.len(), 1);
    assert_eq!(
        report.scheduled_looped_effect_stops[0].instance_id,
        report.scheduled_looped_effects[0].instance_id
    );
    assert_eq!(report.scheduled_looped_effect_stops[0].end_time, 5.0);
}

#[test]
fn watch_draw_trace_preserves_non_finite_values_and_compound_memory_inputs() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 1}, {"value": 10000}, {"value": 0}, {"value": 0},
        {"func":"Divide","args":[3,3]},
        {"func":"SetMultiply","args":[1,2,4]},
        {"func":"Draw","args":[0,5,3,5,3,5,3,5,3,3,3,3,3,3]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    vm.set_draw_tracing(true);
    assert_eq!(vm.execute(6).unwrap(), 0.0);
    let trace = vm.display_list.sprites[0].trace.as_ref().unwrap();
    assert_eq!(
        trace.node_value_texts.get(&4).map(String::as_str),
        Some("NaN")
    );
    assert!(trace.memory_operations.iter().any(|operation| {
        operation.node == 5
            && operation.function == "SetMultiply"
            && operation.old_value == "0"
            && operation.operand == "NaN"
            && operation.result == "NaN"
    }));
}

#[test]
fn watch_vm_break_uses_count_then_value_and_unwinds_requested_blocks() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value":1},{"value":42},{"func":"Break","args":[0,1]},
        {"func":"Block","args":[2]},
        {"value":2},{"value":77},{"func":"Break","args":[4,5]},
        {"func":"Block","args":[6]},{"func":"Block","args":[7]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert_eq!(vm.execute(3).unwrap(), 42.0);
    assert_eq!(vm.execute(8).unwrap(), 77.0);
}

#[test]
fn watch_vm_switch_with_default_uses_discriminant_branch_and_final_default() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 1}, {"value": 10}, {"value": 20}, {"value": 30},
        {"func":"SwitchIntegerWithDefault","args":[0,1,2,3]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert_eq!(vm.execute(4).unwrap(), 20.0);

    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 7}, {"value": 10}, {"value": 20}, {"value": 30},
        {"func":"SwitchIntegerWithDefault","args":[0,1,2,3]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert_eq!(vm.execute(4).unwrap(), 30.0);
}

#[test]
fn watch_vm_switch_with_default_matches_explicit_tests_and_uses_default() {
    // The explicit tests are deliberately distinct from their pair indices:
    // this distinguishes test/consequent matching from indexed selection.
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 51},
        {"value": 51}, {"value": 148},
        {"value": 2}, {"value": 95},
        {"value": 312},
        {"func":"SwitchWithDefault","args":[0,1,2,3,4,5]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert_eq!(vm.execute(6).unwrap(), 148.0);

    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 2},
        {"value": 51}, {"value": 148},
        {"value": 2}, {"value": 95},
        {"value": 312},
        {"func":"SwitchWithDefault","args":[0,1,2,3,4,5]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert_eq!(vm.execute(6).unwrap(), 95.0);

    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 7},
        {"value": 51}, {"value": 148},
        {"value": 2}, {"value": 95},
        {"value": 312},
        {"func":"SwitchWithDefault","args":[0,1,2,3,4,5]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert_eq!(vm.execute(6).unwrap(), 312.0);
}

#[test]
fn watch_vm_switch_with_default_matches_connector_shaped_pair() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 2},
        {"value": 0}, {"value": 10},
        {"value": 1}, {"value": 20},
        {"value": 2}, {"value": 95},
        {"value": 312},
        {"func":"SwitchWithDefault","args":[0,1,2,3,4,5,6,7]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert_eq!(vm.execute(8).unwrap(), 95.0);
}

#[test]
fn watch_vm_switch_integer_with_default_remains_indexed() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 1}, {"value": 10}, {"value": 20}, {"value": 30},
        {"func":"SwitchIntegerWithDefault","args":[0,1,2,3]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert_eq!(vm.execute(4).unwrap(), 20.0);
}

#[test]
fn watch_vm_integer_switches_match_exact_branch_indices_and_default_fractional_values() {
    for (name, discriminant, expected) in [
        ("SwitchInteger", 0.0, 10.0),
        ("SwitchInteger", 1.0, 20.0),
        ("SwitchInteger", 1.5, 0.0),
        ("SwitchInteger", 4.5, 0.0),
        ("SwitchIntegerWithDefault", 0.0, 10.0),
        ("SwitchIntegerWithDefault", 1.0, 20.0),
        ("SwitchIntegerWithDefault", 1.5, 99.0),
        ("SwitchIntegerWithDefault", 4.5, 99.0),
    ] {
        let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
            {"value":discriminant},
            {"value":10}, {"value":20}, {"value":99},
            {"func":name,"args":[0,1,2,3]}
        ]))
        .unwrap();
        let mut vm = renderer::runtime::WatchVm::new(&nodes);
        assert_eq!(vm.execute(4).unwrap(), expected, "{name}({discriminant})");
    }
}

#[test]
fn watch_vm_integer_switches_evaluate_only_the_selected_consequent() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value":1},
        {"func":"UnselectedBranchMustRemainLazy"},
        {"value":42},
        {"func":"AnotherUnselectedBranchMustRemainLazy"},
        {"func":"SwitchInteger","args":[0,1,2,3]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert_eq!(vm.execute(4).unwrap(), 42.0);

    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value":4.5},
        {"func":"UnselectedBranchMustRemainLazy"},
        {"func":"AnotherUnselectedBranchMustRemainLazy"},
        {"value":99},
        {"func":"SwitchIntegerWithDefault","args":[0,1,2,3]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert_eq!(vm.execute(4).unwrap(), 99.0);
}

#[test]
fn watch_vm_remap_obeys_sonolus_argument_order() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 0}, {"value": 10}, {"value": 100}, {"value": 200}, {"value": 5},
        {"func":"Remap","args":[0,1,2,3,4]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert_eq!(vm.execute(5).unwrap(), 150.0);
}

#[test]
fn watch_vm_unknown_function_errors_and_invalid_get_returns_zero() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 1000}, {"value": -1}, {"func":"Get","args":[0,1]},
        {"func":"UnrecognizedOperation","args":[]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert_eq!(vm.execute(2).unwrap(), 0.0);
    assert!(format!("{:#}", vm.execute(3).unwrap_err()).contains("unsupported Watch function"));
}

#[test]
fn watch_vm_debug_log_captures_finite_and_nan_values_and_returns_zero() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 123.5}, {"func":"DebugLog","args":[0]},
        {"value": 0}, {"func":"Divide","args":[2,2]},
        {"func":"DebugLog","args":[3]},
        {"value": 1}, {"func":"Divide","args":[5,2]},
        {"func":"DebugLog","args":[6]},
        {"value": -1}, {"func":"Divide","args":[8,2]},
        {"func":"DebugLog","args":[9]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);

    assert_eq!(vm.execute(1).unwrap(), 0.0);
    assert_eq!(vm.execute(4).unwrap(), 0.0);
    assert_eq!(vm.execute(7).unwrap(), 0.0);
    assert_eq!(vm.execute(10).unwrap(), 0.0);
    assert_eq!(vm.debug_events.len(), 4);
    match &vm.debug_events[0] {
        renderer::runtime::DebugEvent::Log {
            value, value_text, ..
        } => {
            assert_eq!(*value, 123.5);
            assert_eq!(value_text, "123.5");
        }
        event => panic!("expected log event, got {event:?}"),
    }
    match &vm.debug_events[1] {
        renderer::runtime::DebugEvent::Log {
            value, value_text, ..
        } => {
            assert!(value.is_nan());
            assert_eq!(value_text, "NaN");
        }
        event => panic!("expected log event, got {event:?}"),
    }
    match &vm.debug_events[2] {
        renderer::runtime::DebugEvent::Log {
            value, value_text, ..
        } => {
            assert_eq!(*value, f64::INFINITY);
            assert_eq!(value_text, "inf");
        }
        event => panic!("expected log event, got {event:?}"),
    }
    match &vm.debug_events[3] {
        renderer::runtime::DebugEvent::Log {
            value, value_text, ..
        } => {
            assert_eq!(*value, f64::NEG_INFINITY);
            assert_eq!(value_text, "-inf");
        }
        event => panic!("expected log event, got {event:?}"),
    }
    let serialized = serde_json::to_string(&vm.debug_events).unwrap();
    assert!(serialized.contains("\"value_text\":\"NaN\""));
    let serialized: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    assert!(serialized[1].get("value").is_none());
}

#[test]
fn watch_vm_debug_log_evaluates_side_effect_argument_once_and_execute_continues() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 5000}, {"value": 0}, {"value": 1},
        {"func":"SetAdd","args":[0,1,2]},
        {"func":"DebugLog","args":[3]},
        {"value": 42}, {"func":"Execute","args":[4,5]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    vm.memory.set(5000, 0, 0.0);

    assert_eq!(vm.execute(6).unwrap(), 42.0);
    assert_eq!(vm.memory.get(5000, 0), 1.0);
    assert_eq!(vm.function_counts.get("SetAdd"), Some(&1));
    assert_eq!(vm.function_counts.get("DebugLog"), Some(&1));
    assert!(matches!(
        vm.debug_events.as_slice(),
        [renderer::runtime::DebugEvent::Log { value: 1.0, .. }]
    ));
}

#[test]
fn watch_vm_debug_events_preserve_order_signed_zero_and_nonblocking_pause() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": -0.0}, {"func":"DebugLog","args":[0]},
        {"func":"DebugPause","args":[]},
        {"func":"Execute","args":[1,2]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);

    assert_eq!(vm.execute(3).unwrap(), 0.0);
    assert_eq!(vm.debug_events.len(), 2);
    match &vm.debug_events[0] {
        renderer::runtime::DebugEvent::Log {
            value, value_text, ..
        } => {
            assert_eq!(*value, 0.0);
            assert!(value.is_sign_negative());
            assert_eq!(value_text, "-0");
        }
        event => panic!("expected log event, got {event:?}"),
    }
    assert!(matches!(
        vm.debug_events[1],
        renderer::runtime::DebugEvent::Pause { .. }
    ));
}

#[test]
fn watch_runtime_collects_debug_events_in_callback_order_across_lifecycle() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "archetypes": [{
            "name":"DebugOracle",
            "spawnTime":{"index":0,"order":0},
            "despawnTime":{"index":1,"order":0},
            "updateSequential":{"index":9,"order":0},
            "updateParallel":{"index":8,"order":0}
        }],
        "nodes": [
            {"value":-1}, {"value":5},
            {"value":2000}, {"value":0}, {"value":321.25},
            {"func":"Set","args":[2,3,4]},
            {"func":"Get","args":[2,3]},
            {"func":"DebugLog","args":[6]},
            {"func":"DebugLog","args":[6]},
            {"func":"Execute","args":[5,7]}
        ]
    }))
    .unwrap();
    let level: renderer::formats::LevelData = serde_json::from_value(serde_json::json!({
        "entities":[{"archetype":"DebugOracle","data":[]}]
    }))
    .unwrap();
    let mut runtime = renderer::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();

    let report = runtime.frame(0.0).unwrap();
    let logs: Vec<_> = report
        .debug_events
        .iter()
        .filter_map(|event| match event {
            renderer::runtime::DebugEvent::Log {
                value,
                callback,
                entity_id,
                ..
            } => Some((*value, callback.as_deref(), *entity_id)),
            renderer::runtime::DebugEvent::Pause { .. } => None,
        })
        .collect();
    assert_eq!(
        logs,
        vec![
            (321.25, Some("UpdateSequential"), Some(0)),
            (321.25, Some("UpdateParallel"), Some(0)),
        ]
    );
}

#[test]
fn watch_vm_spawn_queues_archetype_and_data_then_returns_zero() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value": 7}, {"value": 12.5}, {"value": -3},
        {"func":"Spawn","args":[0,1,2]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert_eq!(vm.execute(3).unwrap(), 0.0);
    assert_eq!(vm.spawn_queue.len(), 1);
    assert_eq!(vm.spawn_queue[0].archetype_id, 7);
    assert_eq!(vm.spawn_queue[0].data, vec![12.5, -3.0]);
}

#[test]
fn supplied_fixture_initialization_preprocess_executes_spawn() {
    let engine =
        renderer::formats::load_engine(std::path::Path::new("Next Sekai Engine.zip")).unwrap();
    let init = engine
        .watch
        .archetypes
        .iter()
        .find(|a| a.name == "Initialization")
        .unwrap();
    let callback = init.preprocess.as_ref().unwrap();
    let entry = callback
        .get("index")
        .and_then(serde_json::Value::as_u64)
        .unwrap() as usize;
    let mut vm = renderer::runtime::WatchVm::new(&engine.watch.nodes);
    vm.context.lifecycle_stage = Some(renderer::watch_runtime::LifecycleStage::Preprocess as u8);
    assert_eq!(vm.execute(entry).unwrap(), 0.0);
    assert_eq!(vm.spawn_queue.len(), 1);
    assert_eq!(vm.spawn_queue[0].archetype_id, 7);
}

#[test]
fn watch_host_maps_named_imports_and_resolves_references_without_index_aliasing() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "archetypes": [{"name":"Thing","imports":[
            {"name":"value","index":2}, {"name":"target","index":5,"def":91}
        ]}], "nodes": []
    }))
    .unwrap();
    let level: renderer::formats::LevelData = serde_json::from_value(serde_json::json!({
        "entities":[
            {"name":"source","archetype":"Thing","data":[
                {"name":"value","value":4.5},{"name":"target","ref":"destination"}
            ]},
            {"name":"destination","archetype":"Thing","data":[{"name":"value","value":9}]}
        ]
    }))
    .unwrap();
    let runtime = renderer::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();
    assert_eq!(runtime.resolved_level_entity_count(), 2);
    assert_eq!(runtime.entities[0].entity_data[2], 4.5);
    assert_eq!(runtime.entities[0].entity_data[5], 1.0);
    assert_eq!(runtime.entities[1].entity_data[2], 9.0);
    assert_eq!(runtime.entities[1].entity_data[5], 91.0);
}

#[test]
fn watch_host_rejects_unknown_archetypes_and_unresolved_imported_references() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "archetypes":[{"name":"Thing","imports":[{"name":"target","index":0}]}],"nodes":[]
    }))
    .unwrap();
    let unknown: renderer::formats::LevelData = serde_json::from_value(serde_json::json!({
        "entities":[{"archetype":"Missing","data":[]}]
    }))
    .unwrap();
    assert!(renderer::watch_runtime::WatchRuntime::new(&watch, &unknown).is_err());
    let bad_ref: renderer::formats::LevelData = serde_json::from_value(serde_json::json!({
        "entities":[{"archetype":"Thing","data":[{"name":"target","ref":"absent"}]}]
    }))
    .unwrap();
    assert!(renderer::watch_runtime::WatchRuntime::new(&watch, &bad_ref).is_err());
}

#[test]
fn watch_host_isolates_entity_memory_and_runs_ordered_lifecycle() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "archetypes":[{"name":"Thing","imports":[{"name":"value","index":0}],
            "initialize":{"index":4,"order":1},"updateSequential":{"index":6,"order":-1}}],
        "nodes":[
            {"value":4001},{"value":0},{"func":"Get","args":[0,1]},
            {"value":4000},{"func":"Set","args":[3,1,2]},
            {"value":1},{"func":"SetAdd","args":[3,1,5]}
        ]
    }))
    .unwrap();
    let level: renderer::formats::LevelData = serde_json::from_value(serde_json::json!({
        "entities":[
            {"archetype":"Thing","data":[{"name":"value","value":5}]},
            {"archetype":"Thing","data":[{"name":"value","value":9}]},
            {"archetype":"Thing","data":[{"name":"value","value":5}]}
        ]
    }))
    .unwrap();
    let mut runtime = renderer::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();
    let report = runtime.frame(0.0).unwrap();
    assert_eq!(runtime.entities[0].memory.get(4000, 0), 6.0);
    assert_eq!(runtime.entities[1].memory.get(4000, 0), 10.0);
    assert_eq!(runtime.entities[2].memory.get(4000, 0), 6.0);
    assert_eq!(
        report
            .callbacks
            .iter()
            .filter(|c| c.stage == renderer::watch_runtime::LifecycleStage::Initialize)
            .count(),
        3
    );
    assert!(
        report
            .callbacks
            .iter()
            .position(|c| c.stage == renderer::watch_runtime::LifecycleStage::Initialize)
            < report
                .callbacks
                .iter()
                .position(|c| c.stage == renderer::watch_runtime::LifecycleStage::UpdateSequential)
    );
}

#[test]
fn watch_host_lifecycle_does_not_repeat_initialize_or_update_inactive_entities() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "archetypes":[{"name":"Thing","preprocess":{"index":2},
            "spawnTime":{"index":0},"despawnTime":{"index":1},
            "initialize":{"index":2},"updateSequential":{"index":3},
            "updateParallel":{"index":4},"terminate":{"index":5}}],
        "nodes":[{"value":0},{"value":1},{"value":42},{"value":43},{"value":44},{"value":45}]
    }))
    .unwrap();
    let level: renderer::formats::LevelData = serde_json::from_value(serde_json::json!({
        "entities":[{"archetype":"Thing","data":[]}]
    }))
    .unwrap();
    let mut runtime = renderer::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();

    let first = runtime.frame(0.0).unwrap();
    assert_eq!(
        first
            .callbacks
            .iter()
            .filter(|c| c.stage == renderer::watch_runtime::LifecycleStage::Initialize)
            .count(),
        1
    );
    let second = runtime.frame(0.0).unwrap();
    assert_eq!(
        second
            .callbacks
            .iter()
            .filter(|c| c.stage == renderer::watch_runtime::LifecycleStage::Initialize)
            .count(),
        0
    );
    assert_eq!(
        second
            .callbacks
            .iter()
            .filter(|c| c.stage == renderer::watch_runtime::LifecycleStage::UpdateSequential)
            .count(),
        1
    );
    let outside = runtime.frame(2.0).unwrap();
    assert_eq!(
        outside
            .callbacks
            .iter()
            .filter(|c| c.stage == renderer::watch_runtime::LifecycleStage::Terminate)
            .count(),
        1
    );
    assert!(!outside.callbacks.iter().any(|c| matches!(
        c.stage,
        renderer::watch_runtime::LifecycleStage::UpdateSequential
            | renderer::watch_runtime::LifecycleStage::UpdateParallel
    )));
    assert!(!runtime.entities[0].active && !runtime.entities[0].initialized);
    let reentered = runtime.frame(0.0).unwrap();
    assert_eq!(
        reentered
            .callbacks
            .iter()
            .filter(|c| c.stage == renderer::watch_runtime::LifecycleStage::Initialize)
            .count(),
        1
    );
    assert!(runtime.entities[0].active && runtime.entities[0].initialized);
}

#[test]
fn watch_host_sets_delta_time_for_backward_seek_from_requested_timestamps() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "archetypes":[{"name":"Thing"}],
        "nodes":[{"value":0}]
    }))
    .unwrap();
    let level: renderer::formats::LevelData = serde_json::from_value(serde_json::json!({
        "entities":[{"archetype":"Thing","data":[]}]
    }))
    .unwrap();
    let mut runtime = renderer::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();
    runtime.frame(12.0).unwrap();
    runtime.frame(3.0).unwrap();

    assert_eq!(runtime.global_memory.get(1001, 0), 3.0);
    assert_eq!(runtime.global_memory.get(1001, 1), 0.0);
}

#[test]
fn watch_host_runs_spawn_and_despawn_time_as_separate_ordered_passes() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "archetypes":[
            {"name":"A","spawnTime":{"index":0,"order":5},"despawnTime":{"index":4,"order":-5}},
            {"name":"B","spawnTime":{"index":2,"order":-5},"despawnTime":{"index":6,"order":5}}
        ],
        "nodes":[
            {"value":0},{"func":"Execute","args":[0,0]},
            {"value":0},{"func":"Execute","args":[0,2]},
            {"value":0},{"func":"Execute","args":[0,4]},
            {"value":0},{"func":"Execute","args":[0,6]}
        ]
    }))
    .unwrap();
    let level: renderer::formats::LevelData = serde_json::from_value(serde_json::json!({
        "entities":[{"archetype":"A","data":[]},{"archetype":"B","data":[]}]
    }))
    .unwrap();
    let mut runtime = renderer::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();
    let report = runtime.frame(0.0).unwrap();
    let schedule_trace = report
        .callbacks
        .iter()
        .filter(|callback| {
            matches!(
                callback.stage,
                renderer::watch_runtime::LifecycleStage::SpawnTime
                    | renderer::watch_runtime::LifecycleStage::DespawnTime
            )
        })
        .map(|callback| (callback.stage, callback.archetype.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        schedule_trace,
        [
            (renderer::watch_runtime::LifecycleStage::SpawnTime, "B"),
            (renderer::watch_runtime::LifecycleStage::SpawnTime, "A"),
            (renderer::watch_runtime::LifecycleStage::DespawnTime, "A"),
            (renderer::watch_runtime::LifecycleStage::DespawnTime, "B"),
        ]
    );
}

#[test]
fn watch_jump_loop_repeats_until_final_branch_and_returns_final_value() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "nodes":[
            {"value":4000},{"value":0},{"value":1},{"func":"SetAdd","args":[0,1,2]},
            {"value":3},{"func":"GreaterOr","args":[3,4]},
            {"value":2},{"value":1},{"func":"If","args":[5,6,7]},
            {"value":0},{"value":99},{"func":"JumpLoop","args":[8,9,10]}
        ]
    }))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&watch.nodes);

    assert_eq!(vm.execute(11).unwrap(), 99.0);
    assert_eq!(vm.memory.get(4000, 0), 3.0);
}

#[test]
fn watch_while_legacy_condition_body_is_pretest_and_returns_last_body_value() {
    use renderer::watch::EngineNode as N;
    let nodes: Vec<N> = serde_json::from_value(serde_json::json!([
        {"value":2000},{"value":0},{"value":1},{"value":3},
        {"func":"Get","args":[0,1]},{"func":"Less","args":[4,3]},
        {"func":"SetAdd","args":[0,1,2]},{"func":"While","args":[5,6]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert_eq!(vm.execute(7).unwrap(), 3.0);
    assert_eq!(vm.memory.get(2000, 0), 3.0);

    vm.memory.set(2000, 0, 3.0);
    assert_eq!(vm.execute(7).unwrap(), 0.0);
    assert_eq!(vm.memory.get(2000, 0), 3.0);
}

#[test]
fn watch_while_single_body_repeats_until_break_and_returns_break_value() {
    use renderer::watch::EngineNode as N;
    let nodes: Vec<N> = serde_json::from_value(serde_json::json!([
        {"value":2000},{"value":0},{"value":1},{"value":3},
        {"func":"SetAdd","args":[0,1,2]},
        {"func":"Get","args":[0,1]},{"func":"GreaterOr","args":[5,3]},
        {"value":1},{"value":99},{"func":"Break","args":[7,8]},
        {"value":0},{"func":"If","args":[6,9,10]},
        {"func":"Execute","args":[4,11]},{"func":"While","args":[12]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    assert_eq!(vm.execute(13).unwrap(), 99.0);
    assert_eq!(vm.memory.get(2000, 0), 3.0);
}

#[test]
fn watch_vm_diagnostics_keep_bounded_loop_branch_and_memory_fingerprint() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "nodes":[
            {"value":4000},{"value":0},{"value":1},{"func":"SetAdd","args":[0,1,2]},
            {"value":3},{"func":"GreaterOr","args":[3,4]},
            {"value":2},{"value":1},{"func":"If","args":[5,6,7]},
            {"value":0},{"value":99},{"func":"JumpLoop","args":[8,9,10]}
        ]
    }))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&watch.nodes);
    assert!(vm.diagnostic_report().is_none());
    vm.enable_diagnostics(24).unwrap();

    assert_eq!(vm.execute(11).unwrap(), 99.0);
    let trace = vm.diagnostic_report().unwrap();
    assert!(trace.contains("JumpLoop node=11"), "{trace}");
    assert!(trace.contains("selected_branch="), "{trace}");
    assert!(trace.contains("branch_node="), "{trace}");
    assert!(
        trace.contains("SetAdd node=3 block=4000 index=0 old="),
        "{trace}"
    );
    assert!(trace.contains("new=3"), "{trace}");
    assert!(trace.contains("bounded capacity 24"), "{trace}");
}

#[test]
fn watch_host_defers_spawn_then_exposes_supplied_memory_to_initialize() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "archetypes":[
            {"name":"Source","preprocess":{"index":2}},
            {"name":"Target","initialize":{"index":7}}
        ],
        "nodes":[
            {"value":1},{"value":12.5},{"func":"Spawn","args":[0,1]},
            {"value":4000},{"value":0},{"func":"Get","args":[3,4]},
            {"value":1},{"func":"Set","args":[3,6,5]}
        ]
    }))
    .unwrap();
    let level: renderer::formats::LevelData = serde_json::from_value(serde_json::json!({
        "entities":[{"archetype":"Source","data":[]}]
    }))
    .unwrap();
    let mut runtime = renderer::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();
    let report = runtime.frame(0.0).unwrap();

    assert_eq!(runtime.entities.len(), 2);
    assert_eq!(runtime.entities[1].archetype, "Target");
    assert!(runtime.entities[1].spawned && runtime.entities[1].initialized);
    assert_eq!(runtime.entities[1].memory.get(4000, 0), 12.5);
    assert_eq!(runtime.entities[1].memory.get(4000, 1), 12.5);
    assert!(report.callbacks.iter().any(|callback| {
        callback.entity_id == Some(1)
            && callback.stage == renderer::watch_runtime::LifecycleStage::Initialize
            && callback.node == 7
    }));
}

#[test]
fn watch_host_backs_entity_memory_views_with_distinct_array_rows() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "archetypes":[{"name":"Thing","imports":[{"name":"value","index":0}],
            "preprocess":{"index":5},"updateSequential":{"index":9}}],
        "nodes":[
            {"value":4001},{"value":0},{"func":"Get","args":[0,1]},
            {"value":4002},{"value":0},{"func":"Set","args":[3,4,2]},
            {"value":4000},{"func":"Get","args":[3,4]},
            {"value":0},{"func":"Set","args":[6,8,7]}
        ]
    }))
    .unwrap();
    let level: renderer::formats::LevelData = serde_json::from_value(serde_json::json!({
        "entities":[
            {"archetype":"Thing","data":[{"name":"value","value":5}]},
            {"archetype":"Thing","data":[{"name":"value","value":9}]}
        ]
    }))
    .unwrap();
    let mut runtime = renderer::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();
    runtime.frame(0.0).unwrap();
    assert_eq!(runtime.entities[0].memory.get(4000, 0), 5.0);
    assert_eq!(runtime.entities[1].memory.get(4000, 0), 9.0);
    let shared = runtime.context.entity_shared_memory_array.read().unwrap();
    assert_eq!(shared[0], 5.0);
    assert_eq!(shared[32], 9.0);
}

#[test]
fn watch_host_populates_entity_info_and_active_array_during_initialize() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "archetypes":[{"name":"Thing","initialize":{"index":5}}],
        "nodes":[
            {"value":4003},{"value":2},{"func":"Get","args":[0,1]},
            {"value":4000},{"value":0},{"func":"Set","args":[3,4,2]}
        ]
    }))
    .unwrap();
    let level: renderer::formats::LevelData = serde_json::from_value(serde_json::json!({
        "entities":[{"archetype":"Thing","data":[]}]
    }))
    .unwrap();
    let mut runtime = renderer::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();
    runtime.frame(0.0).unwrap();
    assert_eq!(runtime.entities[0].memory.get(4000, 0), 1.0);
    assert_eq!(runtime.context.entity_info_array.read().unwrap()[2], 1.0);
}

#[test]
fn watch_host_converts_bpm_boundaries_and_integrates_timescale_changes() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "archetypes":[
            {"name":"#BPM_CHANGE","imports":[{"name":"#BEAT","index":0},{"name":"#BPM","index":1}]},
            {"name":"#TIMESCALE_CHANGE","imports":[{"name":"#BEAT","index":0},{"name":"#TIMESCALE","index":1}]}
        ],
        "nodes":[
            {"value":4},{"func":"BeatToTime","args":[0]},
            {"func":"BeatToBPM","args":[0]},
            {"value":2},{"func":"TimeToTimeScale","args":[3]},
            {"func":"TimeToScaledTime","args":[3]}
        ]
    }))
    .unwrap();
    let level: renderer::formats::LevelData = serde_json::from_value(serde_json::json!({
        "entities":[
            {"archetype":"#BPM_CHANGE","data":[{"name":"#BEAT","value":0},{"name":"#BPM","value":120}]},
            {"archetype":"#BPM_CHANGE","data":[{"name":"#BEAT","value":4},{"name":"#BPM","value":60}]},
            {"archetype":"#TIMESCALE_CHANGE","data":[{"name":"#BEAT","value":2},{"name":"#TIMESCALE","value":0}]},
            {"archetype":"#TIMESCALE_CHANGE","data":[{"name":"#BEAT","value":4},{"name":"#TIMESCALE","value":-1}]}
        ]
    }))
    .unwrap();
    let runtime = renderer::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&watch.nodes);
    vm.context = runtime.context.clone();

    assert_eq!(vm.execute(1).unwrap(), 2.0);
    assert_eq!(vm.execute(2).unwrap(), 60.0);
    assert_eq!(runtime.context.time_to_timescale(0.5).unwrap(), 1.0);
    assert_eq!(runtime.context.time_to_timescale(1.5).unwrap(), 0.0);
    assert_eq!(runtime.context.time_to_timescale(2.0).unwrap(), -1.0);
    assert_eq!(runtime.context.scaled_time(2.0).unwrap(), 1.0);
    assert_eq!(runtime.context.scaled_time(3.0).unwrap(), 0.0);
}

#[test]
fn watch_host_applies_undeclared_sonolus_timing_archetypes() {
    let watch: renderer::watch::WatchData = serde_json::from_value(serde_json::json!({
        "archetypes":[{"name":"Stage"}],
        "nodes":[{"value":0}]
    }))
    .unwrap();
    let level: renderer::formats::LevelData = serde_json::from_value(serde_json::json!({
        "entities":[
            {"archetype":"#BPM_CHANGE","data":[
                {"name":"#BEAT","value":0},{"name":"#BPM","value":120}]},
            {"archetype":"#BPM_CHANGE","data":[
                {"name":"#BEAT","value":4},{"name":"#BPM","value":60}]},
            {"archetype":"#TIMESCALE_CHANGE","data":[
                {"name":"#BEAT","value":2},{"name":"#TIMESCALE","value":0}]},
            {"archetype":"Stage","data":[]}
        ]
    }))
    .unwrap();

    let runtime = renderer::watch_runtime::WatchRuntime::new(&watch, &level).unwrap();
    assert_eq!(runtime.entities.len(), 4);
    assert_eq!(runtime.entities[0].archetype, "#BPM_CHANGE");
    assert_eq!(runtime.entities[2].archetype, "#TIMESCALE_CHANGE");
    assert_eq!(runtime.context.bpm_map, vec![(0.0, 120.0), (4.0, 60.0)]);
    assert_eq!(runtime.context.timescale_map, vec![(1.0, 0.0)]);
    assert_eq!(runtime.context.time_to_timescale(0.5).unwrap(), 1.0);
    assert_eq!(runtime.context.time_to_timescale(1.0).unwrap(), 0.0);
}

#[test]
fn watch_vm_local_entity_block_bounds_do_not_alias_adjacent_rows() {
    let nodes: Vec<renderer::watch::EngineNode> = serde_json::from_value(serde_json::json!([
        {"value":4001},{"value":32},{"func":"Get","args":[0,1]},
        {"value":99},{"func":"Set","args":[0,1,3]},
        {"value":4002},{"func":"Get","args":[5,1]},
        {"func":"Set","args":[5,1,3]},
        {"value":4101},{"value":64},{"func":"Set","args":[8,9,3]}
    ]))
    .unwrap();
    let mut vm = renderer::runtime::WatchVm::new(&nodes);
    vm.context.entity_id = Some(0);
    vm.context.has_entity_data = true;
    vm.context.has_entity_shared_memory = true;
    vm.context.lifecycle_stage = Some(renderer::watch_runtime::LifecycleStage::Preprocess as u8);
    vm.context.entity_data_array = std::sync::Arc::new(std::sync::RwLock::new(
        vec![11.0; 32].into_iter().chain(vec![22.0; 32]).collect(),
    ));
    vm.context.entity_shared_memory_array = std::sync::Arc::new(std::sync::RwLock::new(
        vec![33.0; 32].into_iter().chain(vec![44.0; 32]).collect(),
    ));

    assert_eq!(vm.execute(2).unwrap(), 0.0);
    assert!(vm.execute(4).is_err());
    assert_eq!(vm.context.entity_data_array.read().unwrap()[32], 22.0);

    assert_eq!(vm.execute(6).unwrap(), 0.0);
    assert!(vm.execute(7).is_err());
    assert_eq!(
        vm.context.entity_shared_memory_array.read().unwrap()[32],
        44.0
    );
    assert!(vm.execute(10).is_err());
    assert_eq!(vm.context.entity_data_array.read().unwrap().len(), 64);
}
