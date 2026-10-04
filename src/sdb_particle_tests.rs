//! Private-corpus particle diagnostics; no alternate production semantics.
use super::*;

#[test]
#[ignore = "writes particle differential evidence from the private Horizon corpus"]
fn horizon_sdb_particle_value_ledger() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = root.join("artifacts/sdb-particle");
    std::fs::create_dir_all(&out).unwrap();
    let package = formats::load_engine(&root.join("TestingSuite/Horizon/Horizon.zip")).unwrap();
    let level = formats::load_level(&root.join("TestingSuite/Horizon/dreamer.json.gz")).unwrap();
    let resources = root.join("TestingSuite/Horizon/project(1).scp");
    let defaults: BTreeMap<_, _> = package.metadata.resource_defaults().into_iter().collect();
    let mut session = FrameSession::new_configured(
        &package.watch,
        &package.rom,
        &package.configuration,
        &level,
        &resources,
        defaults.get("skins").unwrap(),
        defaults.get("backgrounds").map(String::as_str),
        None,
        defaults.get("particles").map(String::as_str),
        640,
        360,
        60,
        &[],
        true,
        crate::render_ui::RendererUiConfig::default(),
        0.0,
        16.0,
        crate::render::RenderBackend::Cpu,
        false,
        false,
    )
    .unwrap();
    let assets = session.render.particles.as_ref().unwrap().clone();
    std::fs::write(
        out.join("effects.json"),
        serde_json::to_vec_pretty(&assets.effects).unwrap(),
    )
    .unwrap();
    let sprite = &assets.sprites[14];
    let mut crop = Vec::new();
    for y in sprite.y..sprite.y + sprite.height {
        for x in sprite.x..sprite.x + sprite.width {
            let at = ((y * assets.width + x) * 4) as usize;
            crop.extend_from_slice(&assets.rgba[at..at + 4]);
        }
    }
    std::fs::write(out.join("sprite14.rgba"), crop).unwrap();
    let mut ledger = Vec::new();
    // Match the historical single-snapshot cyan-diamond probe, then advance
    // two adjacent frames. This deliberately does not claim stepped parity.
    for index in [894, 895, 896] {
        let report = session
            .stepper
            .runtime_mut()
            .frame(f64::from(index) / 60.0)
            .unwrap();
        let instances: Vec<_> = session
            .stepper
            .runtime_mut()
            .context
            .particle_instances
            .read()
            .unwrap()
            .instances()
            .values()
            .cloned()
            .collect();
        let draws = crate::particles::render_instances(
            &assets,
            &session.render.particle_bindings,
            &instances,
            report.timeline.unwrap(),
        )
        .unwrap();
        eprintln!(
            "frame {index}: {} live instances, {} draws; bindings {:?}",
            instances.len(),
            draws.len(),
            session.render.particle_bindings
        );
        let lane = draws.iter().find(|d| {
            d.sprite_id == 14
                && instances.iter().any(|i| {
                    i.instance_id == d.order.0
                        && session
                            .render
                            .particle_bindings
                            .get(&i.effect_id)
                            .map(String::as_str)
                            == Some("#LANE_LINEAR")
                })
        });
        let Some(lane) = lane else {
            ledger.push(serde_json::json!({"frame":index,"time":report.timeline,
                "events":report.particle_events,"instances":instances,"draw_count":draws.len()}));
            continue;
        };
        let mut rgb = vec![0; 640 * 360 * 3];
        crate::runtime::DisplayList::composite_particle_sprites(
            &mut rgb,
            640,
            360,
            640.0 / 360.0,
            &assets,
            std::slice::from_ref(lane),
            &report.runtime_particle_transform,
        )
        .unwrap();
        let empty_skin = formats::SkinAssets {
            width: 1,
            height: 1,
            interpolation: false,
            rgba: vec![0; 4],
            sprites: BTreeMap::new(),
        };
        let gpu_rgb = crate::gpu_render::render_display_list_rgb(
            crate::gpu_render::GpuRenderer::shared().unwrap(),
            &crate::runtime::DisplayList::default(),
            640,
            360,
            640.0 / 360.0,
            &empty_skin,
            crate::gpu_render::atlas_identity(&empty_skin.rgba, 1, 1),
            &BTreeMap::new(),
            &report.runtime_particle_transform,
            None,
            Some((
                &assets,
                std::slice::from_ref(lane),
                &report.runtime_particle_transform,
                crate::gpu_render::atlas_identity(&assets.rgba, assets.width, assets.height),
            )),
            None,
        )
        .unwrap();
        let max_cpu_gpu_difference = rgb
            .iter()
            .zip(&gpu_rgb)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(
            max_cpu_gpu_difference <= 2,
            "isolated particle CPU/WGPU mismatch: {max_cpu_gpu_difference}"
        );
        std::fs::write(out.join(format!("isolated-{index}.rgb")), rgb).unwrap();
        std::fs::write(out.join(format!("isolated-gpu-{index}.rgb")), gpu_rgb).unwrap();
        ledger.push(serde_json::json!({
            "frame":index,"time":report.timeline,"events":report.particle_events,
            "max_cpu_gpu_difference":max_cpu_gpu_difference,
            "instances":instances,"bindings":session.render.particle_bindings,
            "runtime_particle_transform":report.runtime_particle_transform,
            "sprite":sprite,"atlas_dimensions":[assets.width,assets.height],
            "interpolation":assets.interpolation,"draw_count":draws.len(),
            "selected_draw":{"sprite":lane.sprite_id,"corners":lane.corners,
                "final_corners":crate::runtime::gpu_transform_particle_corners(lane,&report.runtime_particle_transform),
                "alpha":lane.alpha,"color":lane.color,"order":lane.order},
        }));
    }
    assert!(
        ledger
            .iter()
            .any(|entry| entry.get("selected_draw").is_some()),
        "no representative lane particle recorded"
    );
    std::fs::write(
        out.join("ledger.json"),
        serde_json::to_vec_pretty(&ledger).unwrap(),
    )
    .unwrap();
}
