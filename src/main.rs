use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use renderer::{compatibility, formats};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "renderer", about = "Sonolus Watch-mode package inspector")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    FfmpegInfo,
    ValidateProject {
        engine: PathBuf,
        resources: PathBuf,
        level_data: PathBuf,
        music: PathBuf,
        #[arg(long)]
        cover: Option<PathBuf>,
        #[arg(long = "mv")]
        custom_mv: Option<PathBuf>,
    },
    InspectEngine {
        engine: PathBuf,
    },
    InspectLevel {
        level: PathBuf,
    },
    InspectResources {
        scp: PathBuf,
    },
    Compatibility {
        engine: PathBuf,
        resources: PathBuf,
        level: PathBuf,
    },
    /// Evaluate one WatchData node and serialize the resulting display list.
    EvaluateNode {
        engine: PathBuf,
        node: usize,
    },
    /// Execute the generic Watch host for one supplied LevelData frame.
    RunWatch {
        engine: PathBuf,
        level: PathBuf,
        #[arg(long)]
        resources: Option<PathBuf>,
        #[arg(long, default_value_t = 0.0)]
        time: f64,
        #[arg(long, default_value_t = 1280)]
        width: u32,
        #[arg(long, default_value_t = 720)]
        height: u32,
        /// Write a skin-textured PPM frame after Watch execution (requires --resources).
        #[arg(long)]
        output: Option<PathBuf>,
        /// Write the executed display list, including Draw provenance, as JSON.
        #[arg(long)]
        display_list_output: Option<PathBuf>,
        /// Capture evaluated Watch node values for every executed Draw argument.
        #[arg(long, requires = "display_list_output")]
        trace_draws: bool,
        /// Write per-Draw transform, screen, atlas-UV, and painter-order diagnostics.
        #[arg(long, requires = "resources")]
        render_diagnostics_output: Option<PathBuf>,
        /// Capture a bounded trace when this entity callback reaches the VM limit.
        #[arg(long)]
        diagnostic_entity: Option<usize>,
        /// Callback stage for --diagnostic-entity (for example preprocess or update-sequential).
        #[arg(long)]
        diagnostic_stage: Option<String>,
        #[arg(long, default_value_t = 512)]
        diagnostic_events: usize,
    },
    /// Render a supplied display-list JSON file to a diagnostic PPM frame.
    RenderDisplayList {
        display_list: PathBuf,
        output: PathBuf,
        #[arg(long, default_value_t = 1280)]
        width: u32,
        #[arg(long, default_value_t = 720)]
        height: u32,
    },
    /// Render a bounded deterministic Watch timeline segment to an MP4 with level music.
    RenderVideo {
        engine: PathBuf,
        resources: PathBuf,
        level: PathBuf,
        music: PathBuf,
        output: PathBuf,
        /// Optional presentation asset; not used in gameplay rendering.
        #[arg(long)]
        cover: Option<PathBuf>,
        /// Custom MV compositing is not implemented.
        #[arg(long = "mv")]
        custom_mv: Option<PathBuf>,
        #[arg(long, default_value_t = 14.0)]
        start_time: f64,
        #[arg(long, default_value_t = 2.0)]
        duration: f64,
        #[arg(long, default_value_t = 12)]
        fps: u32,
        #[arg(long, default_value_t = 640)]
        width: u32,
        #[arg(long, default_value_t = 360)]
        height: u32,
        /// Include Draw geometry/Unlerp values for a runtime entity in frame diagnostics.
        #[arg(long)]
        trace_entity: Option<usize>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::FfmpegInfo => {
            let installation = renderer::ffmpeg::resolve()?;
            let source = match &installation.source {
                renderer::ffmpeg::FfmpegSource::SharedAddons => "shared addons",
                renderer::ffmpeg::FfmpegSource::Managed => "managed",
            };
            eprintln!("FFmpeg source: {source}");
            eprintln!("FFmpeg: {}", installation.ffmpeg.display());
            eprintln!("FFprobe: {}", installation.ffprobe.display());
            println!("{}", serde_json::to_string_pretty(&installation)?);
        }
        Command::ValidateProject {
            engine,
            resources,
            level_data,
            music,
            cover,
            custom_mv,
        } => {
            let project = renderer::project::ProjectInputs {
                engine_package: engine,
                resource_package: resources,
                level_data,
                music,
                cover,
                custom_mv,
            }
            .validate()?;
            println!(
                "Project inputs parsed and files accessible (media not decoded): {} entities, {} resources, cover={}, custom_mv={}",
                project.level.entities.len(),
                project.resources.resources.len(),
                project.cover.is_some(),
                project.custom_mv.is_some()
            );
        }
        Command::InspectEngine { engine } => println!(
            "{}",
            serde_json::to_string_pretty(&compatibility::inspect_engine(&engine)?)?
        ),
        Command::InspectLevel { level } => println!(
            "{}",
            serde_json::to_string_pretty(&formats::load_level(&level)?)?
        ),
        Command::InspectResources { scp } => println!(
            "{}",
            serde_json::to_string_pretty(&formats::inspect_scp(&scp)?)?
        ),
        Command::Compatibility {
            engine,
            resources,
            level,
        } => println!(
            "{}",
            serde_json::to_string_pretty(&compatibility::analyze(&engine, &resources, &level)?)?
        ),
        Command::EvaluateNode { engine, node } => {
            let package = formats::load_engine(&engine)?;
            let mut vm = renderer::runtime::WatchVm::new(&package.watch.nodes);
            let value = vm.execute(node)?;
            println!("value: {value}");
            println!(
                "display list:\n{}",
                serde_json::to_string_pretty(&vm.display_list)?
            );
        }
        Command::RunWatch {
            engine,
            level,
            resources,
            time,
            width,
            height,
            output,
            display_list_output,
            trace_draws,
            render_diagnostics_output,
            diagnostic_entity,
            diagnostic_stage,
            diagnostic_events,
        } => {
            let package = formats::load_engine(&engine)?;
            let level = formats::load_level(&level)?;
            let mut runtime = renderer::watch_runtime::WatchRuntime::new(&package.watch, &level)?;
            runtime.set_draw_tracing(trace_draws);
            match (diagnostic_entity, diagnostic_stage) {
                (Some(entity_id), Some(stage)) => {
                    let stage = parse_lifecycle_stage(&stage)?;
                    runtime.set_diagnostic_target(Some(
                        renderer::watch_runtime::WatchDiagnosticTarget {
                            entity_id,
                            stage,
                            event_capacity: diagnostic_events,
                        },
                    ))?;
                }
                (None, None) => {}
                _ => anyhow::bail!(
                    "--diagnostic-entity and --diagnostic-stage must be provided together"
                ),
            }
            runtime.bind_engine_rom(&package.rom)?;
            runtime.bind_engine_option_defaults(&package.configuration)?;
            if width == 0 || height == 0 {
                anyhow::bail!("Watch frame dimensions must be positive");
            }
            runtime.set_screen_aspect_ratio(f64::from(width) / f64::from(height))?;
            let skin_assets = if let Some(resources) = resources.as_ref() {
                let skin_name =
                    package.metadata.skin_name.as_deref().context(
                        "engine has no default skin name; select an explicit skin first",
                    )?;
                let assets = formats::load_skin_assets(&resources, skin_name)?;
                let sprite_names = assets.sprites.keys().cloned().collect();
                runtime.bind_skin_sprite_names(&sprite_names)?;
                println!(
                    "selected skin: {skin_name} ({} sprites)",
                    assets.sprites.len()
                );
                Some(assets)
            } else {
                None
            };
            let background_assets = match (
                resources.as_ref(),
                package.metadata.background_name.as_deref(),
            ) {
                (Some(resources), Some(background_name)) => {
                    let assets = formats::load_background_assets(resources, background_name)?;
                    let natural_aspect = f64::from(assets.width) / f64::from(assets.height);
                    let quad = assets
                        .data
                        .runtime_quad(f64::from(width) / f64::from(height), natural_aspect)?;
                    runtime.set_runtime_background_quad(quad)?;
                    println!(
                        "selected background: {background_name} ({}x{}, fit={})",
                        assets.width, assets.height, assets.data.fit
                    );
                    Some(assets)
                }
                _ => None,
            };
            let report = runtime.frame(time)?;
            println!(
                "Runtime Update [time, deltaTime, scaledTime, reserved]: {:?}; after callbacks={:?}; timescale={}",
                report.runtime_update, report.runtime_update_after_callbacks, report.timescale
            );
            println!(
                "runtime entities: {}, active entities: {}, callbacks: {}, Draw commands: {}",
                report.runtime_entity_count,
                report.active_entity_count,
                report.callbacks.len(),
                report.display_list.sprites.len()
            );
            println!(
                "raw Draw SHA-1: {}",
                renderer::offline::hash_draw_commands(&report.display_list)
            );
            println!(
                "Runtime Background quad (BL,TL,TR,BR): {:?}",
                report
                    .runtime_background_quad
                    .chunks_exact(2)
                    .map(|xy| [xy[0], xy[1]])
                    .collect::<Vec<_>>()
            );
            let mut stages = std::collections::BTreeMap::new();
            for callback in &report.callbacks {
                *stages
                    .entry(format!("{:?}", callback.stage))
                    .or_insert(0usize) += 1;
            }
            println!(
                "resolved entities: {} / {}",
                runtime.resolved_level_entity_count(),
                level.entities.len()
            );
            println!("runtime entities: {}", runtime.entities.len());
            println!("callbacks executed: {}", report.callbacks.len());
            println!("VM node evaluations: {}", report.vm_evaluations);
            println!("timeline: {:?}", report.timeline);
            println!("callbacks by stage: {}", serde_json::to_string(&stages)?);
            println!("new spawned entities: {}", report.spawned.len());
            println!(
                "Spawn requests produced: {}",
                report.spawn_requests_produced
            );
            println!(
                "active entities: {}",
                runtime
                    .entities
                    .iter()
                    .filter(|entity| entity.active)
                    .count()
            );
            let mut active_archetypes = std::collections::BTreeMap::new();
            for entity in runtime.entities.iter().filter(|entity| entity.active) {
                *active_archetypes
                    .entry(entity.archetype.clone())
                    .or_insert(0usize) += 1;
            }
            println!(
                "active archetypes: {}",
                serde_json::to_string(&active_archetypes)?
            );
            println!(
                "display-list sprites: {}",
                report.display_list.sprites.len()
            );
            println!(
                "Draw operations executed: {}",
                report.function_counts.get("Draw").copied().unwrap_or(0)
            );
            println!(
                "HasSkinSprite checks: {} (present={}, absent={})",
                report.skin_checks.len(),
                report
                    .skin_checks
                    .iter()
                    .filter(|(_, present)| *present)
                    .count(),
                report
                    .skin_checks
                    .iter()
                    .filter(|(_, present)| !*present)
                    .count()
            );
            println!(
                "executed Watch operations: {}",
                serde_json::to_string(&report.function_counts)?
            );
            for entity_id in &report.spawned {
                let entity = &runtime.entities[*entity_id];
                println!(
                    "spawned entity {}: archetype={}, active={}, initialized={}",
                    entity.id, entity.archetype, entity.active, entity.initialized
                );
            }
            for callback in report
                .callbacks
                .iter()
                .filter(|c| c.entity_id.is_some())
                .take(5)
            {
                println!(
                    "callback trace: entity={} archetype={} stage={:?} node={}",
                    callback.entity_id.unwrap_or_default(),
                    callback.archetype,
                    callback.stage,
                    callback.node
                );
            }
            let skin_bindings = skin_bindings(&package.watch)?;
            let mut draw_sources = std::collections::BTreeMap::new();
            for sprite in &report.display_list.sprites {
                if let Some(source) = &sprite.provenance {
                    *draw_sources
                        .entry((source.clone(), sprite.sprite_id))
                        .or_insert(0usize) += 1;
                }
            }
            for ((source, sprite_id), count) in draw_sources {
                let name = skin_bindings
                    .get(&sprite_id)
                    .map(String::as_str)
                    .unwrap_or("<unresolved>");
                println!(
                    "Draw provenance: entity={:?} archetype={:?} callback={:?} callback_node={:?} Draw_node={} sprite_id={sprite_id} sprite={name:?} commands={count}",
                    source.entity_id,
                    source.archetype,
                    source.callback,
                    source.callback_node,
                    source.draw_node
                );
            }
            if let Some(path) = display_list_output {
                let bytes = serde_json::to_vec_pretty(&report.display_list)?;
                std::fs::write(&path, bytes)
                    .with_context(|| format!("writing display list {}", path.display()))?;
                println!("wrote display list JSON to {}", path.display());
            }
            if let Some(path) = output {
                let skin = skin_assets
                    .as_ref()
                    .context("--output requires --resources with a valid skin package")?;
                let runtime_background = report
                    .runtime_background_quad
                    .chunks_exact(2)
                    .map(|coordinates| [coordinates[0], coordinates[1]])
                    .collect::<Vec<_>>();
                let runtime_background: [[f64; 2]; 4] = runtime_background
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("Runtime Background quad has invalid length"))?;
                let background = background_assets
                    .as_ref()
                    .map(|assets| (assets, runtime_background));
                let ppm = report.display_list.render_skin_ppm_with_background(
                    width,
                    height,
                    f64::from(width) / f64::from(height),
                    skin,
                    &skin_bindings,
                    background,
                )?;
                println!(
                    "pre-encode RGB SHA-1: {}",
                    renderer::offline::hash_rgb(&renderer::offline::ppm_rgb_payload(
                        &ppm, width, height
                    )?)
                );
                std::fs::write(&path, ppm)
                    .with_context(|| format!("writing rendered frame {}", path.display()))?;
                println!(
                    "wrote skin-rendered {}x{} PPM to {}",
                    width,
                    height,
                    path.display()
                );
            }
            if let Some(path) = render_diagnostics_output {
                let skin = skin_assets
                    .as_ref()
                    .context("--render-diagnostics-output requires loaded --resources")?;
                let diagnostics = report.display_list.skin_render_diagnostics(
                    width,
                    height,
                    f64::from(width) / f64::from(height),
                    skin,
                    &skin_bindings,
                )?;
                std::fs::write(&path, serde_json::to_vec_pretty(&diagnostics)?)
                    .with_context(|| format!("writing render diagnostics {}", path.display()))?;
                println!("wrote per-Draw render diagnostics to {}", path.display());
            }
        }
        Command::RenderDisplayList {
            display_list,
            output,
            width,
            height,
        } => {
            let list: renderer::runtime::DisplayList =
                serde_json::from_slice(&std::fs::read(&display_list)?)?;
            std::fs::write(&output, list.render_ppm(width, height)?)?;
            println!("wrote {}x{} PPM to {}", width, height, output.display());
        }
        Command::RenderVideo {
            engine,
            resources,
            level,
            music,
            output,
            cover,
            custom_mv,
            start_time,
            duration,
            fps,
            width,
            height,
            trace_entity,
        } => {
            if custom_mv.is_some() {
                anyhow::bail!("custom MV compositing is not implemented");
            }
            let _optional_cover = cover;
            let report = renderer::video_export::export(renderer::video_export::ExportRequest {
                engine: &engine,
                resources: &resources,
                level: &level,
                music: &music,
                output: &output,
                start_time,
                duration,
                fps,
                width,
                height,
                trace_entity_id: trace_entity,
            })?;
            println!("wrote MP4 to {}", output.display());
            println!(
                "submitted deterministic frames: {}",
                report.submitted_frames
            );
            println!("FFmpeg source: {}", report.ffmpeg_source);
            println!("FFmpeg: {}", report.ffmpeg_path);
            println!("FFprobe: {}", report.ffprobe_path);
            println!(
                "Watch timeline: {:.6}s..{:.6}s",
                report.timeline_start, report.timeline_end
            );
            println!("FPS: {}", report.fps);
            println!("Level bgmOffset: {}s", report.bgm_offset);
            println!(
                "Audio mapping: music source {:.6}s, leading silence {:.6}s",
                report.audio_window.source_start, report.audio_window.leading_silence
            );
            println!(
                "pre-encode deterministic frame hashes match: {} ({})",
                report.deterministic_frame_hashes_match, report.first_pass_hash
            );
            println!("FFprobe:\n{}", serde_json::to_string_pretty(&report.probe)?);
            println!(
                "per-frame Watch/render diagnostics:\n{}",
                serde_json::to_string_pretty(&report.frame_diagnostics)?
            );
            println!("Scheduled SFX events are not mixed into the main music track.");
        }
    }
    Ok(())
}

