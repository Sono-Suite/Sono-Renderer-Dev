//! Analytic skin mode coverage and private-fixture Draw observations.
use super::*;
use sha1::Digest;

#[test]
fn tapered_draw_gradient_follows_documented_bilinear_coordinates() {
    let mut transform = [[0.0; 8]; 8];
    for (i, row) in transform.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    let sprite = formats::SkinSpriteAsset {
        x: 0,
        y: 0,
        width: 1,
        height: 256,
        transform,
    };
    let skin = formats::SkinAssets {
        width: 1,
        height: 256,
        interpolation: true,
        rgba: (0u16..256).flat_map(|a| [255, 255, 255, a as u8]).collect(),
        sprites: BTreeMap::from([("gradient".to_owned(), sprite)]),
    };
    let draw = crate::runtime::SpriteDraw {
        sprite_id: 0,
        corners: [[-1.0, -0.8], [-0.0625, 0.8], [0.0625, 0.8], [1.0, -0.8]],
        z: [0.0; 4],
        alpha: 1.0,
        provenance: None,
        trace: None,
    };
    let list = crate::runtime::DisplayList {
        sprites: vec![draw.clone()],
    };
    let matrix = std::array::from_fn(|i| if i % 5 == 0 { 1.0 } else { 0.0 });
    let bindings = BTreeMap::from([(0, "gradient".to_owned())]);
    let cpu = list
        .render_skin_rgb_with_runtime_transform_and_background(
            128, 128, 1.0, &skin, &bindings, &matrix, None,
        )
        .unwrap();
    let explicit_standard = list
        .render_skin_rgb_with_mode(
            128,
            128,
            1.0,
            &skin,
            &bindings,
            &matrix,
            None,
            crate::skin_render_mode::SkinRenderMode::Standard,
        )
        .unwrap();
    assert_eq!(
        cpu, explicit_standard,
        "Standard API must retain the existing pixels"
    );
    let lightweight = list
        .render_skin_rgb_with_mode(
            128,
            128,
            1.0,
            &skin,
            &bindings,
            &matrix,
            None,
            crate::skin_render_mode::SkinRenderMode::Lightweight,
        )
        .unwrap();
    let lightweight_gpu = crate::gpu_render::render_display_list_rgb_with_skin_mode(
        crate::gpu_render::GpuRenderer::shared().unwrap(),
        &list,
        128,
        128,
        1.0,
        &skin,
        crate::gpu_render::atlas_identity(&skin.rgba, skin.width, skin.height),
        &bindings,
        &matrix,
        None,
        None,
        None,
        crate::skin_render_mode::SkinRenderMode::Lightweight,
    )
    .unwrap();
    assert_ne!(
        cpu, lightweight,
        "Lightweight must exercise distinct interpolation"
    );
    let gpu = crate::gpu_render::render_display_list_rgb(
        crate::gpu_render::GpuRenderer::shared().unwrap(),
        &list,
        128,
        128,
        1.0,
        &skin,
        crate::gpu_render::atlas_identity(&skin.rgba, skin.width, skin.height),
        &bindings,
        &matrix,
        None,
        None,
        None,
    )
    .unwrap();
    for py in [16u32, 40, 64, 88, 112] {
        // This trapezoid has horizontal near/far edges. Forward bilinear y is
        // -0.8 + 1.6*v, so solve v analytically without the renderer's inverse.
        let screen_y = 1.0 - (f64::from(py) + 0.5) / 64.0;
        let v = (screen_y + 0.8) / 1.6;
        let expected_alpha = ((1.0 - v) * 256.0 - 0.5).clamp(0.0, 255.0).round() as u8;
        let at = ((py * 128 + 64) * 3) as usize;
        assert_eq!(&cpu[at..at + 3], &[expected_alpha; 3]);
        assert!(gpu[at..at + 3]
            .iter()
            .all(|c| c.abs_diff(expected_alpha) <= 1));
        // Independently solve the homography on the trapezoid's center line:
        // v=t/(16*(1-t)+t), where t is the screen-height fraction from BL to TL.
        let projective_v = v / (16.0 * (1.0 - v) + v);
        let expected_lightweight = ((1.0 - projective_v) * 256.0 - 0.5)
            .clamp(0.0, 255.0)
            .round() as u8;
        assert_eq!(&lightweight[at..at + 3], &[expected_lightweight; 3]);
        assert!(lightweight_gpu[at..at + 3]
            .iter()
            .all(|c| c.abs_diff(expected_lightweight) <= 1));
    }
    assert!(lightweight
        .iter()
        .zip(&lightweight_gpu)
        .all(|(a, b)| a.abs_diff(*b) <= 2));
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/dynamic-stage");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(
        out.join("synthetic-current.ppm"),
        [b"P6\n128 128\n255\n".as_slice(), &cpu].concat(),
    )
    .unwrap();
}

