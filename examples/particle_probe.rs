//! Bounded particle diagnostics on the production, stepped Watch traversal.
use anyhow::{Context, Result};
use clap::Parser;
use renderer::{
    formats, offline,
    render::RenderBackend,
    watch_diagnostics::{TraceConfig, WatchTrace},
};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Parser)]
struct Options {
    engine: PathBuf,
    level: PathBuf,
    resources: PathBuf,
    output: PathBuf,
    #[arg(long)]
    time: f64,
    #[arg(long, default_value_t = 60)]
    fps: u32,
    #[arg(long, default_value_t = 640)]
    width: u32,
    #[arg(long, default_value_t = 360)]
    height: u32,
    #[arg(long)]
    wgpu: bool,
    #[arg(long)]
    trace_config: PathBuf,
    #[arg(long)]
    trace_output: PathBuf,
}
fn main() -> Result<()> {
    let opt = Options::parse();
    anyhow::ensure!(
        opt.time.is_finite() && opt.time >= 0.0 && opt.fps > 0,
        "invalid time/fps"
    );
    let engine = formats::load_engine(&opt.engine)?;
    let level = formats::load_level(&opt.level)?;
    let names: BTreeMap<_, _> = engine.metadata.resource_defaults().into_iter().collect();
    let trace = WatchTrace::new(serde_json::from_slice::<TraceConfig>(&std::fs::read(
        opt.trace_config,
    )?)?)?;
    trace.lock().unwrap().particle_names = offline::particle_effect_bindings(&engine.watch)?;
    let mut ui = renderer::render_ui::RendererUiConfig::default();
    ui.enabled = false;
    let mut session = offline::FrameSession::new_configured(
        &engine.watch,
        &engine.rom,
        &engine.configuration,
        &level,
        &opt.resources,
        names.get("skins").context("engine has no default skin")?,
        names.get("backgrounds").map(String::as_str),
        names.get("effects").map(String::as_str),
        names.get("particles").map(String::as_str),
        opt.width,
        opt.height,
        opt.fps,
        &[],
        true,
        ui,
        0.0,
        opt.time + 1.0,
        if opt.wgpu {
            RenderBackend::Wgpu
        } else {
            RenderBackend::Cpu
        },
        true,
        false,
    )?;
    session.set_execution_trace(Some(trace.clone()));
    let frame = session.render_global_frame((opt.time * f64::from(opt.fps)).round() as u64)?;
    let mut ppm = format!("P6\n{} {}\n255\n", opt.width, opt.height).into_bytes();
    ppm.extend_from_slice(&frame.rgb);
    std::fs::write(opt.output, ppm)?;
    trace.lock().unwrap().write_jsonl(&opt.trace_output)?;
    println!(
        "time={:?}, RGB SHA1={}, profile={:?}",
        frame.report.timeline,
        offline::hash_rgb(&frame.rgb),
        frame.profile
    );
    Ok(())
}