fn skin_bindings(
    watch: &renderer::watch::WatchData,
) -> Result<std::collections::BTreeMap<u32, String>> {
    let bindings = watch
        .skin
        .get("sprites")
        .and_then(serde_json::Value::as_array)
        .context("EngineWatchData skin binding has no sprites array")?;
    bindings
        .iter()
        .map(|binding| {
            let name = binding
                .get("name")
                .and_then(serde_json::Value::as_str)
                .context("EngineWatchData sprite binding has no name")?;
            let id = binding
                .get("id")
                .and_then(serde_json::Value::as_u64)
                .and_then(|id| u32::try_from(id).ok())
                .context("EngineWatchData sprite binding ID is outside u32 range")?;
            Ok((id, name.to_owned()))
        })
        .collect()
}

fn parse_lifecycle_stage(value: &str) -> Result<renderer::watch_runtime::LifecycleStage> {
    use renderer::watch_runtime::LifecycleStage;
    match value.to_ascii_lowercase().replace(['-', '_'], "").as_str() {
        "preprocess" => Ok(LifecycleStage::Preprocess),
        "spawntime" => Ok(LifecycleStage::SpawnTime),
        "despawntime" => Ok(LifecycleStage::DespawnTime),
        "initialize" => Ok(LifecycleStage::Initialize),
        "updatesequential" => Ok(LifecycleStage::UpdateSequential),
        "updateparallel" => Ok(LifecycleStage::UpdateParallel),
        "terminate" => Ok(LifecycleStage::Terminate),
        _ => anyhow::bail!("unknown diagnostic lifecycle stage {value:?}"),
    }
}
