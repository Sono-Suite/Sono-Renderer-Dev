//! Deterministic Watch-frame production for offline rendering.
//! This module knows about Sonolus frames and assets, but not FFmpeg or CLI.

use crate::{
    formats,
    runtime::DisplayList,
    watch::WatchData,
    watch_runtime::{FrameReport, WatchRuntime},
};
use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameRange {
    pub first_index: u64,
    pub frame_count: u64,
    pub fps: u32,
}

impl FrameRange {
    pub fn new(start_time: f64, duration: f64, fps: u32) -> Result<Self> {
        if !start_time.is_finite() || start_time < 0.0 {
            bail!("segment start time must be finite and nonnegative");
        }
        if !duration.is_finite() || duration <= 0.0 {
            bail!("segment duration must be finite and positive");
        }
        if fps == 0 || fps > 240 {
            bail!("FPS must be in 1..=240");
        }
        let first = (start_time * f64::from(fps)).ceil();
        let count = (duration * f64::from(fps) - 1e-10).ceil();
        if !first.is_finite() || !count.is_finite() || first > u64::MAX as f64 || count < 1.0 {
            bail!("segment frame range is outside supported bounds");
        }
        let first_index = first as u64;
        let frame_count = count as u64;
        first_index
            .checked_add(frame_count)
            .context("segment frame range overflows")?;
        if frame_count > 1_000_000 {
            bail!("segment is too long (limit: 1,000,000 frames)");
        }
        Ok(Self {
            first_index,
            frame_count,
            fps,
        })
    }

    pub fn index(self, segment_frame: u64) -> Result<u64> {
        if segment_frame >= self.frame_count {
            bail!("segment frame index is out of range");
        }
        Ok(self.first_index + segment_frame)
    }

    /// Sonolus timeline time for a global video frame index.
    pub fn time(self, global_frame_index: u64) -> f64 {
        global_frame_index as f64 / f64::from(self.fps)
    }

    pub fn start_time(self) -> f64 {
        self.time(self.first_index)
    }
}

/// Map level timeline time to source audio position. Sonolus starts the BGM at
/// `bgmOffset` and derives level time as `audioPosition - bgmOffset`, so the
/// inverse mapping is `sourcePosition = levelTime + bgmOffset`. Negative source
/// positions become leading silence in the exported clip.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct AudioWindow {
    pub source_start: f64,
    pub leading_silence: f64,
}

pub fn audio_window(level_start: f64, bgm_offset: f64) -> Result<AudioWindow> {
    if !level_start.is_finite() || level_start < 0.0 || !bgm_offset.is_finite() {
        bail!("audio timeline inputs must be finite and level start nonnegative");
    }
    let source_time = level_start + bgm_offset;
    Ok(AudioWindow {
        source_start: source_time.max(0.0),
        leading_silence: (-source_time).max(0.0),
    })
}

pub struct FrameSession<'a> {
    stepper: WatchFrameStepper<'a>,
    skin: formats::SkinAssets,
    background: Option<formats::BackgroundAssets>,
    bindings: BTreeMap<u32, String>,
    particles: Option<formats::ParticleAssets>,
    particle_bindings: BTreeMap<i64, String>,
    particles_enabled: bool,
    ui: crate::render_ui::RendererUiConfig,
    ui_segment_start: f64,
    ui_segment_duration: f64,
    width: u32,
    height: u32,
    backend: crate::render::RenderBackend,
    gpu: Option<&'static crate::gpu_render::GpuRenderer>,
    profile: bool,
}

pub struct RenderedFrame {
    pub report: FrameReport,
    pub rgb: Vec<u8>,
    pub profile: Option<FrameStageProfile>,
}

