//! Shared configuration and entry points for CLI, preview, and video rendering.

use crate::{
    formats, offline::FrameSession, offline::RenderedFrame, render_ui::RendererUiConfig,
    video_export,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LevelOptionValue {
    pub index: usize,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RenderLayers {
    pub particles: bool,
    pub sfx: bool,
    pub bgm: bool,
}

impl Default for RenderLayers {
    fn default() -> Self {
        Self {
            particles: true,
            sfx: true,
            bgm: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RenderConfig {
    pub engine: PathBuf,
    pub resources: PathBuf,
    pub level: PathBuf,
    #[serde(default)]
    pub skin: Option<String>,
    #[serde(default)]
    pub music: Option<PathBuf>,
    #[serde(default)]
    pub output: Option<PathBuf>,
    #[serde(default)]
    pub start_time: f64,
    #[serde(default = "default_duration")]
    pub duration: f64,
    #[serde(default = "default_fps")]
    pub fps: u32,
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default)]
    pub level_options: Vec<LevelOptionValue>,
    #[serde(default)]
    pub trace_entity_id: Option<usize>,
    #[serde(default)]
    pub layers: RenderLayers,
    #[serde(default)]
    pub ui: RendererUiConfig,
    #[serde(default)]
    pub backend: RenderBackend,
    #[serde(default)]
    pub profile: bool,
    #[serde(default)]
    pub profile_frames: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RenderBackend {
    Cpu,
    #[default]
    Wgpu,
}

fn default_duration() -> f64 {
    2.0
}
fn default_fps() -> u32 {
    12
}
fn default_width() -> u32 {
    640
}
fn default_height() -> u32 {
    360
}

impl RenderConfig {
    /// Load the engine through the same archive/directory loader used by the
    /// existing inspection and video-export paths.
    pub fn load_engine_package(&self) -> Result<formats::EnginePackage> {
        formats::load_engine(&self.engine)
    }

    pub fn validate(&self) -> Result<()> {
        if self.width == 0 || self.height == 0 || self.width > 8192 || self.height > 8192 {
            bail!("render dimensions must be in 1..=8192");
        }
        if self.fps == 0 || self.fps > 240 {
            bail!("FPS must be in 1..=240");
        }
        if !self.start_time.is_finite() || self.start_time < 0.0 {
            bail!("start time must be finite and nonnegative");
        }
        if !self.duration.is_finite() || self.duration <= 0.0 {
            bail!("duration must be finite and positive");
        }
        for option in &self.level_options {
            if !option.value.is_finite() {
                bail!("Level Option {} must be finite", option.index);
            }
        }
        self.ui.validate()?;
        Ok(())
    }

    /// Run the actual Watch/skin/background/particle/UI pipeline for one time.
    /// Time is aligned upward to the next frame at the configured FPS, matching
    /// the video export's global frame mapping.
    pub fn render_frame(&self, time: f64) -> Result<RenderedFrame> {
        self.validate()?;
        if !time.is_finite() || time < 0.0 {
            bail!("preview timestamp must be finite and nonnegative");
        }
        let package = self.load_engine_package()?;
        let level = formats::load_level(&self.level)?;
        let defaults: std::collections::BTreeMap<_, _> =
            package.metadata.resource_defaults().into_iter().collect();
        let skin = self
            .skin
            .as_deref()
            .or_else(|| defaults.get("skins").map(String::as_str))
            .or(package.metadata.skin_name.as_deref())
            .context("engine has no skin selection")?;
        let background = defaults
            .get("backgrounds")
            .map(String::as_str)
            .or(package.metadata.background_name.as_deref());
        let effects = defaults
            .get("effects")
            .map(String::as_str)
            .or(package.metadata.effect_name.as_deref());
        let particles = defaults
            .get("particles")
            .map(String::as_str)
            .or(package.metadata.particle_name.as_deref());
        let options: Vec<_> = self
            .level_options
            .iter()
            .map(|item| (item.index, item.value))
            .collect();
        let mut session = FrameSession::new_configured(
            &package.watch,
            &package.rom,
            &package.configuration,
            &level,
            &self.resources,
            skin,
            background,
            effects,
            particles,
            self.width,
            self.height,
            self.fps,
            &options,
            self.layers.particles,
            self.ui.clone(),
            self.start_time,
            self.duration,
            self.backend,
            self.profile,
            self.trace_entity_id.is_some(),
        )?;
        let index = (time * f64::from(self.fps)).ceil() as u64;
        session.render_global_frame(index)
    }

    pub fn render_video(&self) -> Result<video_export::ExportReport> {
        self.validate()?;
        video_export::export_config(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_json_preserves_arbitrary_level_option_indices_and_layer_toggles() {
        let json = serde_json::json!({
            "engine":"e.zip", "resources":"r.scp", "level":"l.zip",
            "level_options":[{"index":1,"value":10.5},{"index":73,"value":-2.0}],
            "layers":{"particles":false,"sfx":true,"bgm":false},
            "ui":{"enabled":true,"primary_metric":{"enabled":true,"label":"POINTS",
                "provider":{"kind":"fixed","value":95},"maximum":100}
            }
        });
        let config: RenderConfig = serde_json::from_value(json).unwrap();
        assert_eq!(
            config.level_options[1],
            LevelOptionValue {
                index: 73,
                value: -2.0
            }
        );
        assert!(!config.layers.particles && config.layers.sfx && !config.layers.bgm);
        let encoded = serde_json::to_value(config).unwrap();
        assert_eq!(encoded["level_options"][1]["index"], 73);
    }

    #[test]
    fn preview_and_export_share_config_defaults_and_frame_alignment() {
        let json = serde_json::json!({
            "engine":"engine.zip", "resources":"resources.scp", "level":"level.zip"
        });
        let config: RenderConfig = serde_json::from_value(json).unwrap();
        assert_eq!(config.start_time, 0.0);
        assert_eq!(config.duration, 2.0);
        assert_eq!((config.fps, config.width, config.height), (12, 640, 360));
        assert_eq!(config.layers, RenderLayers::default());
        assert_eq!(config.backend, RenderBackend::Wgpu);
        assert!(!config.ui.enabled);
        assert_eq!((0.01_f64 * 12.0).ceil() as u64, 1);
    }

    #[test]
    fn shared_config_loads_next_rush_engine_zip_through_standard_loader() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let config: RenderConfig = serde_json::from_value(serde_json::json!({
            "engine": repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"),
            "resources": "resources.scp",
            "level": "level.zip"
        }))
        .unwrap();

        let package = config.load_engine_package().unwrap();
        assert!(!package.watch.nodes.is_empty());
        assert!(package.watch.update_spawn.is_some());
    }

    #[test]
    fn next_rush_shared_preview_initializes_baumkuchen_at_cli_options() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let package =
            formats::load_engine(&repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"))
                .unwrap();
        let level = formats::load_level(&repo.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz")).unwrap();
        let options = [(1, 10.8), (22, 1.0)];
        let mut stepper = crate::offline::WatchFrameStepper::new_with_engine_options(
            &package.watch,
            &package.rom,
            &package.configuration,
            &level,
            12,
            &options,
        )
        .unwrap();

        // Preview shares the same frame-zero initialization and monotonically advances
        // through the requested frame, just as video export does.
        let report = stepper.advance_to(0).unwrap();
        assert!(report.callbacks.iter().any(|callback| {
            callback.archetype == "Initialization"
                && callback.stage == crate::watch_runtime::LifecycleStage::Preprocess
        }));
        assert_eq!(stepper.runtime_mut().global_memory.get(2002, 1), 10.8);
        assert_eq!(stepper.runtime_mut().global_memory.get(2002, 22), 1.0);
    }

    #[test]
    fn next_rush_baumkuchen_shared_preview_has_deterministic_wgpu_frame() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let config: RenderConfig = serde_json::from_value(serde_json::json!({
            "engine": repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"),
            "resources": repo.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp"),
            "level": repo.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz"),
            "start_time": 0.0,
            "duration": 0.25,
            "fps": 12,
            "width": 320,
            "height": 180,
            "level_options": [{"index": 1, "value": 10.8}, {"index": 22, "value": 1.0}],
            "layers": {"particles": false, "sfx": false, "bgm": false},
            "backend": "wgpu"
        })).unwrap();

        let frame = config.render_frame(0.0).unwrap();
        assert_eq!(
            crate::offline::hash_rgb(&frame.rgb),
            "d45f286a15fac6be9b51ded7d0015356b8b92a72"
        );
    }

    #[test]
    fn next_rush_armageddon_shared_preview_initialization_succeeds() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let config: RenderConfig = serde_json::from_value(serde_json::json!({
            "engine": repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"),
            "resources": repo.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp"),
            "level": repo.join("TestingSuite/Next Sekai Engine/levels/Touhou - ARMAGEDDON/Armageddon.json.gz"),
            "start_time": 14.0,
            "duration": 2.0,
            "fps": 12,
            "width": 128,
            "height": 72,
            "level_options": [{"index": 1, "value": 10.8}, {"index": 22, "value": 1.0}],
            "layers": {"particles": false, "sfx": false, "bgm": false}
        })).unwrap();

        let frame = config.render_frame(14.0).unwrap();
        assert_eq!(frame.report.runtime_update[0], 14.0);
        assert_eq!(frame.rgb.len(), 128 * 72 * 3);

        let mut gpu_config = config.clone();
        gpu_config.backend = RenderBackend::Wgpu;
        let gpu = gpu_config.render_frame(14.0).unwrap();
        assert_close_rgb(&frame.rgb, &gpu.rgb, 3, 0.002);
    }

    #[test]
    fn next_rush_lapis_shared_preview_matches_cpu_reference_on_wgpu() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let config: RenderConfig = serde_json::from_value(serde_json::json!({
            "engine": repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"),
            "resources": repo.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp"),
            "level": repo.join("TestingSuite/Next Sekai Engine/levels/DIAMOND Lapis/DIAMOND Lapis.json.gz"),
            "start_time": 0.0, "duration": 1.0, "fps": 12, "width": 160, "height": 90,
            "layers": {"particles": false, "sfx": false, "bgm": false}
        })).unwrap();
        let cpu = config.render_frame(0.0).unwrap();
        let mut gpu_config = config;
        gpu_config.backend = RenderBackend::Wgpu;
        let gpu = gpu_config.render_frame(0.0).unwrap();
        assert_close_rgb(&cpu.rgb, &gpu.rgb, 3, 0.002);
    }

    fn assert_close_rgb(cpu: &[u8], gpu: &[u8], max_delta: u8, max_outlier_fraction: f64) {
        assert_eq!(cpu.len(), gpu.len());
        let mut outliers = 0usize;
        for (&a, &b) in cpu.iter().zip(gpu) {
            if a.abs_diff(b) > max_delta {
                outliers += 1;
            }
        }
        let fraction = outliers as f64 / cpu.len().max(1) as f64;
        assert!(
            fraction <= max_outlier_fraction,
            "{outliers}/{} RGB samples exceed {max_delta}: {fraction:.6}",
            cpu.len()
        );
    }
}
