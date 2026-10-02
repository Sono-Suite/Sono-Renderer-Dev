use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use renderer::{compatibility, formats};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "renderer", about = "Sonolus Watch-mode package inspector")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Default, Args)]
struct RenderLayerArgs {
    #[arg(long, conflicts_with = "no_ui")]
    ui: bool,
    #[arg(long)]
    no_ui: bool,
    #[arg(long, conflicts_with = "no_ui_primary_metric")]
    ui_primary_metric: bool,
    #[arg(long)]
    no_ui_primary_metric: bool,
    #[arg(long, conflicts_with = "no_ui_secondary_metric")]
    ui_secondary_metric: bool,
    #[arg(long)]
    no_ui_secondary_metric: bool,
    #[arg(long, conflicts_with = "no_ui_combo")]
    ui_combo: bool,
    #[arg(long)]
    no_ui_combo: bool,
    #[arg(long, conflicts_with = "no_ui_judgment")]
    ui_judgment: bool,
    #[arg(long)]
    no_ui_judgment: bool,
    #[arg(long, conflicts_with = "no_ui_progress")]
    ui_progress: bool,
    #[arg(long)]
    no_ui_progress: bool,
    #[arg(long)]
    primary_metric_provider: Option<String>,
    #[arg(long)]
    primary_metric_max: Option<f64>,
    #[arg(long)]
    primary_metric_label: Option<String>,
    #[arg(long)]
    secondary_metric_provider: Option<String>,
    #[arg(long)]
    secondary_metric_max: Option<f64>,
    #[arg(long)]
    secondary_metric_label: Option<String>,
    #[arg(long)]
    combo_provider: Option<String>,
    #[arg(long)]
    combo_label: Option<String>,
    #[arg(long)]
    judgment_provider: Option<String>,
    #[arg(long)]
    judgment_label: Option<String>,
    #[arg(long, conflicts_with = "no_particles")]
    particles: bool,
    #[arg(long)]
    no_particles: bool,
    #[arg(long, conflicts_with = "no_sfx")]
    sfx: bool,
    #[arg(long)]
    no_sfx: bool,
    #[arg(long, conflicts_with = "no_bgm")]
    bgm: bool,
    #[arg(long)]
    no_bgm: bool,
}

#[cfg(test)]
mod render_argument_tests {
    use super::*;

    #[test]
    fn cli_layer_and_ui_controls_build_independent_render_configuration() {
        let args = RenderLayerArgs {
            ui: false,
            no_ui: false,
            ui_primary_metric: false,
            no_ui_primary_metric: true,
            ui_secondary_metric: true,
            no_ui_secondary_metric: false,
            ui_combo: false,
            no_ui_combo: false,
            ui_judgment: false,
            no_ui_judgment: false,
            ui_progress: false,
            no_ui_progress: true,
            primary_metric_provider: None,
            primary_metric_max: None,
            primary_metric_label: None,
            secondary_metric_provider: Some("memory:2000:7".into()),
            secondary_metric_max: Some(100.0),
            secondary_metric_label: Some("ALT".into()),
            combo_provider: None,
            combo_label: None,
            judgment_provider: None,
            judgment_label: None,
            particles: false,
            no_particles: true,
            sfx: false,
            no_sfx: true,
            bgm: false,
            no_bgm: false,
        };
        let ui = args.ui_config().unwrap();
        assert!(!ui.enabled || ui.secondary_metric.enabled);
        assert!(ui.secondary_metric.enabled);
        assert!(!ui.primary_metric.enabled);
        assert!(!ui.progress);
        assert_eq!(
            ui.secondary_metric.provider,
            renderer::render_ui::UiValueProvider::Memory {
                block_id: 2000,
                index: 7
            }
        );
        let layers = args.layers();
        assert!(!layers.particles && !layers.sfx && layers.bgm);
    }
}

