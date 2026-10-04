//! Focused compositing observations; this module never runs in production.
use super::*;
use crate::{gpu_render, render::RenderBackend};

#[test]
#[ignore = "requires private DISPLAYHOLIC media; writes audit artifacts"]
fn displayholic_mv_alpha_audit() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let media =
        std::path::Path::new("C:/Users/Admin/Desktop/Charts/In Progress/ABM - DISPLAYHOLIC");
    let out = root.join("artifacts/mv-alpha");
    std::fs::create_dir_all(&out).unwrap();
    let package =
        formats::load_engine(&root.join("TestingSuite/Next RUSH/engine/Next RUSH.zip")).unwrap();
    let level = formats::load_level(&media.join("DISPLAYHOLIC AUDIO.json.gz")).unwrap();
    let resources = root.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp");
    let defaults: BTreeMap<_, _> = package.metadata.resource_defaults().into_iter().collect();
    let time = std::env::var("SONO_MV_AUDIT_TIME")
        .ok()
        .map(|s| s.parse::<f64>().unwrap())
        .unwrap_or(5.0);
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
        RenderBackend::Wgpu,
        false,
        true,
    )
    .unwrap();
    let prepared = session
        .prepare_global_frame((time * 60.0).ceil() as u64)
        .unwrap();
    let mut observations = Vec::new();
    let diagnostics = prepared
        .report
        .display_list
        .skin_render_diagnostics_with_runtime_transform(
            640,
            360,
            640.0 / 360.0,
            &session.render.skin,
            &session.render.bindings,
            &prepared.report.runtime_skin_transform,
        )
        .unwrap();
    std::fs::write(
        out.join("draws.json"),
        serde_json::to_vec_pretty(&diagnostics).unwrap(),
    )
    .unwrap();
    let tools = crate::export_mv::tests::tools().unwrap();
    for percent in [None, Some(100.0), Some(50.0), Some(0.0)] {
        let label = percent.map_or("no-mv".to_owned(), |p| format!("mv-{p}"));
        let mut frame = prepared.clone();
        if let Some(percent) = percent {
            session
                .configure_mv(
                    &tools,
                    &media.join("DISPLAYHOLIC MV.mp4"),
                    crate::export_timeline::ExportTimeline::new(
                        time,
                        FrameRange::new(time, 1.0 / 60.0, 60).unwrap(),
                        level.bgm_offset.unwrap_or(0.0),
                    )
                    .unwrap(),
                )
                .unwrap();
            session.set_mv_background(percent);
            frame.mv_frame = session.mv.as_mut().unwrap().at_watch_time(time).unwrap();
            assert!(frame.mv_frame.is_some());
        }
        let decoded_alpha = frame
            .mv_frame
            .as_ref()
            .map(|f| f.assets.rgba.chunks_exact(4).map(|p| p[3]).min().unwrap());
        for backend in [RenderBackend::Wgpu, RenderBackend::Cpu] {
            std::sync::Arc::get_mut(&mut session.render)
                .unwrap()
                .backend = backend;
            let mut bg_only = frame.clone();
            bg_only.report.display_list.sprites.clear();
            bg_only.particle_draws = None;
            bg_only.ui_values.clear();
            let (bg, bg_rgba) = gpu_render::capture_rgba(|| {
                render_prepared_frame(session.render.clone(), false, bg_only).unwrap()
            });
            let (rendered, rgba) = gpu_render::capture_rgba(|| {
                render_prepared_frame(session.render.clone(), false, frame.clone()).unwrap()
            });
            if backend == RenderBackend::Wgpu {
                assert!(bg_rgba.chunks_exact(4).all(|p| p[3] == 255));
                assert!(rgba.chunks_exact(4).all(|p| p[3] == 255));
            }
            let mut probes = Vec::new();
            for (name, x, y) in [("far", 320, 72), ("middle", 320, 180), ("near", 320, 280)] {
                let at = (y * 640 + x) as usize * 3;
                let mut target: [u8; 3] = bg.rgb[at..at + 3].try_into().unwrap();
                let mut order: Vec<_> = frame.report.display_list.sprites.iter().collect();
                order.sort_by(|a, b| crate::runtime::compare_z_tuples(&a.z, &b.z));
                let mut blends = Vec::new();
                for draw in order {
                    let sprite = &session.render.bindings[&draw.sprite_id];
                    if let Some(source) = crate::runtime::skin_sample_at_pixel(
                        draw,
                        &session.render.skin,
                        sprite,
                        &frame.report.runtime_skin_transform,
                        640,
                        360,
                        x,
                        y,
                    ) {
                        let a = f64::from(source[3]) / 255.0 * draw.alpha.clamp(0.0, 1.0);
                        let destination = target;
                        for c in 0..3 {
                            target[c] = (f64::from(target[c]) * (1.0 - a)
                                + f64::from(source[c]) * a)
                                .round() as u8;
                        }
                        if a > 0.0 {
                            blends.push(serde_json::json!({"sprite":sprite,"source_rgba":source,"draw_alpha":draw.alpha,"effective_alpha":a,"destination_rgb":destination,"result_rgb":target}));
                        }
                    }
                }
                probes.push(serde_json::json!({"name":name,"pixel":[x,y],"background_rgb":&bg.rgb[at..at+3],"final_rgb":&rendered.rgb[at..at+3],"blends":blends}));
            }
            std::fs::write(
                out.join(format!("{label}-{backend:?}.ppm")),
                [format!("P6\n640 360\n255\n").as_bytes(), &rendered.rgb].concat(),
            )
            .unwrap();
            observations.push(serde_json::json!({"variant":label,"backend":format!("{backend:?}"),"time":time,"draw_sha1":crate::offline::hash_rgb(&serde_json::to_vec(&frame.report.display_list).unwrap()),"decoded_alpha_min":decoded_alpha,"pts":rendered.mv_media_pts,"background_alpha_range":if bg_rgba.is_empty(){None}else{Some([bg_rgba.chunks_exact(4).map(|p|p[3]).min().unwrap(),bg_rgba.chunks_exact(4).map(|p|p[3]).max().unwrap()])},"frame_alpha_range":if rgba.is_empty(){None}else{Some([rgba.chunks_exact(4).map(|p|p[3]).min().unwrap(),rgba.chunks_exact(4).map(|p|p[3]).max().unwrap()])},"probes":probes}));
        }
    }
    std::fs::write(
        out.join("observations.json"),
        serde_json::to_vec_pretty(&observations).unwrap(),
    )
    .unwrap();
}