#[derive(Debug, Clone, Default)]
pub struct FrameStageProfile {
    pub vm: std::time::Duration,
    pub preparation: std::time::Duration,
    pub cpu_render: std::time::Duration,
    pub runtime_ui: std::time::Duration,
    pub total: std::time::Duration,
    pub entities: usize,
    pub skin_draws: usize,
    pub particle_draws: usize,
    pub gpu_encode_submit: std::time::Duration,
    pub gpu_draw_preparation: std::time::Duration,
    pub gpu_wait_map: std::time::Duration,
    pub framebuffer_unpack: std::time::Duration,
    pub framebuffer_copy: std::time::Duration,
    pub gpu_draw_calls: usize,
    pub atlas_uploads: usize,
    pub readback_bytes: u64,
    pub adapter: Option<String>,
}

pub fn effect_clip_bindings(watch: &WatchData) -> Result<BTreeMap<i64, String>> {
    let values = watch
        .effect
        .get("clips")
        .and_then(serde_json::Value::as_array)
        .context("EngineWatchData effect bindings have no clips array")?;
    values
        .iter()
        .map(|binding| {
            let id = binding
                .get("id")
                .and_then(serde_json::Value::as_i64)
                .context("effect clip binding has invalid ID")?;
            let name = binding
                .get("name")
                .and_then(serde_json::Value::as_str)
                .context("effect clip binding has no name")?;
            Ok((id, name.to_owned()))
        })
        .collect()
}

pub fn particle_effect_bindings(watch: &WatchData) -> Result<BTreeMap<i64, String>> {
    let values = watch
        .particle
        .get("effects")
        .and_then(serde_json::Value::as_array)
        .context("EngineWatchData particle bindings have no effects array")?;
    values
        .iter()
        .map(|binding| {
            let id = binding
                .get("id")
                .and_then(serde_json::Value::as_i64)
                .context("particle effect binding has invalid ID")?;
            let name = binding
                .get("name")
                .and_then(serde_json::Value::as_str)
                .context("particle effect binding has no name")?;
            Ok((id, name.to_owned()))
        })
        .collect()
}

/// A stateful, deterministic Watch timeline advanced at one configured rate.
/// Frame index `N` always maps to absolute Sonolus time `N/F`.
pub struct WatchFrameStepper<'a> {
    runtime: WatchRuntime<'a>,
    fps: u32,
    last_frame_index: Option<u64>,
}

impl<'a> WatchFrameStepper<'a> {
    pub fn new(watch: &'a WatchData, level: &formats::LevelData, fps: u32) -> Result<Self> {
        if fps == 0 || fps > 240 {
            bail!("FPS must be in 1..=240");
        }
        Ok(Self {
            runtime: WatchRuntime::new(watch, level)?,
            fps,
            last_frame_index: None,
        })
    }

    pub fn new_with_engine_options(
        watch: &'a WatchData,
        rom: &[u8],
        configuration: &serde_json::Value,
        level: &formats::LevelData,
        fps: u32,
        overrides: &[(usize, f64)],
    ) -> Result<Self> {
        let mut stepper = Self::new(watch, level, fps)?;
        let runtime = stepper.runtime_mut();
        runtime.bind_engine_rom(rom)?;
        runtime.bind_engine_option_defaults(configuration)?;
        runtime.bind_engine_ui_configuration(configuration)?;
        runtime.bind_engine_option_overrides(configuration, overrides)?;
        Ok(stepper)
    }

    pub fn runtime_mut(&mut self) -> &mut WatchRuntime<'a> {
        &mut self.runtime
    }

    pub fn advance_to(&mut self, frame_index: u64) -> Result<FrameReport> {
        if self
            .last_frame_index
            .is_some_and(|previous| frame_index <= previous)
        {
            bail!("Watch frame indices must advance monotonically");
        }
        if self.last_frame_index.is_none() && frame_index > 1_000_000 {
            bail!("pre-roll exceeds 1,000,000 frames");
        }
        let start = self.last_frame_index.map_or(0, |previous| previous + 1);
        let mut final_report = None;
        let mut audio_events = Vec::new();
        let mut scheduled_effects = Vec::new();
        let mut scheduled_looped_effects = Vec::new();
        let mut scheduled_looped_effect_stops = Vec::new();
        for index in start..=frame_index {
            let time = index as f64 / f64::from(self.fps);
            let report = self.runtime.frame(time)?;
            audio_events.extend(report.audio_events.iter().cloned());
            scheduled_effects.extend(report.scheduled_effects.iter().cloned());
            scheduled_looped_effects.extend(report.scheduled_looped_effects.iter().cloned());
            scheduled_looped_effect_stops
                .extend(report.scheduled_looped_effect_stops.iter().cloned());
            self.last_frame_index = Some(index);
            if index == frame_index {
                let mut report = report;
                report.audio_events = audio_events.clone();
                report.scheduled_effects = scheduled_effects.clone();
                report.scheduled_looped_effects = scheduled_looped_effects.clone();
                report.scheduled_looped_effect_stops = scheduled_looped_effect_stops.clone();
                final_report = Some(report);
            }
        }
        final_report.context("requested Watch frame was not evaluated")
    }
}