impl RenderLayerArgs {
    fn ui_config(&self) -> Result<renderer::render_ui::RendererUiConfig> {
        use renderer::render_ui::RendererUiConfig;
        let mut ui = RendererUiConfig::default();
        ui.enabled = !self.no_ui
            && (self.ui
                || self.ui_primary_metric
                || self.ui_secondary_metric
                || self.ui_combo
                || self.ui_judgment
                || self.ui_progress
                || self.primary_metric_provider.is_some()
                || self.secondary_metric_provider.is_some());
        ui.primary_metric.enabled = !self.no_ui_primary_metric;
        ui.secondary_metric.enabled = self.ui_secondary_metric && !self.no_ui_secondary_metric;
        ui.combo.enabled = self.ui_combo && !self.no_ui_combo;
        ui.judgment.enabled = self.ui_judgment && !self.no_ui_judgment;
        ui.progress = !self.no_ui_progress && (self.ui || self.ui_progress);
        if self.no_ui {
            ui.enabled = false;
        }
        if let Some(provider) = &self.primary_metric_provider {
            ui.primary_metric.provider = parse_ui_provider(provider)?;
            ui.primary_metric.enabled = !self.no_ui_primary_metric;
            ui.enabled = !self.no_ui;
        }
        if let Some(provider) = &self.secondary_metric_provider {
            ui.secondary_metric.provider = parse_ui_provider(provider)?;
            ui.secondary_metric.enabled = !self.no_ui_secondary_metric;
            ui.enabled = !self.no_ui;
        }
        if let Some(maximum) = self.primary_metric_max {
            ui.primary_metric.maximum = Some(maximum);
        }
        if let Some(maximum) = self.secondary_metric_max {
            ui.secondary_metric.maximum = Some(maximum);
        }
        if let Some(label) = &self.primary_metric_label {
            ui.primary_metric.label = label.clone();
        }
        if let Some(label) = &self.secondary_metric_label {
            ui.secondary_metric.label = label.clone();
        }
        if let Some(label) = &self.combo_label {
            ui.combo.label = label.clone();
        }
        if let Some(label) = &self.judgment_label {
            ui.judgment.label = label.clone();
        }
        if let Some(provider) = &self.combo_provider {
            ui.combo.provider = parse_ui_provider(provider)?;
            ui.combo.enabled = !self.no_ui_combo;
            ui.enabled = !self.no_ui;
        }
        if let Some(provider) = &self.judgment_provider {
            ui.judgment.provider = parse_ui_provider(provider)?;
            ui.judgment.enabled = !self.no_ui_judgment;
            ui.enabled = !self.no_ui;
        }
        ui.validate()?;
        Ok(ui)
    }

    fn layers(&self) -> renderer::render::RenderLayers {
        renderer::render::RenderLayers {
            particles: !self.no_particles,
            sfx: !self.no_sfx,
            bgm: !self.no_bgm,
        }
    }
}