#[test]
#[ignore = "requires private DISPLAYHOLIC level; writes dynamic-stage trace"]
fn displayholic_dynamic_stage_draw_trace() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = root.join("artifacts/dynamic-stage");
    std::fs::create_dir_all(&out).unwrap();
    let package =
        formats::load_engine(&root.join("TestingSuite/Next RUSH/engine/Next RUSH.zip")).unwrap();
    let level = formats::load_level(std::path::Path::new(
        "C:/Users/Admin/Desktop/Charts/In Progress/ABM - DISPLAYHOLIC/DISPLAYHOLIC AUDIO.json.gz",
    ))
    .unwrap();
    let resources = root.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp");
    let defaults: BTreeMap<_, _> = package.metadata.resource_defaults().into_iter().collect();
    let mut session = FrameSession::new_configured(
        &package.watch,
        &package.rom,
        &package.configuration,
        &level,
        &resources,
        defaults.get("skins").unwrap(),
        defaults.get("backgrounds").map(String::as_str),
        defaults.get("effects").map(String::as_str),
        defaults.get("particles").map(String::as_str),
        640,
        360,
        60,
        &[],
        true,
        crate::render_ui::RendererUiConfig::default(),
        0.0,
        20.2,
        crate::render::RenderBackend::Cpu,
        false,
        true,
    )
    .unwrap();
    session.stepper.runtime_mut().set_draw_tracing(false);
    session.prepare_global_frame(299).unwrap();
    session.stepper.runtime_mut().set_draw_tracing(true);
    let frame = session.prepare_global_frame(300).unwrap();
    use crate::skin_render_mode::{SkinRenderMode, SkinRenderModePreference};
    assert_eq!(
        session.skin_render_modes(),
        (
            SkinRenderModePreference::Lightweight,
            SkinRenderMode::Lightweight
        )
    );
    let mut rendered_modes = Vec::new();
    let mut cpu_modes = Vec::new();
    for mode in [SkinRenderMode::Standard, SkinRenderMode::Lightweight] {
        let mut resources = (*session.render).clone();
        resources.skin_mode = mode;
        let cpu =
            render_prepared_frame(std::sync::Arc::new(resources.clone()), false, frame.clone())
                .unwrap()
                .rgb;
        resources.backend = crate::render::RenderBackend::Wgpu;
        resources.gpu = Some(crate::gpu_render::GpuRenderer::shared().unwrap());
        resources.skin_atlas_identity = Some(crate::gpu_render::atlas_identity(
            &resources.skin.rgba,
            resources.skin.width,
            resources.skin.height,
        ));
        resources.background_atlas_identity = resources
            .background
            .as_ref()
            .map(|a| crate::gpu_render::atlas_identity(&a.rgba, a.width, a.height));
        resources.particle_atlas_identity = resources
            .particles
            .as_ref()
            .map(|a| crate::gpu_render::atlas_identity(&a.rgba, a.width, a.height));
        let gpu = render_prepared_frame(std::sync::Arc::new(resources), false, frame.clone())
            .unwrap()
            .rgb;
        let max_difference = cpu
            .iter()
            .zip(&gpu)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        let channels_over_two = cpu
            .iter()
            .zip(&gpu)
            .filter(|(a, b)| a.abs_diff(**b) > 2)
            .count();
        for (backend, rgb) in [("cpu", &cpu), ("wgpu", &gpu)] {
            std::fs::write(
                out.join(format!("displayholic-{mode:?}-{backend}.ppm")),
                [b"P6\n640 360\n255\n".as_slice(), rgb].concat(),
            )
            .unwrap();
        }
        rendered_modes.push(
            serde_json::json!({"mode":mode,"max_cpu_gpu_difference":max_difference,
            "channels_over_two":channels_over_two,"cpu_sha1":hex::encode(sha1::Sha1::digest(&cpu)),
            "gpu_sha1":hex::encode(sha1::Sha1::digest(&gpu))}),
        );
        assert!(
            max_difference <= 3,
            "{mode:?} CPU/WGPU diverged: {max_difference}"
        );
        cpu_modes.push(cpu);
    }
    assert_ne!(cpu_modes[0], cpu_modes[1]);
    let mut boundary_draws = Vec::new();
    for draw in &frame.report.display_list.sprites {
        let name = &session.render.bindings[&draw.sprite_id];
        let list = crate::runtime::DisplayList {
            sprites: vec![draw.clone()],
        };
        let cpu = list
            .render_skin_rgb_with_mode(
                640,
                360,
                640.0 / 360.0,
                &session.render.skin,
                &session.render.bindings,
                &frame.report.runtime_skin_transform,
                None,
                SkinRenderMode::Lightweight,
            )
            .unwrap();
        let gpu = crate::gpu_render::render_display_list_rgb_with_skin_mode(
            crate::gpu_render::GpuRenderer::shared().unwrap(),
            &list,
            640,
            360,
            640.0 / 360.0,
            &session.render.skin,
            crate::gpu_render::atlas_identity(
                &session.render.skin.rgba,
                session.render.skin.width,
                session.render.skin.height,
            ),
            &session.render.bindings,
            &frame.report.runtime_skin_transform,
            None,
            None,
            None,
            SkinRenderMode::Lightweight,
        )
        .unwrap();
        for (x, y) in [(368, 28), (290, 115)] {
            let at = ((y * 640 + x) * 3) as usize;
            if cpu[at..at + 3] != gpu[at..at + 3] {
                boundary_draws.push(
                    serde_json::json!({"pixel":[x,y],"sprite":name,"corners":draw.corners,
                    "cpu":cpu[at..at+3],"gpu":gpu[at..at+3]}),
                );
            }
        }
    }
    std::fs::write(
        out.join("lightweight-boundary-diagnostics.json"),
        serde_json::to_vec_pretty(&boundary_draws).unwrap(),
    )
    .unwrap();
    assert!(
        boundary_draws.is_empty(),
        "Thin-quad coverage must agree across backends"
    );
    for mode in [SkinRenderMode::Standard, SkinRenderMode::Lightweight] {
        let mut resources = (*session.render).clone();
        resources.skin_mode = mode;
        let mut stage_frame = frame.clone();
        stage_frame.particle_draws = None;
        stage_frame.report.display_list.sprites.retain(|d| {
            d.provenance.as_ref().and_then(|p| p.archetype.as_deref()) == Some("Stage")
        });
        let rgb = render_prepared_frame(std::sync::Arc::new(resources), false, stage_frame)
            .unwrap()
            .rgb;
        std::fs::write(
            out.join(format!("stage-only-{mode:?}.ppm")),
            [b"P6\n640 360\n255\n".as_slice(), &rgb].concat(),
        )
        .unwrap();
    }
    std::fs::write(
        out.join("mode-render-comparison.json"),
        serde_json::to_vec_pretty(&rendered_modes).unwrap(),
    )
    .unwrap();
    let draws: Vec<_> = frame
        .report
        .display_list
        .sprites
        .iter()
        .filter(|d| d.provenance.as_ref().and_then(|p| p.archetype.as_deref()) == Some("Stage"))
        .collect();
    assert!(!draws.is_empty());
    std::fs::write(
        out.join("raw-stage-draws.json"),
        serde_json::to_vec_pretty(&draws).unwrap(),
    )
    .unwrap();
    std::fs::write(
        out.join("state.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "watch_time":5.0, "skin_name":defaults.get("skins"), "bindings":session.render.bindings,
            "runtime_skin_transform":frame.report.runtime_skin_transform,
            "rendering_mode":package.watch.skin.get("renderMode"),
            "requested_skin_mode":session.render.requested_skin_mode,
            "effective_skin_mode":session.render.skin_mode,
        }))
        .unwrap(),
    )
    .unwrap();
    let mut samples = Vec::new();
    for draw in draws {
        let name = &session.render.bindings[&draw.sprite_id];
        if ![
            "Sekai Lane Background",
            "Sekai Stage Border",
            "Sekai Lane Divider",
        ]
        .contains(&name.as_str())
        {
            continue;
        }
        let sprite = &session.render.skin.sprites[name];
        let mut pixels = Vec::new();
        for y in sprite.y..sprite.y + sprite.height {
            for x in sprite.x..sprite.x + sprite.width {
                let at = ((y * session.render.skin.width + x) * 4) as usize;
                pixels.extend_from_slice(&session.render.skin.rgba[at..at + 4]);
            }
        }
        std::fs::write(out.join(format!("{name}.rgba")), pixels).unwrap();
        for (label, x, y) in [("far", 320, 72), ("middle", 320, 180), ("near", 320, 280)] {
            samples.push(serde_json::json!({"draw_node":draw.provenance.as_ref().unwrap().draw_node,"corners":draw.corners,
                "sprite":name,"label":label,"pixel":[x,y],
                "actual_cpu_sample":crate::runtime::skin_sample_at_pixel(draw,&session.render.skin,name,
                    &frame.report.runtime_skin_transform,640,360,x,y),
                "effective_mode_sample":crate::runtime::skin_sample_at_pixel_in_mode(draw,&session.render.skin,name,
                    &frame.report.runtime_skin_transform,640,360,x,y,session.render.skin_mode)}));
        }
    }
    std::fs::write(
        out.join("actual-samples.json"),
        serde_json::to_vec_pretty(&samples).unwrap(),
    )
    .unwrap();
}