impl<'a> FrameSession<'a> {
    pub fn new(
        watch: &'a WatchData,
        rom: &[u8],
        configuration: &serde_json::Value,
        level: &formats::LevelData,
        resources_path: &std::path::Path,
        skin_name: &str,
        background_name: Option<&str>,
        effect_name: Option<&str>,
        particle_name: Option<&str>,
        width: u32,
        height: u32,
        fps: u32,
        level_option_overrides: &[(usize, f64)],
    ) -> Result<Self> {
        Self::new_configured(
            watch,
            rom,
            configuration,
            level,
            resources_path,
            skin_name,
            background_name,
            effect_name,
            particle_name,
            width,
            height,
            fps,
            level_option_overrides,
            true,
            crate::render_ui::RendererUiConfig::default(),
            0.0,
            1.0,
            crate::render::RenderBackend::Cpu,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_configured(
        watch: &'a WatchData,
        rom: &[u8],
        configuration: &serde_json::Value,
        level: &formats::LevelData,
        resources_path: &std::path::Path,
        skin_name: &str,
        background_name: Option<&str>,
        effect_name: Option<&str>,
        particle_name: Option<&str>,
        width: u32,
        height: u32,
        fps: u32,
        level_option_overrides: &[(usize, f64)],
        particles_enabled: bool,
        ui: crate::render_ui::RendererUiConfig,
        ui_segment_start: f64,
        ui_segment_duration: f64,
        backend: crate::render::RenderBackend,
        profile: bool,
    ) -> Result<Self> {
        if width == 0 || height == 0 || width > 8192 || height > 8192 {
            bail!("frame dimensions must be in 1..=8192");
        }
        if fps == 0 || fps > 240 {
            bail!("FPS must be in 1..=240");
        }
        let mut stepper = WatchFrameStepper::new_with_engine_options(
            watch,
            rom,
            configuration,
            level,
            fps,
            level_option_overrides,
        )?;
        let runtime = stepper.runtime_mut();
        runtime.set_draw_tracing(true);
        runtime.set_screen_aspect_ratio(f64::from(width) / f64::from(height))?;
        let skin = formats::load_skin_assets(resources_path, skin_name)?;
        let bindings = skin_bindings(watch)?;
        let sprite_names = skin.sprites.keys().cloned().collect();
        runtime.bind_skin_sprite_names(&sprite_names)?;
        if let Some(name) = effect_name {
            let names = formats::load_effect_clip_names_optional(resources_path, name)
                .with_context(|| format!("resolving selected effect resource {name:?}"))?;
            if watch
                .effect
                .get("clips")
                .and_then(serde_json::Value::as_array)
                .is_some()
            {
                runtime.bind_effect_clip_names(&names.unwrap_or_default())?;
            }
        }
        let mut particles = None;
        let mut particle_bindings = BTreeMap::new();
        if let Some(name) = particle_name {
            let names = formats::load_particle_effect_names(resources_path, name)
                .with_context(|| format!("resolving selected particle resource {name:?}"))?;
            if particles_enabled {
                particles = Some(
                    formats::load_particle_assets(resources_path, name)
                        .with_context(|| format!("loading selected particle resource {name:?}"))?,
                );
            }
            let effect_names: std::collections::BTreeSet<String> = particles
                .as_ref()
                .map(|assets| assets.effects.keys().cloned().collect())
                .unwrap_or_default();
            if watch
                .particle
                .get("effects")
                .and_then(serde_json::Value::as_array)
                .is_some()
            {
                runtime.bind_particle_effect_names(&names)?;
                particle_bindings = particle_effect_bindings(watch)?
                    .into_iter()
                    .filter(|(_, name)| effect_names.contains(name))
                    .collect();
            }
        } else if particles_enabled
            && watch
                .particle
                .get("effects")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|bindings| !bindings.is_empty())
        {
            bail!("engine declares particle effects but no particle resource was selected");
        }
        let background = background_name
            .map(|name| {
                formats::load_background_assets(resources_path, name)
                    .with_context(|| format!("resolving selected background resource {name:?}"))
            })
            .transpose()?;
        if let Some(assets) = background.as_ref() {
            let quad = assets.data.runtime_quad(
                f64::from(width) / f64::from(height),
                f64::from(assets.width) / f64::from(assets.height),
            )?;
            runtime.set_runtime_background_quad(quad)?;
        }
        Ok(Self {
            stepper,
            skin,
            background,
            bindings,
            particles,
            particle_bindings,
            particles_enabled,
            ui,
            ui_segment_start,
            ui_segment_duration,
            width,
            height,
            backend,
            gpu: if backend == crate::render::RenderBackend::Wgpu {
                Some(crate::gpu_render::GpuRenderer::shared()?)
            } else {
                None
            },
            profile,
        })
    }

    /// Advance the deterministic host at every frame from level time zero.
    /// The returned buffer is tightly packed top-to-bottom RGB24.
    pub fn render_global_frame(&mut self, frame_index: u64) -> Result<RenderedFrame> {
        let total_start = self.profile.then(std::time::Instant::now);
        let vm_start = self.profile.then(std::time::Instant::now);
        let report = self.stepper.advance_to(frame_index)?;
        let vm = vm_start.map(|t| t.elapsed()).unwrap_or_default();
        let preparation_start = self.profile.then(std::time::Instant::now);
        let quad: [[f64; 2]; 4] = report
            .runtime_background_quad
            .chunks_exact(2)
            .map(|xy| [xy[0], xy[1]])
            .collect::<Vec<_>>()
            .try_into()
            .map_err(|_| anyhow::anyhow!("Runtime Background quad has invalid length"))?;
        let background = self.background.as_ref().map(|assets| (assets, quad));
        let particle_draws = if self.particles_enabled {
            if let Some(assets) = &self.particles {
                Some(crate::particles::render_instances(
                    assets,
                    &self.particle_bindings,
                    &self
                        .stepper
                        .runtime_mut()
                        .context
                        .particle_instances
                        .read()
                        .map_err(|_| anyhow::anyhow!("particle instance state lock was poisoned"))?
                        .instances()
                        .values()
                        .cloned()
                        .collect::<Vec<_>>(),
                    report.timeline.unwrap_or(0.0),
                )?)
            } else {
                None
            }
        } else {
            None
        };
        let preparation = preparation_start.map(|t| t.elapsed()).unwrap_or_default();
        let render_start = self.profile.then(std::time::Instant::now);
        let mut gpu_profile = self
            .profile
            .then(crate::gpu_render::GpuFrameProfile::default);
        let mut ppm = match self.backend {
            crate::render::RenderBackend::Cpu => report
                .display_list
                .render_skin_ppm_with_runtime_transform_and_background(
                    self.width,
                    self.height,
                    f64::from(self.width) / f64::from(self.height),
                    &self.skin,
                    &self.bindings,
                    &report.runtime_skin_transform,
                    background,
                )?,
            crate::render::RenderBackend::Wgpu => crate::gpu_render::render_display_list(
                self.gpu
                    .as_ref()
                    .context("wgpu backend was not initialized")?,
                &report.display_list,
                self.width,
                self.height,
                f64::from(self.width) / f64::from(self.height),
                &self.skin,
                &self.bindings,
                &report.runtime_skin_transform,
                background,
                self.particles
                    .as_ref()
                    .zip(particle_draws.as_deref())
                    .map(|(assets, draws)| (assets, draws, &report.runtime_particle_transform)),
                gpu_profile.as_mut(),
            )?,
        };
        if self.backend == crate::render::RenderBackend::Cpu {
            if let (Some(assets), Some(draws)) = (&self.particles, particle_draws.as_deref()) {
                let mut rgb = ppm_rgb_payload(&ppm, self.width, self.height)?;
                DisplayList::composite_particle_sprites(
                    &mut rgb,
                    self.width,
                    self.height,
                    f64::from(self.width) / f64::from(self.height),
                    assets,
                    draws,
                    &report.runtime_particle_transform,
                )?;
                ppm = rgb_to_ppm(&rgb, self.width, self.height);
            }
        }
        let cpu_render = render_start.map(|t| t.elapsed()).unwrap_or_default();
        let ui_start = self.profile.then(std::time::Instant::now);
        if self.ui.enabled {
            let mut rgb = ppm_rgb_payload(&ppm, self.width, self.height)?;
            let timeline = report.timeline.unwrap_or(0.0);
            let memory = &self.stepper.runtime_mut().global_memory;
            crate::render_ui::render_overlay(
                &mut rgb,
                self.width,
                self.height,
                timeline,
                self.ui_segment_start,
                self.ui_segment_duration,
                &self.ui,
                |provider| match provider {
                    crate::render_ui::UiValueProvider::Fixed { value }
                    | crate::render_ui::UiValueProvider::External { value } => Ok(*value),
                    crate::render_ui::UiValueProvider::Memory { block_id, index } => {
                        Ok(memory.get(*block_id, *index))
                    }
                    crate::render_ui::UiValueProvider::Progress => {
                        Ok(if self.ui_segment_duration > 0.0 {
                            ((timeline - self.ui_segment_start) / self.ui_segment_duration)
                                .clamp(0.0, 1.0)
                        } else {
                            0.0
                        })
                    }
                    crate::render_ui::UiValueProvider::EngineMetric { key } => {
                        anyhow::bail!(
                            "engine metric provider {key:?} has no authoritative Watch source"
                        )
                    }
                    crate::render_ui::UiValueProvider::Accuracy => {
                        anyhow::bail!(
                            "accuracy provider is unsupported without a verified Watch source"
                        )
                    }
                    crate::render_ui::UiValueProvider::JudgmentDerived { mapping } => {
                        anyhow::bail!("judgment-derived provider {mapping:?} requires a caller-supplied implementation")
                    }
                },
            )?;
            ppm = rgb_to_ppm(&rgb, self.width, self.height);
        }
        let runtime_ui = ui_start.map(|t| t.elapsed()).unwrap_or_default();
        let framebuffer_copy_start = self.profile.then(std::time::Instant::now);
        let rgb = ppm_rgb_payload(&ppm, self.width, self.height)?;
        let framebuffer_copy = framebuffer_copy_start
            .map(|t| t.elapsed())
            .unwrap_or_default();
        let profile = total_start.map(|t| FrameStageProfile {
            vm,
            preparation,
            cpu_render,
            runtime_ui,
            total: t.elapsed(),
            entities: report.runtime_entity_count,
            skin_draws: report.display_list.sprites.len(),
            particle_draws: particle_draws.as_ref().map_or(0, Vec::len),
            gpu_encode_submit: gpu_profile
                .as_ref()
                .map_or(std::time::Duration::ZERO, |p| p.encode_submit),
            gpu_draw_preparation: gpu_profile
                .as_ref()
                .map_or(std::time::Duration::ZERO, |p| p.draw_preparation),
            gpu_wait_map: gpu_profile
                .as_ref()
                .map_or(std::time::Duration::ZERO, |p| p.wait_map),
            framebuffer_unpack: gpu_profile
                .as_ref()
                .map_or(std::time::Duration::ZERO, |p| p.unpack),
            framebuffer_copy,
            gpu_draw_calls: gpu_profile.as_ref().map_or(0, |p| p.draw_calls),
            atlas_uploads: gpu_profile.as_ref().map_or(0, |p| p.atlas_uploads),
            readback_bytes: gpu_profile.as_ref().map_or(0, |p| p.readback_bytes),
            adapter: gpu_profile.and_then(|p| p.adapter),
        });
        Ok(RenderedFrame {
            report,
            rgb,
            profile,
        })
    }
}

fn rgb_to_ppm(rgb: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
    ppm.extend_from_slice(rgb);
    ppm
}

fn skin_bindings(watch: &WatchData) -> Result<BTreeMap<u32, String>> {
    let values = watch
        .skin
        .get("sprites")
        .and_then(serde_json::Value::as_array)
        .context("EngineWatchData skin bindings have no sprites array")?;
    values
        .iter()
        .map(|value| {
            let name = value
                .get("name")
                .and_then(serde_json::Value::as_str)
                .context("skin sprite binding has no name")?;
            let id = value
                .get("id")
                .and_then(serde_json::Value::as_u64)
                .and_then(|id| u32::try_from(id).ok())
                .context("skin sprite binding ID is outside u32 range")?;
            Ok((id, name.to_owned()))
        })
        .collect()
}

pub fn ppm_rgb_payload(ppm: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let mut offset = 0;
    for _ in 0..3 {
        let relative = ppm[offset..]
            .iter()
            .position(|byte| *byte == b'\n')
            .context("renderer returned malformed PPM header")?;
        offset += relative + 1;
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(3))
        .context("RGB frame size overflows address space")?;
    if ppm.len() - offset != expected {
        bail!("renderer returned a PPM with an unexpected RGB payload length");
    }
    Ok(ppm[offset..].to_vec())
}

pub fn hash_draw_commands(display_list: &crate::runtime::DisplayList) -> String {
    use sha1::{Digest, Sha1};
    let mut digest = Sha1::new();
    digest.update((display_list.sprites.len() as u64).to_le_bytes());
    for draw in &display_list.sprites {
        digest.update(draw.sprite_id.to_le_bytes());
        for coordinate in draw.corners.iter().flatten().chain(draw.z.iter()) {
            digest.update(coordinate.to_bits().to_le_bytes());
        }
        digest.update(draw.alpha.to_bits().to_le_bytes());
        if let Some(source) = &draw.provenance {
            digest.update(
                source
                    .entity_id
                    .map(|id| id as u64)
                    .unwrap_or(u64::MAX)
                    .to_le_bytes(),
            );
            digest.update((source.draw_node as u64).to_le_bytes());
            for text in [source.archetype.as_deref(), source.callback.as_deref()] {
                if let Some(text) = text {
                    digest.update((text.len() as u64).to_le_bytes());
                    digest.update(text.as_bytes());
                }
            }
        }
    }
    hex::encode(digest.finalize())
}

pub fn hash_rgb(rgb: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    hex::encode(Sha1::digest(rgb))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_range_uses_global_indices_and_exact_rational_times() {
        let range = FrameRange::new(14.0, 2.0, 12).unwrap();
        assert_eq!(range.first_index, 168);
        assert_eq!(range.frame_count, 24);
        assert_eq!(range.index(0).unwrap(), 168);
        assert_eq!(range.index(23).unwrap(), 191);
        assert_eq!(range.time(180), 15.0);
    }

    #[test]
    fn frame_range_rounds_start_up_to_the_next_frame_boundary() {
        let range = FrameRange::new(1.01, 0.2, 10).unwrap();
        assert_eq!(range.first_index, 11);
        assert_eq!(range.frame_count, 2);
        assert!((range.start_time() - 1.1).abs() < 1e-12);
    }

    #[test]
    fn audio_window_applies_level_bgm_offset_and_initial_silence() {
        assert_eq!(
            audio_window(14.0, 0.05).unwrap(),
            AudioWindow {
                source_start: 14.05,
                leading_silence: 0.0
            }
        );
        assert_eq!(
            audio_window(0.0, -0.05).unwrap(),
            AudioWindow {
                source_start: 0.0,
                leading_silence: 0.05
            }
        );
        assert_eq!(
            audio_window(2.0, -0.25).unwrap(),
            AudioWindow {
                source_start: 1.75,
                leading_silence: 0.0
            }
        );
        assert_eq!(
            audio_window(15.0, -1.109).unwrap(),
            AudioWindow {
                source_start: 13.891,
                leading_silence: 0.0
            }
        );
    }

    #[test]
    fn ppm_payload_is_exact_rgb24_without_header() {
        let ppm = b"P6\n2 1\n255\n\x01\x02\x03\x04\x05\x06";
        assert_eq!(ppm_rgb_payload(ppm, 2, 1).unwrap(), [1, 2, 3, 4, 5, 6]);
        assert!(ppm_rgb_payload(b"P6\n2 1\n255\n\0", 2, 1).is_err());
    }

    #[test]
    fn offline_watch_frames_keep_runtime_update_fresh_and_draws_time_dependent() {
        let watch: WatchData = serde_json::from_value(serde_json::json!({
            "archetypes": [{"name":"Moving", "updateParallel":{"index":12}}],
            "nodes": [
                {"value":1001}, {"value":0}, {"func":"Get", "args":[0,1]},
                {"value":0.2}, {"func":"Multiply", "args":[2,3]},
                {"value":1}, {"func":"Add", "args":[4,5]},
                {"value":-0.5}, {"value":0.5}, {"value":0}, {"value":1},
                {"value":0},
                {"func":"Draw", "args":[11,4,7,6,7,6,8,4,8,9,10]}
            ]
        }))
        .unwrap();
        let level: formats::LevelData = serde_json::from_value(serde_json::json!({
            "entities":[{"archetype":"Moving","data":[]}]
        }))
        .unwrap();
        let mut timeline = WatchFrameStepper::new(&watch, &level, 1).unwrap();
        let first = timeline.advance_to(0).unwrap();
        let second = timeline.advance_to(1).unwrap();
        let third = timeline.advance_to(2).unwrap();

        assert_eq!(first.runtime_update, [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(second.runtime_update, [1.0, 1.0, 1.0, 0.0]);
        assert_eq!(third.runtime_update, [2.0, 1.0, 2.0, 0.0]);
        assert_eq!(second.runtime_update_after_callbacks[0], 1.0);
        assert_eq!(third.runtime_update_after_callbacks[0], 2.0);

        let draw_hashes = [
            hash_draw_commands(&first.display_list),
            hash_draw_commands(&second.display_list),
            hash_draw_commands(&third.display_list),
        ];
        let frame_hashes = [first, second, third].map(|report| {
            let ppm = report.display_list.render_ppm(64, 64).unwrap();
            hash_rgb(&ppm_rgb_payload(&ppm, 64, 64).unwrap())
        });
        assert!(draw_hashes.windows(2).all(|pair| pair[0] != pair[1]));
        assert!(frame_hashes.windows(2).all(|pair| pair[0] != pair[1]));
        assert!(timeline.runtime_mut().entities[0]
            .memory
            .entries_for_block(1001)
            .is_empty());
    }

    #[test]
    fn configured_frame_stepper_applies_overrides_before_preprocessing() {
        let watch: WatchData = serde_json::from_value(serde_json::json!({
            "archetypes": [{"name":"Initialization", "preprocess":{"index":3}}],
            "nodes": [
                {"value":2002}, {"value":0}, {"func":"Get", "args":[0,1]},
                {"func":"DebugLog", "args":[2]}
            ]
        }))
        .unwrap();
        let level: formats::LevelData = serde_json::from_value(serde_json::json!({
            "entities":[{"archetype":"Initialization","data":[]}]
        }))
        .unwrap();
        let configuration = serde_json::json!({"options":[{"def":6.0}]});
        let mut timeline = WatchFrameStepper::new_with_engine_options(
            &watch,
            &[],
            &configuration,
            &level,
            12,
            &[(0, 10.8)],
        )
        .unwrap();

        let frame = timeline.advance_to(0).unwrap();
        assert!(matches!(
            frame.debug_events.as_slice(),
            [crate::runtime::DebugEvent::Log { value: 10.8, .. }]
        ));
    }
}