fn parse_ui_provider(value: &str) -> Result<renderer::render_ui::UiValueProvider> {
    use renderer::render_ui::UiValueProvider;
    if let Some(raw) = value.strip_prefix("fixed:") {
        return Ok(UiValueProvider::Fixed {
            value: raw.parse()?,
        });
    }
    if let Some(raw) = value.strip_prefix("external:") {
        return Ok(UiValueProvider::External {
            value: raw.parse()?,
        });
    }
    if value == "progress" {
        return Ok(UiValueProvider::Progress);
    }
    if value == "accuracy" {
        return Ok(UiValueProvider::Accuracy);
    }
    if let Some(key) = value.strip_prefix("engine:") {
        return Ok(UiValueProvider::EngineMetric { key: key.into() });
    }
    if let Some(mapping) = value.strip_prefix("judgment:") {
        return Ok(UiValueProvider::JudgmentDerived {
            mapping: mapping.into(),
        });
    }
    if let Some(pair) = value.strip_prefix("memory:") {
        let (block, index) = pair
            .split_once(':')
            .context("memory provider must be memory:BLOCK:INDEX")?;
        return Ok(UiValueProvider::Memory {
            block_id: block.parse()?,
            index: index.parse()?,
        });
    }
    anyhow::bail!("unknown metric provider {value:?}; use fixed:N, external:N, memory:BLOCK:INDEX, progress, engine:KEY, accuracy, or judgment:MAP")
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
        /// Override a Level Option after EngineConfiguration defaults are bound (repeatable: INDEX=VALUE).
        #[arg(long = "level-option", value_name = "INDEX=VALUE", action = clap::ArgAction::Append)]
        level_options: Vec<String>,
        #[arg(long, default_value_t = 1280)]
        width: u32,
        #[arg(long, default_value_t = 720)]
        height: u32,
        #[command(flatten)]
        render_layers: RenderLayerArgs,
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
        /// Override a Level Option after EngineConfiguration defaults are bound (repeatable: INDEX=VALUE).
        #[arg(long = "level-option", value_name = "INDEX=VALUE", action = clap::ArgAction::Append)]
        level_options: Vec<String>,
        #[command(flatten)]
        render_layers: RenderLayerArgs,
    },
    /// Open the local browser based preview and video export frontend.
    Gui,
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
            level_options,
            width,
            height,
            render_layers,
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
            let defaults: std::collections::BTreeMap<_, _> =
                package.metadata.resource_defaults().into_iter().collect();
            let resource_path = resources.as_deref();
            let has_skin_bindings = package
                .watch
                .skin
                .get("sprites")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|bindings| !bindings.is_empty());
            let has_effect_bindings = package
                .watch
                .effect
                .get("clips")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|bindings| !bindings.is_empty());
            let has_particle_bindings = package
                .watch
                .particle
                .get("effects")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|bindings| !bindings.is_empty());
            let needs_resources = has_skin_bindings
                || has_effect_bindings
                || has_particle_bindings
                || ["skins", "backgrounds", "effects", "particles"]
                    .iter()
                    .any(|category| defaults.contains_key(*category));
            let resource_path = match (resource_path, needs_resources) {
                (Some(path), _) => Some(path),
                (None, true) => {
                    let selected = defaults
                        .iter()
                        .map(|(category, name)| format!("{category}={name}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    anyhow::bail!(
                        "required resource package is missing (--resources); engine selections: {selected}"
                    )
                }
                (None, false) => None,
            };
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
            runtime.bind_engine_ui_configuration(&package.configuration)?;
            let option_overrides = level_options
                .iter()
                .map(|value| parse_level_option_override(value))
                .collect::<Result<Vec<_>>>()?;
            runtime.bind_engine_option_overrides(&package.configuration, &option_overrides)?;
            for (index, value) in &option_overrides {
                println!("Level Option[{index}] override: {value}");
            }
            if width == 0 || height == 0 {
                anyhow::bail!("Watch frame dimensions must be positive");
            }
            runtime.set_screen_aspect_ratio(f64::from(width) / f64::from(height))?;
            let mut particle_assets_for_render = None;
            let mut particle_bindings_for_render = std::collections::BTreeMap::new();
            let skin_assets = if let Some(resource_path) = resource_path {
                let skin_name = defaults
                    .get("skins")
                    .map(String::as_str)
                    .or(package.metadata.skin_name.as_deref());
                let skin_name = match (skin_name, has_skin_bindings) {
                    (Some(name), _) => Some(name),
                    (None, true) => anyhow::bail!(
                        "engine declares skin sprites but has no selected/default skin resource"
                    ),
                    (None, false) => None,
                };
                let assets = skin_name
                    .map(|name| {
                        formats::load_skin_assets(resource_path, name)
                            .with_context(|| format!("resolving selected skin resource {name:?}"))
                    })
                    .transpose()?;
                if let Some(assets) = assets.as_ref() {
                    let sprite_names = assets.sprites.keys().cloned().collect();
                    runtime.bind_skin_sprite_names(&sprite_names)?;
                    println!(
                        "selected skin: {} ({} sprites)",
                        skin_name.unwrap_or("unspecified"),
                        assets.sprites.len()
                    );
                }
                let effect_name = defaults
                    .get("effects")
                    .map(String::as_str)
                    .or(package.metadata.effect_name.as_deref());
                if effect_name.is_none() && has_effect_bindings {
                    anyhow::bail!(
                        "engine declares effect clips but has no selected/default effect resource"
                    );
                }
                if let Some(effect_name) = effect_name {
                    let names = formats::load_effect_clip_names(resource_path, effect_name)
                        .with_context(|| {
                            format!("resolving selected effect resource {effect_name:?}")
                        })?;
                    if package
                        .watch
                        .effect
                        .get("clips")
                        .and_then(serde_json::Value::as_array)
                        .is_some()
                    {
                        runtime.bind_effect_clip_names(&names)?;
                    }
                    println!(
                        "selected effect resource: {effect_name} ({} clips)",
                        names.len()
                    );
                }
                let particle_name = defaults
                    .get("particles")
                    .map(String::as_str)
                    .or(package.metadata.particle_name.as_deref());
                if particle_name.is_none() && has_particle_bindings {
                    anyhow::bail!(
                        "engine declares particle effects but has no selected/default particle resource"
                    );
                }
                if let Some(particle_name) = particle_name {
                    let names = formats::load_particle_effect_names(resource_path, particle_name)
                        .with_context(|| {
                        format!("resolving selected particle resource {particle_name:?}")
                    })?;
                    if package
                        .watch
                        .particle
                        .get("effects")
                        .and_then(serde_json::Value::as_array)
                        .is_some()
                    {
                        runtime.bind_particle_effect_names(&names)?;
                    }
                    if output.is_some() && render_layers.layers().particles {
                        let particle_assets =
                            formats::load_particle_assets(resource_path, particle_name)?;
                        let bindings = renderer::offline::particle_effect_bindings(&package.watch)?;
                        particle_bindings_for_render = bindings
                            .into_iter()
                            .filter(|(_, name)| particle_assets.effects.contains_key(name))
                            .collect();
                        particle_assets_for_render = Some(particle_assets);
                    }
                    println!(
                        "selected particle resource: {particle_name} ({} effects)",
                        names.len()
                    );
                }
                assets
            } else {
                None
            };
            let background_assets = match (
                resource_path,
                defaults
                    .get("backgrounds")
                    .map(String::as_str)
                    .or(package.metadata.background_name.as_deref()),
            ) {
                (Some(resources), Some(background_name)) => {
                    let assets = formats::load_background_assets(resources, background_name)
                        .with_context(|| {
                            format!("resolving selected background resource {background_name:?}")
                        })?;
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
                (Some(_), None) | (None, _) => None,
            };
            let report = runtime.frame(time)?;
            let rhs_forensic_reports = runtime.rhs_forensic_reports();
            if !rhs_forensic_reports.is_empty() {
                println!(
                    "RHS forensic callback replays (time={time}, callback=NormalHeadTapNote.UpdateParallel, root=node88326):"
                );
                for (entity_id, events) in rhs_forensic_reports {
                    println!("  entity={entity_id}");
                    for event in events {
                        println!("    {event}");
                    }
                }
            }
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
            println!(
                "Runtime Skin Transform (row-major values 0..15): {:?}",
                report.runtime_skin_transform
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
            println!("Watch debug events:");
            for event in &report.debug_events {
                match event {
                    renderer::runtime::DebugEvent::Log {
                        value,
                        entity_id,
                        callback,
                        node,
                        ..
                    } => println!(
                        "  DebugLog entity={entity_id:?} callback={callback:?} node={node}: {}",
                        renderer::runtime::DebugEvent::display_value(*value)
                    ),
                    renderer::runtime::DebugEvent::Pause {
                        entity_id,
                        callback,
                        node,
                        ..
                    } => println!(
                        "  DebugPause entity={entity_id:?} callback={callback:?} node={node}"
                    ),
                }
            }
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
                let mut ppm = report
                    .display_list
                    .render_skin_ppm_with_runtime_transform_and_background(
                        width,
                        height,
                        f64::from(width) / f64::from(height),
                        skin,
                        &skin_bindings,
                        &report.runtime_skin_transform,
                        background,
                    )?;
                if render_layers.layers().particles {
                    if let Some(assets) = particle_assets_for_render.as_ref() {
                        let instances = runtime
                            .context
                            .particle_instances
                            .read()
                            .map_err(|_| {
                                anyhow::anyhow!("particle instance state lock was poisoned")
                            })?
                            .instances()
                            .values()
                            .cloned()
                            .collect::<Vec<_>>();
                        let draws = renderer::particles::render_instances(
                            assets,
                            &particle_bindings_for_render,
                            &instances,
                            time,
                        )?;
                        let mut rgb = renderer::offline::ppm_rgb_payload(&ppm, width, height)?;
                        renderer::runtime::DisplayList::composite_particle_sprites(
                            &mut rgb,
                            width,
                            height,
                            f64::from(width) / f64::from(height),
                            assets,
                            &draws,
                            &report.runtime_particle_transform,
                        )?;
                        ppm = {
                            let mut bytes = format!("P6\n{width} {height}\n255\n").into_bytes();
                            bytes.extend_from_slice(&rgb);
                            bytes
                        };
                    }
                }
                let ui = render_layers.ui_config()?;
                if ui.enabled {
                    let mut rgb = renderer::offline::ppm_rgb_payload(&ppm, width, height)?;
                    renderer::render_ui::render_overlay(
                        &mut rgb,
                        width,
                        height,
                        report.timeline.unwrap_or(time),
                        0.0,
                        1.0,
                        &ui,
                        |provider| match provider {
                            renderer::render_ui::UiValueProvider::Fixed { value }
                            | renderer::render_ui::UiValueProvider::External { value } => {
                                Ok(*value)
                            }
                            renderer::render_ui::UiValueProvider::Memory { block_id, index } => {
                                Ok(runtime.global_memory.get(*block_id, *index))
                            }
                            renderer::render_ui::UiValueProvider::Progress => Ok(time),
                            renderer::render_ui::UiValueProvider::EngineMetric { key } => {
                                anyhow::bail!(
                                "engine metric provider {key:?} has no authoritative Watch source"
                            )
                            }
                            renderer::render_ui::UiValueProvider::Accuracy => anyhow::bail!(
                                "accuracy provider is unsupported without a verified Watch source"
                            ),
                            renderer::render_ui::UiValueProvider::JudgmentDerived { mapping } => {
                                anyhow::bail!(
                                    "judgment-derived provider {mapping:?} requires caller logic"
                                )
                            }
                        },
                    )?;
                    ppm = {
                        let mut bytes = format!("P6\n{width} {height}\n255\n").into_bytes();
                        bytes.extend_from_slice(&rgb);
                        bytes
                    };
                }
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
                let diagnostics = report
                    .display_list
                    .skin_render_diagnostics_with_runtime_transform(
                        width,
                        height,
                        f64::from(width) / f64::from(height),
                        skin,
                        &skin_bindings,
                        &report.runtime_skin_transform,
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
            level_options,
            render_layers,
        } => {
            if custom_mv.is_some() {
                anyhow::bail!("custom MV compositing is not implemented");
            }
            let _optional_cover = cover;
            let option_overrides = level_options
                .iter()
                .map(|value| parse_level_option_override(value))
                .collect::<Result<Vec<_>>>()?;
            let config = renderer::render::RenderConfig {
                engine,
                resources,
                level,
                skin: None,
                music: Some(music),
                output: Some(output.clone()),
                start_time,
                duration,
                fps,
                width,
                height,
                level_options: option_overrides
                    .into_iter()
                    .map(|(index, value)| renderer::render::LevelOptionValue { index, value })
                    .collect(),
                layers: render_layers.layers(),
                ui: render_layers.ui_config()?,
                trace_entity_id: trace_entity,
            };
            let report = config.render_video()?;
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
            if render_layers.layers().bgm {
                println!(
                    "Audio mapping: music source {:.6}s, leading silence {:.6}s",
                    report.audio_window.source_start, report.audio_window.leading_silence
                );
            }
            println!(
                "pre-encode deterministic frame hashes match: {} ({})",
                report.deterministic_frame_hashes_match, report.first_pass_hash
            );
            println!("FFprobe:\n{}", serde_json::to_string_pretty(&report.probe)?);
            println!(
                "per-frame Watch/render diagnostics:\n{}",
                serde_json::to_string_pretty(&report.frame_diagnostics)?
            );
            println!("SFX mixed: {}", render_layers.layers().sfx);
        }
        Command::Gui => renderer::gui::launch()?,
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

fn parse_level_option_override(value: &str) -> Result<(usize, f64)> {
    let (index, value) = value
        .split_once('=')
        .with_context(|| format!("invalid --level-option {value:?}; expected INDEX=VALUE"))?;
    let index = index
        .parse::<usize>()
        .with_context(|| format!("invalid Level Option index {index:?}"))?;
    let value = value
        .parse::<f64>()
        .with_context(|| format!("invalid Level Option value {value:?}"))?;
    if !value.is_finite() {
        anyhow::bail!("Level Option override must be finite");
    }
    Ok((index, value))
}

#[cfg(test)]
mod cli_tests {
    use super::parse_level_option_override;
    use clap::Parser;

    #[test]
    fn parses_level_option_override() {
        assert_eq!(parse_level_option_override("22=0").unwrap(), (22, 0.0));
        assert_eq!(parse_level_option_override("1=10.8").unwrap(), (1, 10.8));
    }

    #[test]
    fn rejects_malformed_level_option_override() {
        assert!(parse_level_option_override("22").is_err());
        assert!(parse_level_option_override("x=1").is_err());
        assert!(parse_level_option_override("1=abc").is_err());
        assert!(parse_level_option_override("1=NaN").is_err());
    }

    #[test]
    fn run_watch_accepts_repeated_level_option_flags() {
        let cli = super::Cli::try_parse_from([
            "renderer",
            "run-watch",
            "engine.zip",
            "level.json.gz",
            "--level-option",
            "1=10.8",
            "--level-option",
            "22=0",
        ])
        .unwrap();
        match cli.command {
            super::Command::RunWatch { level_options, .. } => {
                assert_eq!(level_options, ["1=10.8", "22=0"]);
            }
            _ => panic!("expected run-watch command"),
        }
    }

    #[test]
    fn render_video_accepts_repeated_level_option_flags() {
        let cli = super::Cli::try_parse_from([
            "renderer",
            "render-video",
            "engine.zip",
            "resources.scp",
            "level.json.gz",
            "music.mp3",
            "output.mp4",
            "--level-option",
            "1=10.8",
            "--level-option",
            "22=0",
        ])
        .unwrap();
        match cli.command {
            super::Command::RenderVideo { level_options, .. } => {
                let parsed = level_options
                    .iter()
                    .map(|value| parse_level_option_override(value).unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(parsed, [(1, 10.8), (22, 0.0)]);
            }
            _ => panic!("expected render-video command"),
        }
    }
}
