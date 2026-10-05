//! Reconstruct diagnostic SFX PCM from captured host events, without executing Watch.
use anyhow::{Context, Result};
use clap::Parser;
use renderer::{
    audio, formats, offline,
    runtime::{ScheduledEffect, ScheduledLoopedEffect, ScheduledLoopedEffectStop},
};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
};

#[derive(Parser)]
struct Options {
    engine: PathBuf,
    resources: PathBuf,
    effect_name: String,
    trace: PathBuf,
    output: PathBuf,
    #[arg(long)]
    start: f64,
    #[arg(long)]
    duration: f64,
    /// Diagnostic control: omit captured loop events from the comparison PCM.
    #[arg(long)]
    omit_loops: bool,
}
fn main() -> Result<()> {
    let opt = Options::parse();
    let engine = formats::load_engine(&opt.engine)?;
    let bindings = offline::effect_clip_bindings(&engine.watch)?;
    let assets = formats::load_effect_assets(&opt.resources, &opt.effect_name)?;
    let mut scheduled = Vec::<ScheduledEffect>::new();
    let mut starts = Vec::<ScheduledLoopedEffect>::new();
    let mut stops = Vec::<ScheduledLoopedEffectStop>::new();
    for line in BufReader::new(std::fs::File::open(opt.trace)?).lines() {
        let v: Value = serde_json::from_str(&line?)?;
        if v["kind"] != "audio" {
            continue;
        }
        let event = v["data"]["event"].clone();
        match v["operation"].as_str() {
            Some("PlayScheduled") => scheduled.push(serde_json::from_value(event)?),
            Some("PlayLoopedScheduled") if !opt.omit_loops => {
                starts.push(serde_json::from_value(event)?)
            }
            Some("StopLoopedScheduled") if !opt.omit_loops => {
                stops.push(serde_json::from_value(event)?)
            }
            _ => {}
        }
    }
    let mix = audio::mix_effects_wav(
        &assets,
        &bindings,
        &[],
        &scheduled,
        &starts,
        &stops,
        opt.start,
        opt.duration,
    )?;
    std::fs::write(opt.output, mix.wav.context("no playable captured events")?)?;
    println!(
        "captured events: {} one-shots, {} loop starts, {} stops; warnings: {:?}",
        scheduled.len(),
        starts.len(),
        stops.len(),
        mix.warnings
    );
    Ok(())
}
