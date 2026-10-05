//! Portable Watch node execution primitives.
//!
//! This is the scalar VM/display-list boundary. Platform rendering and host
//! services are deliberately kept out of this module.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, RwLock};

const MAX_EVALUATIONS: usize = 5_000_000;
const MAX_CALL_DEPTH: usize = 2048;
const ENGINE_ROM_BLOCK: i64 = 3000;
const RUNTIME_UPDATE_BLOCK: i64 = 1001;
const RUNTIME_SKIN_TRANSFORM_BLOCK: i64 = 1002;
pub(crate) const RUNTIME_PARTICLE_TRANSFORM_BLOCK: i64 = 1003;
const ENTITY_DATA_ARRAY_BLOCK: i64 = 4101;
const ENTITY_SHARED_MEMORY_ARRAY_BLOCK: i64 = 4102;
const ENTITY_INFO_ARRAY_BLOCK: i64 = 4103;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpriteDraw {
    pub sprite_id: u32,
    /// Sonolus order: bottom-left, top-left, top-right, bottom-right.
    pub corners: [[f64; 2]; 4],
    pub z: [f64; 4],
    pub alpha: f64,
    #[serde(default)]
    pub provenance: Option<DrawProvenance>,
    #[serde(default)]
    pub trace: Option<DrawTrace>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawTrace {
    /// WatchData node IDs corresponding positionally to the Draw arguments.
    pub argument_nodes: Vec<usize>,
    /// Values observed while evaluating the executed argument graph.
    pub node_values: BTreeMap<usize, f64>,
    /// Text form preserves NaN and infinities, which JSON numbers cannot encode.
    #[serde(default)]
    pub node_value_texts: BTreeMap<usize, String>,
    /// Memory reads performed while evaluating the Draw arguments, including
    /// the most recent in-callback write to the same block/index when known.
    pub memory_reads: Vec<MemoryReadTrace>,
    /// Latest writes to memory locations at the point this Draw executes.
    pub memory_writes: Vec<MemoryWriteTrace>,
    /// Compound memory operations with their old value, operand, and result.
    #[serde(default)]
    pub memory_operations: Vec<MemoryOperationTrace>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryOperationTrace {
    pub node: usize,
    pub function: String,
    pub block: i64,
    pub index: usize,
    pub old_value: String,
    pub operand: String,
    pub result: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryReadTrace {
    pub node: usize,
    pub block: i64,
    pub index: usize,
    pub value: f64,
    pub last_write: Option<(usize, f64)>,
    #[serde(default)]
    pub value_text: String,
    #[serde(default)]
    pub last_write_value_text: Option<String>,
    #[serde(default)]
    pub last_write_origin: Option<MemoryWriteOrigin>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryWriteOrigin {
    pub node: usize,
    pub entity_id: Option<usize>,
    pub archetype: Option<String>,
    pub callback: Option<String>,
    pub callback_node: Option<usize>,
    pub value: String,
    #[serde(default)]
    pub operation: Option<MemoryOperationTrace>,
    #[serde(default)]
    pub expression_values: BTreeMap<usize, String>,
    #[serde(default)]
    pub expression_reads: Vec<MemoryReadTrace>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryWriteTrace {
    pub node: usize,
    pub block: i64,
    pub index: usize,
    pub value: f64,
    #[serde(default)]
    pub value_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DrawProvenance {
    pub entity_id: Option<usize>,
    pub archetype: Option<String>,
    pub callback: Option<String>,
    pub callback_node: Option<usize>,
    pub draw_node: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DisplayList {
    pub sprites: Vec<SpriteDraw>,
}

/// Per-command observations for auditing the skin-backed rendering pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkinDrawDiagnostic {
    pub display_list_index: usize,
    pub render_order: usize,
    pub provenance: Option<DrawProvenance>,
    pub sprite_id: u32,
    pub sprite_name: String,
    pub atlas_size: [u32; 2],
    pub atlas_interpolation: bool,
    /// 8x8 linear transform supplied by this skin sprite's SkinData entry.
    pub sprite_transform: [[f64; 8]; 8],
    pub atlas_nontransparent_pixels: u32,
    pub atlas_alpha_range: [u8; 2],
    pub input_corners: [[f64; 2]; 4],
    /// Corners after Runtime Skin Transform and before the sprite Skin Data transform.
    #[serde(default)]
    pub runtime_transformed_corners: [[f64; 2]; 4],
    pub transformed_corners: [[f64; 2]; 4],
    pub screen_pixel_corners: [[f64; 2]; 4],
    pub atlas_rect: [u32; 4],
    /// Atlas positions corresponding to input BL, TL, TR, BR after UV mapping.
    pub atlas_corner_samples: [[f64; 2]; 4],
    pub z: [f64; 4],
    /// Full tuple used by the renderer's painter ordering. The observed
    /// Sonolus behavior establishes lexicographic ordering for ordinary
    /// finite unequal tuples; exceptional float values and exact ties remain
    /// implementation-defined here.
    pub z_order_key: [f64; 4],
    pub alpha: f64,
    pub unclipped_pixel_bounds: [i64; 4],
    pub clipped_pixel_bounds: [i64; 4],
    /// Pixel centers inside the bilinear destination quad and with sampled
    /// nonzero sprite alpha, independent of the sprite's RGB color.
    pub nontransparent_quad_pixels: u64,
    pub sampled_alpha_range: Option<[u8; 2]>,
    pub sampled_rgb_range: Option<[[u8; 2]; 3]>,
}

/// A deferred entity creation requested by WatchData's `Spawn` function.
/// Sonolus queues these until the next spawning system pass.
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnRequest {
    pub archetype_id: i64,
    pub data: Vec<f64>,
}

/// Deferred audio intent emitted by Watch. Times remain in the BGM/gameplay
/// timeline; an audio backend applies its device/audio offset when dispatching.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduledEffect {
    pub clip_id: i64,
    pub time: f64,
    pub minimum_distance: f64,
    pub requested_at: f64,
    pub has_required_lead_time: bool,
}

/// Deferred request to start a looping effect clip on the BGM timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduledLoopedEffect {
    pub instance_id: i64,
    pub clip_id: i64,
    pub start_time: f64,
    pub requested_at: f64,
    pub has_required_lead_time: bool,
}

/// Deferred request to stop one looping effect instance on the BGM timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduledLoopedEffectStop {
    pub instance_id: i64,
    pub end_time: f64,
    pub requested_at: f64,
    pub has_required_lead_time: bool,
}

/// Backend-neutral audio commands emitted by Watch VM calls. These preserve
/// the operation request and timeline time; they do not perform playback.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AudioEffectEvent {
    Play {
        clip_id: i64,
        minimum_distance: f64,
        requested_at: f64,
    },
    StartLoop {
        instance_id: i64,
        clip_id: i64,
        requested_at: f64,
    },
    StopLoop {
        instance_id: i64,
        requested_at: f64,
    },
}

/// Request to destroy a previously spawned particle-effect instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DestroyedParticleEffect {
    pub particle_id: i64,
}

/// Backend-neutral description of a live particle-effect instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleEffectInstance {
    pub instance_id: i64,
    pub effect_id: i64,
    /// Sonolus corner order: bottom-left, top-left, top-right, bottom-right.
    pub corners: [[f64; 2]; 4],
    pub duration: f64,
    pub is_looped: bool,
    pub spawned_at: f64,
}

/// Particle host events emitted by Watch operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ParticleEffectEvent {
    Spawn(ParticleEffectInstance),
    Move {
        instance_id: i64,
        /// Sonolus corner order: bottom-left, top-left, top-right, bottom-right.
        corners: [[f64; 2]; 4],
    },
    Destroy {
        instance_id: i64,
    },
}

/// Headless observations of Sonolus debug operations. `value` remains the
/// original VM `f64`; `value_text` is the JSON-safe display form used by CLI
/// and serialized frame reports for exceptional values.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DebugEvent {
    Log {
        #[serde(skip_serializing)]
        value: f64,
        value_text: String,
        entity_id: Option<usize>,
        archetype: Option<String>,
        callback: Option<String>,
        callback_node: Option<usize>,
        node: usize,
    },
    Pause {
        entity_id: Option<usize>,
        archetype: Option<String>,
        callback: Option<String>,
        callback_node: Option<usize>,
        node: usize,
    },
}

impl DebugEvent {
    pub fn display_value(value: f64) -> String {
        if value.is_nan() {
            "NaN".to_owned()
        } else if value == f64::INFINITY {
            "inf".to_owned()
        } else if value == f64::NEG_INFINITY {
            "-inf".to_owned()
        } else if value == 0.0 && value.is_sign_negative() {
            "-0".to_owned()
        } else {
            value.to_string()
        }
    }
}

/// Shared particle instance allocator and live-instance table for a Watch run.
#[derive(Debug, Default)]
pub struct ParticleEffectState {
    next_instance_id: i64,
    instances: BTreeMap<i64, ParticleEffectInstance>,
}

impl ParticleEffectState {
    pub fn instances(&self) -> &BTreeMap<i64, ParticleEffectInstance> {
        &self.instances
    }
}

impl DisplayList {
    /// Explain the same geometry, resource, UV, screen, and painter-order
    /// conversions used by `render_skin_ppm`, without altering any command.
    pub fn skin_render_diagnostics(
        &self,
        width: u32,
        height: u32,
        aspect_ratio: f64,
        skin: &crate::formats::SkinAssets,
        bindings: &BTreeMap<u32, String>,
    ) -> Result<Vec<SkinDrawDiagnostic>> {
        self.skin_render_diagnostics_with_runtime_transform(
            width,
            height,
            aspect_ratio,
            skin,
            bindings,
            &identity_skin_transform(),
        )
    }

    pub fn skin_render_diagnostics_with_runtime_transform(
        &self,
        width: u32,
        height: u32,
        aspect_ratio: f64,
        skin: &crate::formats::SkinAssets,
        bindings: &BTreeMap<u32, String>,
        runtime_transform: &[f64; 16],
    ) -> Result<Vec<SkinDrawDiagnostic>> {
        if width == 0 || height == 0 || width > 8192 || height > 8192 {
            bail!("frame dimensions must be in 1..=8192");
        }
        if !aspect_ratio.is_finite() || aspect_ratio <= 0.0 {
            bail!("screen aspect ratio must be finite and positive");
        }
        let expected_texture_bytes = (skin.width as usize)
            .checked_mul(skin.height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .context("skin texture dimensions overflow address space")?;
        if skin.width == 0 || skin.height == 0 || skin.rgba.len() != expected_texture_bytes {
            bail!("skin atlas dimensions do not match its RGBA pixel buffer");
        }
        let mut order: Vec<_> = self.sprites.iter().enumerate().collect();
        order.sort_by(|(_, a), (_, b)| compare_z_tuples(&a.z, &b.z));
        let mut diagnostics = Vec::with_capacity(order.len());
        for (render_order, (display_list_index, draw)) in order.into_iter().enumerate() {
            let sprite_name = bindings.get(&draw.sprite_id).with_context(|| {
                format!("Draw references unbound skin sprite ID {}", draw.sprite_id)
            })?;
            let sprite = skin
                .sprites
                .get(sprite_name)
                .with_context(|| format!("skin does not contain bound sprite {sprite_name:?}"))?;
            let right = sprite
                .x
                .checked_add(sprite.width)
                .context("sprite x range overflow")?;
            let bottom = sprite
                .y
                .checked_add(sprite.height)
                .context("sprite y range overflow")?;
            if sprite.width == 0 || sprite.height == 0 || right > skin.width || bottom > skin.height
            {
                bail!("skin sprite {sprite_name:?} has an invalid atlas rectangle");
            }
            let runtime_transformed_corners =
                transform_runtime_skin_corners(draw, runtime_transform);
            let transformed_corners = transform_skin_corners(runtime_transformed_corners, sprite);
            let screen_pixel_corners = transformed_corners.map(|[x, y]| {
                [
                    ((x / aspect_ratio + 1.0) * 0.5) * f64::from(width),
                    ((1.0 - y) * 0.5) * f64::from(height),
                ]
            });
            let min_x = screen_pixel_corners
                .iter()
                .map(|p| p[0])
                .fold(f64::INFINITY, f64::min);
            let max_x = screen_pixel_corners
                .iter()
                .map(|p| p[0])
                .fold(f64::NEG_INFINITY, f64::max);
            let min_y = screen_pixel_corners
                .iter()
                .map(|p| p[1])
                .fold(f64::INFINITY, f64::min);
            let max_y = screen_pixel_corners
                .iter()
                .map(|p| p[1])
                .fold(f64::NEG_INFINITY, f64::max);
            let unclipped_pixel_bounds = [
                min_x.floor() as i64,
                min_y.floor() as i64,
                max_x.ceil() as i64,
                max_y.ceil() as i64,
            ];
            let clipped_pixel_bounds = [
                unclipped_pixel_bounds[0].clamp(0, i64::from(width)),
                unclipped_pixel_bounds[1].clamp(0, i64::from(height)),
                unclipped_pixel_bounds[2].clamp(0, i64::from(width)),
                unclipped_pixel_bounds[3].clamp(0, i64::from(height)),
            ];
            let mut atlas_nontransparent_pixels = 0u32;
            let mut min_alpha = u8::MAX;
            let mut max_alpha = u8::MIN;
            for y in sprite.y..bottom {
                for x in sprite.x..right {
                    let offset = (y as usize * skin.width as usize + x as usize) * 4;
                    let alpha = skin.rgba.get(offset + 3).copied().unwrap_or(0);
                    min_alpha = min_alpha.min(alpha);
                    max_alpha = max_alpha.max(alpha);
                    atlas_nontransparent_pixels += u32::from(alpha > 0);
                }
            }
            let mut nontransparent_quad_pixels = 0u64;
            let mut sampled_alpha_min = u8::MAX;
            let mut sampled_alpha_max = u8::MIN;
            let mut sampled_rgb_min = [u8::MAX; 3];
            let mut sampled_rgb_max = [u8::MIN; 3];
            let mut covered_quad_pixels = 0u64;
            for py in clipped_pixel_bounds[1]..clipped_pixel_bounds[3] {
                for px in clipped_pixel_bounds[0]..clipped_pixel_bounds[2] {
                    let target = [
                        (((px as f64 + 0.5) / f64::from(width)) * 2.0 - 1.0) * aspect_ratio,
                        1.0 - ((py as f64 + 0.5) / f64::from(height)) * 2.0,
                    ];
                    let Some((u, v)) = inverse_bilinear(&transformed_corners, target) else {
                        continue;
                    };
                    if !(-1e-7..=1.0000001).contains(&u) || !(-1e-7..=1.0000001).contains(&v) {
                        continue;
                    }
                    let tex_x =
                        f64::from(sprite.x) + u.clamp(0.0, 1.0) * f64::from(sprite.width) - 0.5;
                    let tex_y = f64::from(sprite.y)
                        + (1.0 - v.clamp(0.0, 1.0)) * f64::from(sprite.height)
                        - 0.5;
                    let sample = sample_skin(skin, sprite, tex_x, tex_y);
                    let sample_alpha = sample[3];
                    sampled_alpha_min = sampled_alpha_min.min(sample_alpha);
                    sampled_alpha_max = sampled_alpha_max.max(sample_alpha);
                    for channel in 0..3 {
                        sampled_rgb_min[channel] = sampled_rgb_min[channel].min(sample[channel]);
                        sampled_rgb_max[channel] = sampled_rgb_max[channel].max(sample[channel]);
                    }
                    covered_quad_pixels += 1;
                    nontransparent_quad_pixels += u64::from(sample_alpha > 0);
                }
            }
            diagnostics.push(SkinDrawDiagnostic {
                display_list_index,
                render_order,
                provenance: draw.provenance.clone(),
                sprite_id: draw.sprite_id,
                sprite_name: sprite_name.clone(),
                atlas_size: [skin.width, skin.height],
                atlas_interpolation: skin.interpolation,
                sprite_transform: sprite.transform,
                atlas_nontransparent_pixels,
                atlas_alpha_range: [min_alpha, max_alpha],
                input_corners: draw.corners,
                runtime_transformed_corners,
                transformed_corners,
                screen_pixel_corners,
                atlas_rect: [sprite.x, sprite.y, sprite.width, sprite.height],
                atlas_corner_samples: [
                    [f64::from(sprite.x) - 0.5, f64::from(bottom) - 0.5],
                    [f64::from(sprite.x) - 0.5, f64::from(sprite.y) - 0.5],
                    [f64::from(right) - 0.5, f64::from(sprite.y) - 0.5],
                    [f64::from(right) - 0.5, f64::from(bottom) - 0.5],
                ],
                z: draw.z,
                z_order_key: draw.z,
                alpha: draw.alpha,
                unclipped_pixel_bounds,
                clipped_pixel_bounds,
                nontransparent_quad_pixels,
                sampled_alpha_range: (covered_quad_pixels > 0)
                    .then_some([sampled_alpha_min, sampled_alpha_max]),
                sampled_rgb_range: (covered_quad_pixels > 0).then(|| {
                    std::array::from_fn(|channel| {
                        [sampled_rgb_min[channel], sampled_rgb_max[channel]]
                    })
                }),
            });
        }
        Ok(diagnostics)
    }

    /// Rasterize the current abstract sprites to a portable RGB PPM frame.
    /// This simple color-ID rasterizer is a diagnostic boundary; it is not yet
    /// a Sonolus skin atlas renderer.
    pub fn render_ppm(&self, width: u32, height: u32) -> Result<Vec<u8>> {
        if width == 0 || height == 0 || width > 8192 || height > 8192 {
            bail!("frame dimensions must be in 1..=8192");
        }
        let pixels = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(3))
            .context("frame dimensions overflow address space")?;
        let mut rgb = vec![0u8; pixels];
        let mut order: Vec<_> = self.sprites.iter().collect();
        order.sort_by(|a, b| compare_z_tuples(&a.z, &b.z));
        for sprite in order {
            let left = sprite
                .corners
                .iter()
                .map(|p| p[0])
                .fold(f64::INFINITY, f64::min);
            let right = sprite
                .corners
                .iter()
                .map(|p| p[0])
                .fold(f64::NEG_INFINITY, f64::max);
            let bottom = sprite
                .corners
                .iter()
                .map(|p| p[1])
                .fold(f64::INFINITY, f64::min);
            let top = sprite
                .corners
                .iter()
                .map(|p| p[1])
                .fold(f64::NEG_INFINITY, f64::max);
            if ![left, right, bottom, top, sprite.alpha]
                .iter()
                .all(|x| x.is_finite())
            {
                continue;
            }
            let x0 = (((left + 1.0) * 0.5 * width as f64).floor() as i64).clamp(0, width as i64);
            let x1 = (((right + 1.0) * 0.5 * width as f64).ceil() as i64).clamp(0, width as i64);
            let y0 = (((1.0 - top) * 0.5 * height as f64).floor() as i64).clamp(0, height as i64);
            let y1 = (((1.0 - bottom) * 0.5 * height as f64).ceil() as i64).clamp(0, height as i64);
            let alpha = sprite.alpha.clamp(0.0, 1.0);
            let color = sprite_color(sprite.sprite_id);
            for y in y0..y1 {
                for x in x0..x1 {
                    let offset = (y as usize * width as usize + x as usize) * 3;
                    for channel in 0..3 {
                        rgb[offset + channel] = (rgb[offset + channel] as f64 * (1.0 - alpha)
                            + color[channel] as f64 * alpha)
                            .round() as u8;
                    }
                }
            }
        }
        let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
        ppm.extend_from_slice(&rgb);
        Ok(ppm)
    }

    /// Rasterize sprites by sampling their actual skin atlas rectangles.
    /// Geometry follows Sonolus' BL, TL, TR, BR quad corner order.
    pub fn render_skin_ppm(
        &self,
        width: u32,
        height: u32,
        aspect_ratio: f64,
        skin: &crate::formats::SkinAssets,
        bindings: &BTreeMap<u32, String>,
    ) -> Result<Vec<u8>> {
        self.render_skin_ppm_with_background(width, height, aspect_ratio, skin, bindings, None)
    }

    /// Render the Sonolus background first, then the unchanged skin display
    /// list using the existing geometry, sampling, alpha, and painter rules.
    pub fn render_skin_ppm_with_background(
        &self,
        width: u32,
        height: u32,
        aspect_ratio: f64,
        skin: &crate::formats::SkinAssets,
        bindings: &BTreeMap<u32, String>,
        background: Option<(&crate::formats::BackgroundAssets, [[f64; 2]; 4])>,
    ) -> Result<Vec<u8>> {
        self.render_skin_ppm_with_runtime_transform_and_background(
            width,
            height,
            aspect_ratio,
            skin,
            bindings,
            &identity_skin_transform(),
            background,
        )
    }

    pub fn render_skin_ppm_with_runtime_transform_and_background(
        &self,
        width: u32,
        height: u32,
        aspect_ratio: f64,
        skin: &crate::formats::SkinAssets,
        bindings: &BTreeMap<u32, String>,
        runtime_transform: &[f64; 16],
        background: Option<(&crate::formats::BackgroundAssets, [[f64; 2]; 4])>,
    ) -> Result<Vec<u8>> {
        let rgb = self.render_skin_rgb_with_runtime_transform_and_background(
            width,
            height,
            aspect_ratio,
            skin,
            bindings,
            runtime_transform,
            background,
        )?;
        Ok(rgb_to_ppm(&rgb, width, height))
    }

    /// Rasterize the skin and background directly to tightly packed RGB24.
    /// Call the PPM API when a serialized P6 image is part of the output contract.
    pub fn render_skin_rgb_with_runtime_transform_and_background(
        &self,
        width: u32,
        height: u32,
        aspect_ratio: f64,
        skin: &crate::formats::SkinAssets,
        bindings: &BTreeMap<u32, String>,
        runtime_transform: &[f64; 16],
        background: Option<(&crate::formats::BackgroundAssets, [[f64; 2]; 4])>,
    ) -> Result<Vec<u8>> {
        self.render_skin_rgb_with_mode(
            width,
            height,
            aspect_ratio,
            skin,
            bindings,
            runtime_transform,
            background,
            crate::skin_render_mode::SkinRenderMode::Standard,
        )
    }

    pub fn render_skin_rgb_with_mode(
        &self,
        width: u32,
        height: u32,
        aspect_ratio: f64,
        skin: &crate::formats::SkinAssets,
        bindings: &BTreeMap<u32, String>,
        runtime_transform: &[f64; 16],
        background: Option<(&crate::formats::BackgroundAssets, [[f64; 2]; 4])>,
        mode: crate::skin_render_mode::SkinRenderMode,
    ) -> Result<Vec<u8>> {
        if width == 0 || height == 0 || width > 8192 || height > 8192 {
            bail!("frame dimensions must be in 1..=8192");
        }
        if !aspect_ratio.is_finite() || aspect_ratio <= 0.0 {
            bail!("screen aspect ratio must be finite and positive");
        }
        let expected_texture_bytes = (skin.width as usize)
            .checked_mul(skin.height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .context("skin texture dimensions overflow address space")?;
        if skin.width == 0 || skin.height == 0 || skin.rgba.len() != expected_texture_bytes {
            bail!("skin atlas dimensions do not match its RGBA pixel buffer");
        }
        let pixel_count = (width as usize)
            .checked_mul(height as usize)
            .context("frame dimensions overflow address space")?;
        let byte_count = pixel_count
            .checked_mul(3)
            .context("frame dimensions overflow address space")?;
        let mut rgb = vec![0u8; byte_count];
        if let Some((background, quad)) = background {
            render_background(&mut rgb, width, height, aspect_ratio, background, quad)?;
        }
        let mut order: Vec<_> = self.sprites.iter().collect();
        order.sort_by(|a, b| compare_z_tuples(&a.z, &b.z));
        for draw in order {
            let sprite_name = bindings.get(&draw.sprite_id).with_context(|| {
                format!("Draw references unbound skin sprite ID {}", draw.sprite_id)
            })?;
            let sprite = skin
                .sprites
                .get(sprite_name)
                .with_context(|| format!("skin does not contain bound sprite {sprite_name:?}"))?;
            let sprite_right = sprite.x.checked_add(sprite.width);
            let sprite_bottom = sprite.y.checked_add(sprite.height);
            if sprite.width == 0
                || sprite.height == 0
                || sprite_right.is_none_or(|right| right > skin.width)
                || sprite_bottom.is_none_or(|bottom| bottom > skin.height)
            {
                bail!("skin sprite {sprite_name:?} has an invalid atlas rectangle");
            }
            if !draw.alpha.is_finite()
                || !draw.corners.iter().flatten().all(|value| value.is_finite())
                || !draw.z.iter().all(|value| value.is_finite())
            {
                continue;
            }
            let runtime_corners = transform_runtime_skin_corners(draw, runtime_transform);
            let corners = transform_skin_corners(runtime_corners, sprite);
            let projective_weights = if mode == crate::skin_render_mode::SkinRenderMode::Lightweight
            {
                crate::skin_render_mode::projective_weights(&corners)
            } else {
                [0.0; 4]
            };
            if !corners.iter().flatten().all(|value| value.is_finite()) {
                continue;
            }
            let left = corners.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min);
            let right = corners
                .iter()
                .map(|p| p[0])
                .fold(f64::NEG_INFINITY, f64::max);
            let bottom = corners.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
            let top = corners
                .iter()
                .map(|p| p[1])
                .fold(f64::NEG_INFINITY, f64::max);
            let x0 = (((left / aspect_ratio + 1.0) * 0.5 * width as f64).floor() as i64)
                .clamp(0, width as i64);
            let x1 = (((right / aspect_ratio + 1.0) * 0.5 * width as f64).ceil() as i64)
                .clamp(0, width as i64);
            let y0 = (((1.0 - top) * 0.5 * height as f64).floor() as i64).clamp(0, height as i64);
            let y1 = (((1.0 - bottom) * 0.5 * height as f64).ceil() as i64).clamp(0, height as i64);
            let alpha = draw.alpha.clamp(0.0, 1.0);
            if alpha == 0.0 {
                continue;
            }
            for py in y0..y1 {
                for px in x0..x1 {
                    let target = [
                        (((px as f64 + 0.5) / width as f64) * 2.0 - 1.0) * aspect_ratio,
                        1.0 - ((py as f64 + 0.5) / height as f64) * 2.0,
                    ];
                    let uv = match mode {
                        crate::skin_render_mode::SkinRenderMode::Standard => {
                            inverse_bilinear(&corners, target)
                        }
                        crate::skin_render_mode::SkinRenderMode::Lightweight => {
                            crate::skin_render_mode::projective_uv(
                                &corners,
                                &projective_weights,
                                [
                                    f64::from(
                                        (((px as f32 + 0.5) / width as f32) * 2.0 - 1.0)
                                            * aspect_ratio as f32,
                                    ),
                                    f64::from(1.0 - ((py as f32 + 0.5) / height as f32) * 2.0),
                                ],
                            )
                        }
                    };
                    let Some((u, v)) = uv else {
                        continue;
                    };
                    // Lightweight coverage was already decided in triangle space.
                    // Re-testing projective UV amplifies tiny edge errors by q.
                    if mode == crate::skin_render_mode::SkinRenderMode::Standard
                        && (!(-1e-7..=1.0000001).contains(&u) || !(-1e-7..=1.0000001).contains(&v))
                    {
                        continue;
                    }
                    let tex_x = sprite.x as f64 + u.clamp(0.0, 1.0) * sprite.width as f64 - 0.5;
                    let tex_y =
                        sprite.y as f64 + (1.0 - v.clamp(0.0, 1.0)) * sprite.height as f64 - 0.5;
                    let sample = sample_skin(skin, sprite, tex_x, tex_y);
                    let source_alpha = alpha * f64::from(sample[3]) / 255.0;
                    let offset = (py as usize * width as usize + px as usize) * 3;
                    for channel in 0..3 {
                        rgb[offset + channel] = (f64::from(rgb[offset + channel])
                            * (1.0 - source_alpha)
                            + f64::from(sample[channel]) * source_alpha)
                            .round() as u8;
                    }
                }
            }
        }
        Ok(rgb)
    }

    /// Composite particle atlas sprites above an already rendered RGB frame.
    pub fn composite_particle_sprites(
        rgb: &mut [u8],
        width: u32,
        height: u32,
        aspect_ratio: f64,
        assets: &crate::formats::ParticleAssets,
        draws: &[crate::particles::ParticleSpriteDraw],
        runtime_transform: &[f64; 16],
    ) -> Result<()> {
        if width == 0 || height == 0 || !aspect_ratio.is_finite() || aspect_ratio <= 0.0 {
            bail!("particle render dimensions and aspect ratio must be positive");
        }
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(3))
            .context("particle target frame size overflows")?;
        if rgb.len() != expected {
            bail!("particle target frame has an invalid RGB buffer length");
        }
        if assets.width == 0
            || assets.height == 0
            || assets.rgba.len()
                != (assets.width as usize)
                    .checked_mul(assets.height as usize)
                    .and_then(|pixels| pixels.checked_mul(4))
                    .context("particle atlas size overflows")?
        {
            bail!("particle atlas dimensions do not match its pixel buffer");
        }
        for draw in draws {
            let sprite = assets.sprites.get(draw.sprite_id).with_context(|| {
                format!("particle references missing sprite {}", draw.sprite_id)
            })?;
            if draw.alpha <= 0.0
                || !draw.alpha.is_finite()
                || !draw.corners.iter().flatten().all(|value| value.is_finite())
            {
                continue;
            }
            let corners = transform_runtime_skin_corners(
                &SpriteDraw {
                    sprite_id: 0,
                    corners: draw.corners,
                    z: [0.0; 4],
                    alpha: draw.alpha,
                    provenance: None,
                    trace: None,
                },
                runtime_transform,
            );
            let left = corners
                .iter()
                .map(|point| point[0])
                .fold(f64::INFINITY, f64::min);
            let right = corners
                .iter()
                .map(|point| point[0])
                .fold(f64::NEG_INFINITY, f64::max);
            let bottom = corners
                .iter()
                .map(|point| point[1])
                .fold(f64::INFINITY, f64::min);
            let top = corners
                .iter()
                .map(|point| point[1])
                .fold(f64::NEG_INFINITY, f64::max);
            let x0 = (((left / aspect_ratio + 1.0) * 0.5 * f64::from(width)).floor() as i64)
                .clamp(0, i64::from(width));
            let x1 = (((right / aspect_ratio + 1.0) * 0.5 * f64::from(width)).ceil() as i64)
                .clamp(0, i64::from(width));
            let y0 = (((1.0 - top) * 0.5 * f64::from(height)).floor() as i64)
                .clamp(0, i64::from(height));
            let y1 = (((1.0 - bottom) * 0.5 * f64::from(height)).ceil() as i64)
                .clamp(0, i64::from(height));
            for py in y0..y1 {
                for px in x0..x1 {
                    let target = [
                        (((px as f64 + 0.5) / f64::from(width)) * 2.0 - 1.0) * aspect_ratio,
                        1.0 - ((py as f64 + 0.5) / f64::from(height)) * 2.0,
                    ];
                    let Some((u, v)) = inverse_bilinear(&corners, target) else {
                        continue;
                    };
                    if !(-1e-7..=1.0000001).contains(&u) || !(-1e-7..=1.0000001).contains(&v) {
                        continue;
                    }
                    let tex_x =
                        f64::from(sprite.x) + u.clamp(0.0, 1.0) * f64::from(sprite.width) - 0.5;
                    let tex_y = f64::from(sprite.y)
                        + (1.0 - v.clamp(0.0, 1.0)) * f64::from(sprite.height)
                        - 0.5;
                    let sample = sample_particle_sprite(assets, sprite, tex_x, tex_y);
                    let source_alpha = draw.alpha.clamp(0.0, 1.0) * f64::from(sample[3]) / 255.0;
                    let offset = (py as usize * width as usize + px as usize) * 3;
                    for channel in 0..3 {
                        let tinted =
                            f64::from(sample[channel]) * f64::from(draw.color[channel]) / 255.0;
                        rgb[offset + channel] = (f64::from(rgb[offset + channel])
                            * (1.0 - source_alpha)
                            + tinted * source_alpha)
                            .round() as u8;
                    }
                }
            }
        }
        Ok(())
    }
}

fn sample_particle_sprite(
    assets: &crate::formats::ParticleAssets,
    sprite: &crate::formats::ParticleSpriteAsset,
    x: f64,
    y: f64,
) -> [u8; 4] {
    let get = |x: i64, y: i64| {
        let x = x.clamp(sprite.x as i64, (sprite.x + sprite.width - 1) as i64) as usize;
        let y = y.clamp(sprite.y as i64, (sprite.y + sprite.height - 1) as i64) as usize;
        let offset = (y * assets.width as usize + x) * 4;
        assets.rgba[offset..offset + 4].try_into().unwrap_or([0; 4])
    };
    if !assets.interpolation {
        return get((x + 0.5).floor() as i64, (y + 0.5).floor() as i64);
    }
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = x - x0;
    let fy = y - y0;
    let samples = [
        get(x0 as i64, y0 as i64),
        get(x0 as i64 + 1, y0 as i64),
        get(x0 as i64, y0 as i64 + 1),
        get(x0 as i64 + 1, y0 as i64 + 1),
    ];
    let weights = [
        (1.0 - fx) * (1.0 - fy),
        fx * (1.0 - fy),
        (1.0 - fx) * fy,
        fx * fy,
    ];
    std::array::from_fn(|channel| {
        (0..4)
            .map(|index| f64::from(samples[index][channel]) * weights[index])
            .sum::<f64>()
            .round() as u8
    })
}

fn render_background(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    aspect_ratio: f64,
    background: &crate::formats::BackgroundAssets,
    quad: [[f64; 2]; 4],
) -> Result<()> {
    if background.width == 0 || background.height == 0 {
        bail!("background image dimensions must be positive");
    }
    let expected = (background.width as usize)
        .checked_mul(background.height as usize)
        .and_then(|count| count.checked_mul(4))
        .context("background image dimensions overflow address space")?;
    if background.rgba.len() != expected {
        bail!("background image dimensions do not match its RGBA pixel buffer");
    }
    if background.configuration.blur != 0.0 {
        bail!("nonzero Sonolus background blur is not yet supported");
    }
    if !quad
        .iter()
        .flatten()
        .all(|coordinate| coordinate.is_finite())
    {
        bail!("Runtime Background quad contains a non-finite coordinate");
    }
    let base = crate::formats::parse_background_color(&background.data.color, false)?;
    let mask = crate::formats::parse_background_color(&background.configuration.mask, true)?;
    for pixel in rgb.chunks_exact_mut(3) {
        pixel.copy_from_slice(&base[..3]);
    }
    for py in 0..height {
        for px in 0..width {
            let target = [
                (((f64::from(px) + 0.5) / f64::from(width)) * 2.0 - 1.0) * aspect_ratio,
                1.0 - ((f64::from(py) + 0.5) / f64::from(height)) * 2.0,
            ];
            let Some((u, v)) = inverse_bilinear(&quad, target) else {
                continue;
            };
            if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
                continue;
            }
            let sample = sample_rgba_bilinear(
                &background.rgba,
                background.width,
                background.height,
                u * f64::from(background.width - 1),
                (1.0 - v) * f64::from(background.height - 1),
            );
            let pixel = &mut rgb[(py as usize * width as usize + px as usize) * 3..][..3];
            blend_rgb(pixel, sample, f64::from(sample[3]) / 255.0);
        }
    }
    for pixel in rgb.chunks_exact_mut(3) {
        blend_rgb(pixel, mask, f64::from(mask[3]) / 255.0);
    }
    Ok(())
}

/// Rasterize only the Sonolus background layer, using the same quad sampling
/// path as gameplay frame composition. Useful for deterministic diagnostics.
pub fn render_background_ppm(
    width: u32,
    height: u32,
    aspect_ratio: f64,
    background: &crate::formats::BackgroundAssets,
    quad: [[f64; 2]; 4],
) -> Result<Vec<u8>> {
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        bail!("frame dimensions must be in 1..=8192");
    }
    if !aspect_ratio.is_finite() || aspect_ratio <= 0.0 {
        bail!("screen aspect ratio must be finite and positive");
    }
    let byte_count = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(3))
        .context("frame dimensions overflow address space")?;
    let mut rgb = vec![0u8; byte_count];
    render_background(&mut rgb, width, height, aspect_ratio, background, quad)?;
    let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
    ppm.extend_from_slice(&rgb);
    Ok(ppm)
}

fn rgb_to_ppm(rgb: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
    ppm.extend_from_slice(rgb);
    ppm
}

fn sample_rgba_bilinear(rgba: &[u8], width: u32, height: u32, x: f64, y: f64) -> [u8; 4] {
    let x0 = x.floor().clamp(0.0, f64::from(width - 1)) as u32;
    let y0 = y.floor().clamp(0.0, f64::from(height - 1)) as u32;
    let x1 = (x0 + 1).min(width - 1);
    let y1 = (y0 + 1).min(height - 1);
    let tx = x - f64::from(x0);
    let ty = y - f64::from(y0);
    std::array::from_fn(|channel| {
        let at =
            |px: u32, py: u32| rgba[(py as usize * width as usize + px as usize) * 4 + channel];
        let top = f64::from(at(x0, y0)) * (1.0 - tx) + f64::from(at(x1, y0)) * tx;
        let bottom = f64::from(at(x0, y1)) * (1.0 - tx) + f64::from(at(x1, y1)) * tx;
        (top * (1.0 - ty) + bottom * ty).round() as u8
    })
}

// Reuse the actual CPU sampler for focused compositing diagnostics.
#[cfg(test)]
pub(crate) fn skin_sample_at_pixel(
    draw: &SpriteDraw,
    skin: &crate::formats::SkinAssets,
    name: &str,
    matrix: &[f64; 16],
    width: u32,
    height: u32,
    px: u32,
    py: u32,
) -> Option<[u8; 4]> {
    skin_sample_at_pixel_in_mode(
        draw,
        skin,
        name,
        matrix,
        width,
        height,
        px,
        py,
        crate::skin_render_mode::SkinRenderMode::Standard,
    )
}

#[cfg(test)]
pub(crate) fn skin_sample_at_pixel_in_mode(
    draw: &SpriteDraw,
    skin: &crate::formats::SkinAssets,
    name: &str,
    matrix: &[f64; 16],
    width: u32,
    height: u32,
    px: u32,
    py: u32,
    mode: crate::skin_render_mode::SkinRenderMode,
) -> Option<[u8; 4]> {
    let sprite = &skin.sprites[name];
    let corners = transform_skin_corners(transform_runtime_skin_corners(draw, matrix), sprite);
    let coord = [
        ((f64::from(px) + 0.5) / f64::from(width) * 2.0 - 1.0) * f64::from(width)
            / f64::from(height),
        1.0 - (f64::from(py) + 0.5) / f64::from(height) * 2.0,
    ];
    let coord = if mode == crate::skin_render_mode::SkinRenderMode::Lightweight {
        [
            f64::from(
                (((px as f32 + 0.5) / width as f32) * 2.0 - 1.0)
                    * (width as f64 / height as f64) as f32,
            ),
            f64::from(1.0 - ((py as f32 + 0.5) / height as f32) * 2.0),
        ]
    } else {
        coord
    };
    let (u, v) = match mode {
        crate::skin_render_mode::SkinRenderMode::Standard => inverse_bilinear(&corners, coord),
        crate::skin_render_mode::SkinRenderMode::Lightweight => {
            crate::skin_render_mode::projective_uv(
                &corners,
                &crate::skin_render_mode::projective_weights(&corners),
                coord,
            )
        }
    }?;
    if mode == crate::skin_render_mode::SkinRenderMode::Standard
        && (!(-1e-7..=1.0000001).contains(&u) || !(-1e-7..=1.0000001).contains(&v))
    {
        return None;
    }
    Some(sample_skin(
        skin,
        sprite,
        f64::from(sprite.x) + u.clamp(0.0, 1.0) * f64::from(sprite.width) - 0.5,
        f64::from(sprite.y) + (1.0 - v.clamp(0.0, 1.0)) * f64::from(sprite.height) - 0.5,
    ))
}

fn blend_rgb(target: &mut [u8], source: [u8; 4], alpha: f64) {
    for channel in 0..3 {
        target[channel] = (f64::from(target[channel]) * (1.0 - alpha)
            + f64::from(source[channel]) * alpha)
            .round() as u8;
    }
}

/// Compare Draw z tuples in component order. `total_cmp` gives the Rust
/// renderer a deterministic ordering for all f64 bit patterns; the real-client
/// conformance observations establish the larger-first-difference behavior
/// only for ordinary finite, unequal tuples. Exact ties remain equal so the
/// stable `sort_by` calls preserve Draw submission order.
pub fn compare_z_tuples(left: &[f64; 4], right: &[f64; 4]) -> std::cmp::Ordering {
    for (left, right) in left.iter().zip(right.iter()) {
        let ordering = left.total_cmp(right);
        if ordering != std::cmp::Ordering::Equal {
            return ordering;
        }
    }
    std::cmp::Ordering::Equal
}

fn identity_skin_transform() -> [f64; 16] {
    std::array::from_fn(|index| if index % 5 == 0 { 1.0 } else { 0.0 })
}

fn transform_runtime_skin_corners(draw: &SpriteDraw, matrix: &[f64; 16]) -> [[f64; 2]; 4] {
    // Watch Draw corners are 2D screen positions; the Z tuple remains the
    // painter-order input. Apply the first two rows of Runtime Skin Transform
    // to each XY corner before the per-sprite Skin Data transform.
    std::array::from_fn(|corner| {
        let [x, y] = draw.corners[corner];
        [
            x * matrix[0] + y * matrix[1] + matrix[2] + matrix[3],
            x * matrix[4] + y * matrix[5] + matrix[6] + matrix[7],
        ]
    })
}

pub(crate) fn gpu_transform_skin_corners(
    draw: &SpriteDraw,
    matrix: &[f64; 16],
    sprite: &crate::formats::SkinSpriteAsset,
) -> [[f64; 2]; 4] {
    transform_skin_corners(transform_runtime_skin_corners(draw, matrix), sprite)
}

pub(crate) fn gpu_transform_particle_corners(
    draw: &crate::particles::ParticleSpriteDraw,
    matrix: &[f64; 16],
) -> [[f64; 2]; 4] {
    transform_runtime_skin_corners(
        &SpriteDraw {
            sprite_id: 0,
            corners: draw.corners,
            z: [0.0; 4],
            alpha: draw.alpha,
            provenance: None,
            trace: None,
        },
        matrix,
    )
}

fn transform_skin_corners(
    input_corners: [[f64; 2]; 4],
    sprite: &crate::formats::SkinSpriteAsset,
) -> [[f64; 2]; 4] {
    let input = [
        input_corners[0][0],
        input_corners[0][1],
        input_corners[1][0],
        input_corners[1][1],
        input_corners[2][0],
        input_corners[2][1],
        input_corners[3][0],
        input_corners[3][1],
    ];
    std::array::from_fn(|vertex| {
        [
            dot8(&sprite.transform[vertex * 2], &input),
            dot8(&sprite.transform[vertex * 2 + 1], &input),
        ]
    })
}

fn dot8(coefficients: &[f64; 8], values: &[f64; 8]) -> f64 {
    coefficients.iter().zip(values).map(|(a, b)| a * b).sum()
}

fn inverse_bilinear(corners: &[[f64; 2]; 4], target: [f64; 2]) -> Option<(f64, f64)> {
    // Sonolus corner order is bottom-left, top-left, top-right, bottom-right.
    let p00 = corners[0];
    let p10 = corners[3];
    let p01 = corners[1];
    let p11 = corners[2];
    let cross = [
        p11[0] - p10[0] - p01[0] + p00[0],
        p11[1] - p10[1] - p01[1] + p00[1],
    ];
    let mut u = 0.5;
    let mut v = 0.5;
    for _ in 0..12 {
        let point = [
            p00[0] + u * (p10[0] - p00[0]) + v * (p01[0] - p00[0]) + u * v * cross[0],
            p00[1] + u * (p10[1] - p00[1]) + v * (p01[1] - p00[1]) + u * v * cross[1],
        ];
        let error = [point[0] - target[0], point[1] - target[1]];
        if error[0].abs().max(error[1].abs()) < 1e-7 {
            return Some((u, v));
        }
        let du = [
            p10[0] - p00[0] + v * cross[0],
            p10[1] - p00[1] + v * cross[1],
        ];
        let dv = [
            p01[0] - p00[0] + u * cross[0],
            p01[1] - p00[1] + u * cross[1],
        ];
        let determinant = du[0] * dv[1] - dv[0] * du[1];
        if determinant.abs() < 1e-12 {
            return None;
        }
        u -= (error[0] * dv[1] - dv[0] * error[1]) / determinant;
        v -= (du[0] * error[1] - error[0] * du[1]) / determinant;
    }
    let point = [
        p00[0] + u * (p10[0] - p00[0]) + v * (p01[0] - p00[0]) + u * v * cross[0],
        p00[1] + u * (p10[1] - p00[1]) + v * (p01[1] - p00[1]) + u * v * cross[1],
    ];
    ((point[0] - target[0])
        .abs()
        .max((point[1] - target[1]).abs())
        < 1e-5)
        .then_some((u, v))
}

fn sample_skin(
    skin: &crate::formats::SkinAssets,
    sprite: &crate::formats::SkinSpriteAsset,
    x: f64,
    y: f64,
) -> [u8; 4] {
    let get = |x: i64, y: i64| -> [u8; 4] {
        let x = x.clamp(sprite.x as i64, (sprite.x + sprite.width - 1) as i64) as usize;
        let y = y.clamp(sprite.y as i64, (sprite.y + sprite.height - 1) as i64) as usize;
        let offset = (y * skin.width as usize + x) * 4;
        skin.rgba[offset..offset + 4]
            .try_into()
            .unwrap_or([0, 0, 0, 0])
    };
    if !skin.interpolation {
        return get((x + 0.5).floor() as i64, (y + 0.5).floor() as i64);
    }
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = x - x0;
    let fy = y - y0;
    let samples = [
        get(x0 as i64, y0 as i64),
        get(x0 as i64 + 1, y0 as i64),
        get(x0 as i64, y0 as i64 + 1),
        get(x0 as i64 + 1, y0 as i64 + 1),
    ];
    let weights = [
        (1.0 - fx) * (1.0 - fy),
        fx * (1.0 - fy),
        (1.0 - fx) * fy,
        fx * fy,
    ];
    let mut result = [0u8; 4];
    for channel in 0..4 {
        result[channel] = samples
            .iter()
            .zip(weights)
            .map(|(pixel, weight)| f64::from(pixel[channel]) * weight)
            .sum::<f64>()
            .round() as u8;
    }
    result
}

fn sprite_color(id: u32) -> [u8; 3] {
    let value = id.wrapping_mul(0x9e3779b9);
    [
        64 + ((value >> 16) as u8 & 0xbf),
        64 + ((value >> 8) as u8 & 0xbf),
        64 + (value as u8 & 0xbf),
    ]
}

#[derive(Debug, Clone, Default)]
pub struct Memory {
    values: BTreeMap<(i64, usize), f64>,
}

impl Memory {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn get(&self, block: i64, index: usize) -> f64 {
        self.values.get(&(block, index)).copied().unwrap_or(0.0)
    }

    pub(crate) fn len(&self) -> usize {
        self.values.len()
    }

    pub fn set(&mut self, block: i64, index: usize, value: f64) {
        self.values.insert((block, index), value);
    }

    pub fn entries_for_block(&self, block: i64) -> Vec<(usize, f64)> {
        self.values
            .iter()
            .filter_map(|(&(key, index), &value)| (key == block).then_some((index, value)))
            .collect()
    }

    pub fn retain_other_than(&mut self, block: i64) {
        self.values.retain(|(key, _), _| *key != block);
    }

    pub fn retain_only(&mut self, block: i64) {
        self.values.retain(|(key, _), _| *key == block);
    }

    pub fn overlay(&mut self, source: &Memory) {
        self.values
            .extend(source.values.iter().map(|(k, v)| (*k, *v)));
    }

    pub(crate) fn into_watch_entity_parts(
        mut self,
        entity_memory_block: i64,
        excluded_global_blocks: &[i64],
    ) -> (Self, Self) {
        let mut entity_and_later = self.values.split_off(&(entity_memory_block, 0));
        let mut later = entity_and_later.split_off(&(entity_memory_block.saturating_add(1), 0));
        later.retain(|(block, _), _| !excluded_global_blocks.contains(block));
        self.values.append(&mut later);
        (
            self,
            Self {
                values: entity_and_later,
            },
        )
    }
}

pub struct WatchVm<'a> {
    nodes: &'a [crate::watch::EngineNode],
    pub memory: Memory,
    pub display_list: DisplayList,
    /// Entities requested by `Spawn`; a Watch host drains this at its next
    /// spawning-system boundary.
    pub spawn_queue: Vec<SpawnRequest>,
    pub scheduled_effects: Vec<ScheduledEffect>,
    pub scheduled_looped_effects: Vec<ScheduledLoopedEffect>,
    pub scheduled_looped_effect_stops: Vec<ScheduledLoopedEffectStop>,
    pub audio_events: Vec<AudioEffectEvent>,
    pub destroyed_particle_effects: Vec<DestroyedParticleEffect>,
    pub particle_events: Vec<ParticleEffectEvent>,
    pub debug_events: Vec<DebugEvent>,
    evaluations: usize,
    depth: usize,
    block_depth: usize,
    break_signal: Option<(usize, f64)>,
    pub context: VmContext,
    pub function_counts: BTreeMap<String, u64>,
    pub skin_checks: Vec<(i64, bool)>,
    /// Optional bounded VM execution diagnostics.
    diagnostics: Option<VmDiagnostics>,
    loop_trace: Vec<LoopTraceFrame>,
    execution_trace_frames: Vec<(usize, Vec<Option<String>>)>,
    max_evaluations: usize,
    trace_draws: bool,
    capture_draw_argument_values: bool,
    profiling: bool,
    collect_accounting: bool,
    pub(crate) profile_function_dispatches: u64,
    draw_argument_values: BTreeMap<usize, f64>,
    draw_argument_value_texts: BTreeMap<usize, String>,
    draw_memory_reads: Vec<MemoryReadTrace>,
    draw_memory_operations: Vec<MemoryOperationTrace>,
    last_memory_writes: BTreeMap<(i64, usize), (usize, f64)>,
}

struct VmDiagnostics {
    capacity: usize,
    capture_after_evaluation: usize,
    events: VecDeque<String>,
}

#[derive(Clone, Copy)]
struct LoopTraceFrame {
    node: usize,
    iteration: usize,
    branch: usize,
}

#[derive(Debug, Clone, Default)]
pub struct VmContext {
    pub execution_trace: Option<crate::watch_diagnostics::SharedWatchTrace>,
    pub time: f64,
    pub beat: f64,
    pub timescale: f64,
    pub starting_time: f64,
    pub starting_beat: f64,
    pub skin_sprites: BTreeSet<u32>,
    pub effect_clips: BTreeSet<u32>,
    pub particle_effects: BTreeSet<u32>,
    /// Runtime Background quad in Sonolus BL, TL, TR, BR order, flattened x/y.
    pub runtime_background: Arc<RwLock<[f64; 8]>>,
    /// Live particle instances are shared across callback VMs in a Watch run.
    pub particle_instances: Arc<RwLock<ParticleEffectState>>,
    /// Instance IDs are shared by all callback VMs in a Watch run.
    pub next_looped_effect_id: Arc<AtomicI64>,
    pub streams: BTreeMap<(i64, i64), Vec<(f64, f64)>>,
    pub time_map: Vec<(f64, f64)>,
    pub timescale_map: Vec<(f64, f64)>,
    pub bpm_map: Vec<(f64, f64)>,
    /// Shared diagnostic provenance for memory values that survive between
    /// callbacks. This map is only read or written with Draw tracing enabled.
    pub memory_write_origins: Arc<RwLock<BTreeMap<(Option<usize>, i64, usize), MemoryWriteOrigin>>>,
    /// Immutable Engine Rom values shared cheaply by each callback VM.
    pub engine_rom: Arc<Vec<f64>>,
    /// Live, 32-slot rows of entity data indexed by entity ID.
    pub entity_data_array: Arc<RwLock<Vec<f64>>>,
    /// Host-shared 32-slot rows for non-spawned level entities.
    pub entity_shared_memory_array: Arc<RwLock<Vec<f64>>>,
    /// Three-value rows: entity index, archetype index, and active flag.
    pub entity_info_array: Arc<RwLock<Vec<f64>>>,
    pub entity_archetype: Option<String>,
    pub callback_name: Option<String>,
    pub callback_node: Option<usize>,
    /// Current entity's Entity Info block, unavailable to spawned entities.
    pub entity_info: Option<[f64; 3]>,
    /// Lifecycle permissions for mutable host blocks.
    pub lifecycle_stage: Option<u8>,
    /// Current non-spawned entity ID for entity-local block views.
    pub entity_id: Option<usize>,
    pub has_entity_data: bool,
    pub has_entity_shared_memory: bool,
    /// Entity Data Array writes are permitted only during preprocessing.
    pub entity_data_array_writable: bool,
}

impl<'a> WatchVm<'a> {
    pub fn new(nodes: &'a [crate::watch::EngineNode]) -> Self {
        Self {
            nodes,
            memory: Memory::default(),
            display_list: DisplayList::default(),
            spawn_queue: Vec::new(),
            scheduled_effects: Vec::new(),
            scheduled_looped_effects: Vec::new(),
            scheduled_looped_effect_stops: Vec::new(),
            audio_events: Vec::new(),
            destroyed_particle_effects: Vec::new(),
            particle_events: Vec::new(),
            debug_events: Vec::new(),
            evaluations: 0,
            depth: 0,
            block_depth: 0,
            break_signal: None,
            context: VmContext {
                timescale: 1.0,
                ..VmContext::default()
            },
            function_counts: BTreeMap::new(),
            skin_checks: Vec::new(),
            diagnostics: None,
            loop_trace: Vec::new(),
            execution_trace_frames: Vec::new(),
            max_evaluations: MAX_EVALUATIONS,
            trace_draws: false,
            capture_draw_argument_values: false,
            profiling: false,
            collect_accounting: true,
            profile_function_dispatches: 0,
            draw_argument_values: BTreeMap::new(),
            draw_argument_value_texts: BTreeMap::new(),
            draw_memory_reads: Vec::new(),
            draw_memory_operations: Vec::new(),
            last_memory_writes: BTreeMap::new(),
        }
    }

    pub fn set_draw_tracing(&mut self, enabled: bool) {
        self.trace_draws = enabled;
        self.capture_draw_argument_values = enabled;
    }

    pub(crate) fn set_profiling(&mut self, enabled: bool) {
        self.profiling = enabled;
    }

    pub(crate) fn set_accounting(&mut self, enabled: bool) {
        self.collect_accounting = enabled;
    }

    /// Capture a bounded VM trace from the start of the next execution.
    /// Intended for small, targeted diagnostic calls and tests.
    pub fn enable_diagnostics(&mut self, capacity: usize) -> Result<()> {
        self.enable_diagnostics_after(capacity, 0)
    }

    /// Capture only the tail of a potentially expensive execution. The
    /// evaluation cap and VM operation semantics remain unchanged.
    pub fn enable_limit_diagnostics(&mut self, capacity: usize) -> Result<()> {
        self.enable_diagnostics_after(capacity, self.max_evaluations.saturating_sub(16_384))
    }

    pub(crate) fn set_evaluation_limit(&mut self, limit: usize) {
        self.max_evaluations = limit.max(1);
    }

    fn enable_diagnostics_after(
        &mut self,
        capacity: usize,
        capture_after_evaluation: usize,
    ) -> Result<()> {
        if !(1..=4096).contains(&capacity) {
            bail!("diagnostic event capacity must be in 1..=4096");
        }
        self.diagnostics = Some(VmDiagnostics {
            capacity,
            capture_after_evaluation,
            events: VecDeque::with_capacity(capacity),
        });
        Ok(())
    }

    /// Return bounded diagnostics for an enabled VM, including runtime time
    /// values and the most recent node, loop, and memory events.
    pub fn diagnostic_report(&self) -> Option<String> {
        let diagnostics = self.diagnostics.as_ref()?;
        let current_time = self.memory_get(RUNTIME_UPDATE_BLOCK, 0);
        let delta_time = self.memory_get(RUNTIME_UPDATE_BLOCK, 1);
        let mut report = format!(
            "Watch VM diagnostic: evaluations={}, time={}, deltaTime={}, timescale={}",
            self.evaluations, current_time, delta_time, self.context.timescale
        );
        if !self.loop_trace.is_empty() {
            let active = self
                .loop_trace
                .iter()
                .filter_map(|frame| {
                    let body = self
                        .nodes
                        .get(frame.node)
                        .and_then(|node| node.args.get(frame.branch))
                        .and_then(serde_json::Value::as_u64);
                    body.map(|body| {
                        format!(
                            "node={} iteration={} branch={} body_node={body}",
                            frame.node, frame.iteration, frame.branch
                        )
                    })
                })
                .collect::<Vec<_>>();
            if !active.is_empty() {
                report.push_str(&format!("\nActive loop fingerprint: {}", active.join("; ")));
            }
        }
        if diagnostics.events.is_empty() {
            report.push_str("\n(no events captured before execution stopped)");
        } else {
            let mut loop_branches = BTreeMap::<usize, BTreeSet<(usize, usize)>>::new();
            let mut memory_slots = BTreeMap::<i64, BTreeSet<usize>>::new();
            for event in &diagnostics.events {
                if let Some(rest) = event.split("JumpLoop node=").nth(1) {
                    let mut fields = rest.split_whitespace();
                    let node = fields.next().and_then(|v| v.parse::<usize>().ok());
                    let branch = rest
                        .split("selected_branch=")
                        .nth(1)
                        .and_then(|v| v.split_whitespace().next())
                        .and_then(|v| v.parse::<usize>().ok());
                    let body = rest
                        .split("branch_node=")
                        .nth(1)
                        .and_then(|v| v.split_whitespace().next())
                        .and_then(|v| v.parse::<usize>().ok());
                    if let (Some(node), Some(branch), Some(body)) = (node, branch, body) {
                        loop_branches
                            .entry(node)
                            .or_default()
                            .insert((branch, body));
                    }
                }
                if let (Some(block), Some(slot)) = (
                    event
                        .split(" block=")
                        .nth(1)
                        .and_then(|v| v.split_whitespace().next())
                        .and_then(|v| v.parse::<i64>().ok()),
                    event
                        .split(" index=")
                        .nth(1)
                        .and_then(|v| v.split_whitespace().next())
                        .and_then(|v| v.parse::<usize>().ok()),
                ) {
                    memory_slots.entry(block).or_default().insert(slot);
                }
            }
            if !loop_branches.is_empty() {
                let summaries = loop_branches
                    .iter()
                    .map(|(node, branches)| {
                        let branches = branches
                            .iter()
                            .map(|(branch, body)| format!("{branch}->{body}"))
                            .collect::<Vec<_>>()
                            .join(",");
                        format!("JumpLoop {node} branches [{branches}]")
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                report.push_str(&format!("\nLoop fingerprint in trace window: {summaries}"));
            }
            if !memory_slots.is_empty() {
                let accesses = memory_slots
                    .iter()
                    .map(|(block, slots)| {
                        format!(
                            "{block}=[{}]",
                            slots
                                .iter()
                                .map(usize::to_string)
                                .collect::<Vec<_>>()
                                .join(",")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                report.push_str(&format!(
                    "\nMemory block/index pairs in trace window: {accesses}"
                ));
            }
            report.push_str(&format!(
                "\nlast {} VM events (bounded capacity {}) :",
                diagnostics.events.len(),
                diagnostics.capacity
            ));
            for event in &diagnostics.events {
                report.push_str("\n  ");
                report.push_str(event);
            }
        }
        Some(report)
    }

    fn record_diagnostic(&mut self, message: impl FnOnce() -> String) {
        let Some(diagnostics) = self.diagnostics.as_mut() else {
            return;
        };
        if self.evaluations < diagnostics.capture_after_evaluation {
            return;
        }
        if diagnostics.events.len() == diagnostics.capacity {
            diagnostics.events.pop_front();
        }
        let loops = self
            .loop_trace
            .iter()
            .map(|frame| format!("{}#{}:branch{}", frame.node, frame.iteration, frame.branch))
            .collect::<Vec<_>>()
            .join("/");
        let loop_context = if loops.is_empty() {
            String::new()
        } else {
            format!(" loop={loops}")
        };
        diagnostics.events.push_back(format!(
            "eval={}{} {}",
            self.evaluations,
            loop_context,
            message()
        ));
    }

    pub fn execute(&mut self, entry: usize) -> Result<f64> {
        self.evaluations = 0;
        self.depth = 0;
        self.break_signal = None;
        self.eval(entry)
    }

    pub fn evaluation_count(&self) -> usize {
        self.evaluations
    }

    fn eval(&mut self, index: usize) -> Result<f64> {
        self.evaluations += 1;
        if self.evaluations > self.max_evaluations {
            bail!("Watch execution exceeded the evaluation limit");
        }
        if self.depth >= MAX_CALL_DEPTH {
            bail!("Watch execution exceeded the recursion limit at node {index}");
        }
        let node = self
            .nodes
            .get(index)
            .with_context(|| format!("Watch node index {index} is out of bounds"))?;
        if let Some(value) = node.value.as_ref().and_then(serde_json::Value::as_f64) {
            if self.capture_draw_argument_values {
                self.draw_argument_values.insert(index, value);
                self.draw_argument_value_texts
                    .insert(index, trace_value(value));
            }
            if self.context.execution_trace.is_some() {
                self.record_execution_trace(
                    index,
                    "Constant",
                    "evaluation",
                    serde_json::json!({"children":[],"arguments":[],"result":trace_value(value)}),
                );
            }
            return Ok(value);
        }
        let function = node.func.as_deref().with_context(|| {
            format!("Watch node {index} has neither a numeric value nor a function")
        })?;
        self.record_diagnostic(|| format!("node={index} function={function}"));
        if self.context.execution_trace.is_some() {
            self.execution_trace_frames
                .push((index, vec![None; node.args.len()]));
        }
        self.depth += 1;
        let particle_event_count = self.particle_events.len();
        let result = self.eval_function(index, function);
        self.depth -= 1;
        if self.context.execution_trace.is_some() {
            let (_, arguments) = self.execution_trace_frames.pop().unwrap();
            let kind = if matches!(
                function,
                "Play"
                    | "PlayScheduled"
                    | "PlayLooped"
                    | "PlayLoopedScheduled"
                    | "StopLooped"
                    | "StopLoopedScheduled"
            ) {
                "audio"
            } else if matches!(
                function,
                "SpawnParticleEffect" | "MoveParticleEffect" | "DestroyParticleEffect"
            ) {
                "particle_host"
            } else {
                "evaluation"
            };
            let mut data = serde_json::json!({"children": self.nodes[index].args, "arguments": arguments,
                "result": result.as_ref().ok().map(|v| trace_value(*v)), "error":result.as_ref().err().map(|e| e.to_string())});
            if kind == "audio" && result.is_ok() {
                data["event"] = match function {
                    "PlayScheduled" => serde_json::to_value(self.scheduled_effects.last())?,
                    "PlayLoopedScheduled" => {
                        serde_json::to_value(self.scheduled_looped_effects.last())?
                    }
                    "StopLoopedScheduled" => {
                        serde_json::to_value(self.scheduled_looped_effect_stops.last())?
                    }
                    _ => serde_json::to_value(self.audio_events.last())?,
                };
            }
            if kind == "particle_host" && result.is_ok() {
                data["event"] = if self.particle_events.len() > particle_event_count {
                    serde_json::to_value(self.particle_events.last())?
                } else {
                    serde_json::Value::Null
                };
            }
            self.record_execution_trace(index, function, kind, data);
        }
        let result =
            result.with_context(|| format!("executing Watch function {function} at node {index}"));
        if self.capture_draw_argument_values {
            if let Ok(value) = result.as_ref() {
                self.draw_argument_values.insert(index, *value);
                self.draw_argument_value_texts
                    .insert(index, trace_value(*value));
            }
        }
        result
    }

    fn arg(&mut self, node_index: usize, arg_index: usize) -> Result<f64> {
        let index = self
            .nodes
            .get(node_index)
            .and_then(|n| n.args.get(arg_index))
            .with_context(|| format!("missing argument {arg_index} at node {node_index}"))?;
        let index = index.as_u64().with_context(|| {
            format!("argument {arg_index} of node {node_index} is not a node index")
        })? as usize;
        let value = self.eval(index)?;
        if let Some((_, arguments)) = self.execution_trace_frames.last_mut() {
            if let Some(argument) = arguments.get_mut(arg_index) {
                *argument = Some(trace_value(value));
            }
        }
        Ok(value)
    }

    fn record_execution_trace(
        &self,
        node: usize,
        operation: &str,
        kind: &str,
        data: serde_json::Value,
    ) {
        if let Some(trace) = &self.context.execution_trace {
            let loops = self
                .loop_trace
                .iter()
                .map(|f| (f.node, f.iteration, f.branch))
                .collect::<Vec<_>>();
            if let Ok(mut trace) = trace.lock() {
                trace.record(&self.context, node, operation, kind, &loops, data);
            }
        }
    }

    fn memory_get(&self, block: i64, slot: usize) -> f64 {
        let value = self.memory_get_untraced(block, slot);
        if let Some((node, _)) = self.execution_trace_frames.last() {
            self.record_execution_trace(
                *node,
                self.nodes[*node].func.as_deref().unwrap_or("?"),
                "memory_read",
                serde_json::json!({"block":block,"index":slot,"value":trace_value(value)}),
            );
        }
        value
    }

    fn memory_get_untraced(&self, block: i64, slot: usize) -> f64 {
        if block == 1004 {
            self.context
                .runtime_background
                .read()
                .ok()
                .and_then(|values| values.get(slot).copied())
                .unwrap_or(0.0)
        } else if block == ENGINE_ROM_BLOCK {
            self.context.engine_rom.get(slot).copied().unwrap_or(0.0)
        } else if block == ENTITY_DATA_ARRAY_BLOCK {
            self.context
                .entity_data_array
                .read()
                .ok()
                .and_then(|values| values.get(slot).copied())
                .unwrap_or(0.0)
        } else if block == 4001 {
            if slot >= 32 {
                return 0.0;
            }
            self.context
                .entity_id
                .filter(|_| self.context.has_entity_data)
                .and_then(|id| {
                    self.context
                        .entity_data_array
                        .read()
                        .ok()
                        .and_then(|values| values.get(id.saturating_mul(32) + slot).copied())
                })
                .unwrap_or(0.0)
        } else if block == 4002 {
            if slot >= 32 {
                return 0.0;
            }
            self.context
                .entity_id
                .filter(|_| self.context.has_entity_shared_memory)
                .and_then(|id| {
                    self.context
                        .entity_shared_memory_array
                        .read()
                        .ok()
                        .and_then(|values| values.get(id.saturating_mul(32) + slot).copied())
                })
                .unwrap_or(0.0)
        } else if block == ENTITY_SHARED_MEMORY_ARRAY_BLOCK {
            self.context
                .entity_shared_memory_array
                .read()
                .ok()
                .and_then(|values| values.get(slot).copied())
                .unwrap_or(0.0)
        } else if block == ENTITY_INFO_ARRAY_BLOCK {
            self.context
                .entity_info_array
                .read()
                .ok()
                .and_then(|values| values.get(slot).copied())
                .unwrap_or(0.0)
        } else if block == 4003 {
            self.context
                .entity_info
                .and_then(|values| values.get(slot).copied())
                .unwrap_or(0.0)
        } else {
            self.memory.get(block, slot)
        }
    }

    /// Resolve the address used by the documented pointed memory operations.
    /// `GetPointed(id, index, offset)` is specified as
    /// `Get(Get(id, index), Get(id, index + 1) + offset)`; pointed writes use
    /// the same two-cell pointer representation before applying Set.
    fn pointed_location_values(
        &self,
        block_value: f64,
        index_value: f64,
        offset: f64,
        writing: bool,
    ) -> Result<Option<(i64, usize)>> {
        // The two inner Gets follow Get's invalid-address behavior: an
        // invalid block/index yields zero. The final operation then follows
        // the corresponding Get or Set conversion behavior.
        let base_block = integer(block_value, "block id").ok();
        let first_slot = index_of(index_value).ok();
        let pointed_block_value = match (base_block, first_slot) {
            (Some(block), Some(slot)) => self.memory_get(block, slot),
            _ => 0.0,
        };

        let second_slot = index_of(index_value + 1.0).ok();
        let pointer_value = match (base_block, second_slot) {
            (Some(block), Some(slot)) => self.memory_get(block, slot),
            _ => 0.0,
        };

        let target_index_value = pointer_value + offset;
        if writing {
            let target_block = integer(pointed_block_value, "block id")?;
            let target_index = index_of(target_index_value)?;
            Ok(Some((target_block, target_index)))
        } else {
            let target_block = integer(pointed_block_value, "block id").ok();
            let target_index = index_of(target_index_value).ok();
            Ok(target_block.zip(target_index))
        }
    }

    fn allocate_looped_effect_id(&self) -> Result<i64> {
        self.context
            .next_looped_effect_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                // VM numeric values are f64, so every issued handle remains
                // exactly representable as well as within i64 range.
                (next <= 9_007_199_254_740_991)
                    .then(|| next.checked_add(1))
                    .flatten()
            })
            .map_err(|_| anyhow::anyhow!("looped effect instance ID space exhausted"))
    }

    fn memory_set(&mut self, block: i64, slot: usize, value: f64, node: usize) -> Result<()> {
        if self.context.execution_trace.is_none() {
            return self.memory_set_untraced(block, slot, value, node);
        }
        let old = self.memory_get_untraced(block, slot);
        let result = self.memory_set_untraced(block, slot, value, node);
        self.record_execution_trace(node, self.nodes[node].func.as_deref().unwrap_or("?"), "memory_write",
            serde_json::json!({"block":block,"index":slot,"old":trace_value(old),"value":trace_value(value),"committed":result.is_ok(),"error":result.as_ref().err().map(|e|e.to_string())}));
        result
    }

    fn memory_set_untraced(
        &mut self,
        block: i64,
        slot: usize,
        value: f64,
        node: usize,
    ) -> Result<()> {
        if block == RUNTIME_SKIN_TRANSFORM_BLOCK {
            if !matches!(self.context.lifecycle_stage, Some(0 | 5)) {
                bail!("Runtime Skin Transform is read-only in this lifecycle stage");
            }
            if slot >= 16 {
                bail!("Runtime Skin Transform index {slot} exceeds 15");
            }
            self.memory.set(block, slot, value);
            if self.capture_draw_argument_values {
                self.last_memory_writes.insert((block, slot), (node, value));
            }
            return Ok(());
        }
        if block == RUNTIME_PARTICLE_TRANSFORM_BLOCK {
            if !matches!(self.context.lifecycle_stage, Some(0 | 5)) {
                bail!("Runtime Particle Transform is read-only in this lifecycle stage");
            }
            if slot >= 16 {
                bail!("Runtime Particle Transform index {slot} exceeds 15");
            }
            self.memory.set(block, slot, value);
            if self.capture_draw_argument_values {
                self.last_memory_writes.insert((block, slot), (node, value));
            }
            return Ok(());
        }
        if block == 1007 {
            if self.context.lifecycle_stage != Some(0) {
                bail!("Runtime UI Configuration is read-only outside preprocessing");
            }
            if slot >= 10 {
                bail!("Runtime UI Configuration index {slot} exceeds 9");
            }
            self.memory.set(block, slot, value);
            if self.capture_draw_argument_values {
                self.last_memory_writes.insert((block, slot), (node, value));
            }
            return Ok(());
        }
        if block == 1004 {
            if !matches!(self.context.lifecycle_stage, Some(0 | 5)) {
                bail!("Runtime Background is read-only in this lifecycle stage");
            }
            let mut values = self
                .context
                .runtime_background
                .write()
                .map_err(|_| anyhow::anyhow!("Runtime Background lock was poisoned"))?;
            let target = values
                .get_mut(slot)
                .with_context(|| format!("Runtime Background index {slot} exceeds 7"))?;
            *target = value;
            if self.capture_draw_argument_values {
                self.last_memory_writes.insert((block, slot), (node, value));
            }
            return Ok(());
        }
        if block == ENGINE_ROM_BLOCK {
            bail!("Engine Rom block is read-only");
        }
        if block == ENTITY_DATA_ARRAY_BLOCK {
            if !self.context.entity_data_array_writable {
                bail!("Entity Data Array block is read-only outside preprocessing");
            }
            let mut values = self
                .context
                .entity_data_array
                .write()
                .map_err(|_| anyhow::anyhow!("Entity Data Array lock was poisoned"))?;
            if slot >= values.len() {
                bail!(
                    "Entity Data Array index {slot} is outside {} values",
                    values.len()
                );
            }
            values[slot] = value;
            if self.capture_draw_argument_values {
                self.last_memory_writes.insert((block, slot), (node, value));
            }
            return Ok(());
        }
        if block == ENTITY_SHARED_MEMORY_ARRAY_BLOCK {
            if !matches!(self.context.lifecycle_stage, Some(0 | 5)) {
                bail!("Entity Shared Memory Array is read-only in this lifecycle stage");
            }
            let mut values = self
                .context
                .entity_shared_memory_array
                .write()
                .map_err(|_| anyhow::anyhow!("Entity Shared Memory Array lock was poisoned"))?;
            if slot >= values.len() {
                bail!(
                    "Entity Shared Memory Array index {slot} is outside {} values",
                    values.len()
                );
            }
            values[slot] = value;
            if self.capture_draw_argument_values {
                self.last_memory_writes.insert((block, slot), (node, value));
            }
            return Ok(());
        }
        if block == 4001 {
            if self.context.lifecycle_stage != Some(0) {
                bail!("Entity Data is read-only outside preprocessing");
            }
            if !self.context.has_entity_data {
                bail!("Entity Data is unavailable for this entity");
            }
            if slot >= 32 {
                bail!("Entity Data slot {slot} exceeds its 32-value row");
            }
            let id = self
                .context
                .entity_id
                .context("missing current entity ID")?;
            let mut values = self
                .context
                .entity_data_array
                .write()
                .map_err(|_| anyhow::anyhow!("Entity Data Array lock was poisoned"))?;
            let index = id.saturating_mul(32).saturating_add(slot);
            if index >= values.len() {
                bail!("Entity Data row for entity {id} is outside the array");
            }
            values[index] = value;
            if self.capture_draw_argument_values {
                self.last_memory_writes.insert((block, slot), (node, value));
            }
            return Ok(());
        }
        if block == 4002 {
            if !matches!(self.context.lifecycle_stage, Some(0 | 5)) {
                bail!("Entity Shared Memory is read-only in this lifecycle stage");
            }
            if !self.context.has_entity_shared_memory {
                bail!("Entity Shared Memory is unavailable for this entity");
            }
            if slot >= 32 {
                bail!("Entity Shared Memory slot {slot} exceeds its 32-value row");
            }
            let id = self
                .context
                .entity_id
                .context("missing current entity ID")?;
            let mut values = self
                .context
                .entity_shared_memory_array
                .write()
                .map_err(|_| anyhow::anyhow!("Entity Shared Memory Array lock was poisoned"))?;
            let index = id.saturating_mul(32).saturating_add(slot);
            if index >= values.len() {
                bail!("Entity Shared Memory row for entity {id} is outside the array");
            }
            values[index] = value;
            if self.capture_draw_argument_values {
                self.last_memory_writes.insert((block, slot), (node, value));
            }
            return Ok(());
        }
        if block == ENTITY_INFO_ARRAY_BLOCK || block == 4003 {
            bail!("Entity Info memory blocks are read-only");
        }
        if block == 4001 && self.context.lifecycle_stage != Some(0) {
            bail!("Entity Data is read-only outside preprocessing");
        }
        if block == 4002 && !matches!(self.context.lifecycle_stage, Some(0 | 5)) {
            bail!("Entity Shared Memory is read-only in this lifecycle stage");
        }
        self.memory.set(block, slot, value);
        if self.capture_draw_argument_values && block != 10000 {
            let operation = self
                .draw_memory_operations
                .iter()
                .rev()
                .find(|operation| operation.node == node)
                .cloned();
            let (expression_values, expression_reads) = if block == 2000 {
                self.trace_memory_write_expression(node)
            } else {
                (BTreeMap::new(), Vec::new())
            };
            if let Ok(mut origins) = self.context.memory_write_origins.write() {
                origins.insert(
                    memory_origin_key(self.context.entity_id, block, slot),
                    MemoryWriteOrigin {
                        node,
                        entity_id: self.context.entity_id,
                        archetype: self.context.entity_archetype.clone(),
                        callback: self.context.callback_name.clone(),
                        callback_node: self.context.callback_node,
                        value: trace_value(value),
                        operation,
                        expression_values,
                        expression_reads,
                    },
                );
            }
        }
        if self.capture_draw_argument_values {
            self.last_memory_writes.insert((block, slot), (node, value));
        }
        Ok(())
    }

    fn trace_memory_write_expression(
        &self,
        writer_node: usize,
    ) -> (BTreeMap<usize, String>, Vec<MemoryReadTrace>) {
        let Some(root) = self
            .nodes
            .get(writer_node)
            .and_then(|node| node.args.last())
            .and_then(serde_json::Value::as_u64)
            .map(|node| node as usize)
        else {
            return (BTreeMap::new(), Vec::new());
        };
        let mut pending = vec![root];
        let mut seen = BTreeSet::new();
        let mut values = BTreeMap::new();
        let mut reads = BTreeMap::<usize, MemoryReadTrace>::new();
        while let Some(index) = pending.pop() {
            if !seen.insert(index) {
                continue;
            }
            if let Some(value) = self.draw_argument_value_texts.get(&index) {
                values.insert(index, value.clone());
            }
            if let Some(read) = self
                .draw_memory_reads
                .iter()
                .rev()
                .find(|read| read.node == index)
            {
                reads.insert(index, read.clone());
            }
            if let Some(node) = self.nodes.get(index) {
                pending.extend(
                    node.args
                        .iter()
                        .filter_map(serde_json::Value::as_u64)
                        .map(|node| node as usize),
                );
            }
        }
        (values, reads.into_values().collect())
    }

    fn all_args(&mut self, node_index: usize) -> Result<Vec<f64>> {
        let count = self
            .nodes
            .get(node_index)
            .with_context(|| format!("Watch node index {node_index} is out of bounds"))?
            .args
            .len();
        (0..count)
            .map(|index| self.arg(node_index, index))
            .collect()
    }

    fn eval_function(&mut self, index: usize, name: &str) -> Result<f64> {
        if self.profiling {
            self.profile_function_dispatches += 1;
        }
        if self.collect_accounting {
            *self.function_counts.entry(name.to_owned()).or_default() += 1;
        }
        let count = self.nodes[index].args.len();
        let unary = |this: &mut Self| this.arg(index, 0);
        let binary = |this: &mut Self| -> Result<(f64, f64)> {
            Ok((this.arg(index, 0)?, this.arg(index, 1)?))
        };
        match name {
            "DebugLog" => {
                if count != 1 {
                    bail!("DebugLog requires exactly one value");
                }
                let value = unary(self)?;
                self.debug_events.push(DebugEvent::Log {
                    value,
                    value_text: DebugEvent::display_value(value),
                    entity_id: self.context.entity_id,
                    archetype: self.context.entity_archetype.clone(),
                    callback: self.context.callback_name.clone(),
                    callback_node: self.context.callback_node,
                    node: index,
                });
                Ok(0.0)
            }
            "DebugPause" => {
                if count != 0 {
                    bail!("DebugPause does not take arguments");
                }
                self.debug_events.push(DebugEvent::Pause {
                    entity_id: self.context.entity_id,
                    archetype: self.context.entity_archetype.clone(),
                    callback: self.context.callback_name.clone(),
                    callback_node: self.context.callback_node,
                    node: index,
                });
                Ok(0.0)
            }
            "Execute" => {
                let mut result = 0.0;
                for arg in 0..count {
                    result = self.arg(index, arg)?;
                    if self.break_signal.is_some() {
                        break;
                    }
                }
                Ok(result)
            }
            "If" => {
                let test = unary(self)?;
                let selected = if test == 0.0 { 2 } else { 1 };
                self.record_diagnostic(|| {
                    format!("If node={index} condition={test} selected_argument={selected}")
                });
                self.arg(index, selected)
            }
            "HasSkinSprite" | "HasEffectClip" | "HasParticleEffect" => {
                let resource = integer(self.arg(index, 0)?, "resource id")?;
                let exists = match name {
                    "HasSkinSprite" => self
                        .context
                        .skin_sprites
                        .iter()
                        .any(|id| *id as i64 == resource),
                    "HasEffectClip" => self
                        .context
                        .effect_clips
                        .iter()
                        .any(|id| *id as i64 == resource),
                    _ => self
                        .context
                        .particle_effects
                        .iter()
                        .any(|id| *id as i64 == resource),
                };
                if name == "HasSkinSprite" {
                    if self.collect_accounting {
                        self.skin_checks.push((resource, exists));
                    }
                }
                Ok(truth(exists))
            }
            "TimeToTimeScale" => {
                let time = self.arg(index, 0)?;
                self.context.time_to_timescale(time)
            }
            "TimeToScaledTime" => {
                let time = self.arg(index, 0)?;
                self.context.scaled_time(time)
            }
            "TimeToStartingTime" => {
                let time = self.arg(index, 0)?;
                self.context.time_to_starting_time(time)
            }
            "TimeToStartingScaledTime" => {
                let time = self.arg(index, 0)?;
                self.context.time_to_starting_scaled_time(time)
            }
            "BeatToTime" => {
                let beat = self.arg(index, 0)?;
                self.context.beat_to_time(beat)
            }
            "BeatToStartingTime" => {
                let beat = self.arg(index, 0)?;
                self.context.beat_to_starting_time(beat)
            }
            "BeatToStartingBeat" => {
                let beat = self.arg(index, 0)?;
                self.context.beat_to_starting_beat(beat)
            }
            "BeatToBPM" => {
                let beat = self.arg(index, 0)?;
                self.context.bpm_at_beat(beat)
            }
            "StreamHas" | "StreamGetValue" | "StreamGetPreviousKey" | "StreamGetNextKey" => {
                let stream = integer(self.arg(index, 0)?, "stream id")?;
                let time = self.arg(index, 1)?;
                let key = self.context.streams.get(&(stream, 0));
                match name {
                    "StreamHas" => {
                        Ok(truth(key.is_some_and(|events| {
                            events.iter().any(|(t, _)| *t == time)
                        })))
                    }
                    "StreamGetValue" => {
                        Ok(stream_value(key.map(Vec::as_slice).unwrap_or(&[]), time))
                    }
                    "StreamGetPreviousKey" => Ok(key
                        .and_then(|events| events.iter().rev().find(|(t, _)| *t < time))
                        .map(|(t, _)| *t)
                        .unwrap_or(time)),
                    _ => Ok(key
                        .and_then(|events| events.iter().find(|(t, _)| *t > time))
                        .map(|(t, _)| *t)
                        .unwrap_or(time)),
                }
            }
            "SwitchWithDefault" => {
                let discriminant = self.arg(index, 0)?;
                if count < 2 {
                    bail!("SwitchWithDefault requires a discriminant and default branch");
                }
                if count % 2 != 0 {
                    bail!("SwitchWithDefault requires test/consequent pairs and a default");
                }
                let pair_count = (count - 2) / 2;
                for pair in 0..pair_count {
                    let test = self.arg(index, 1 + pair * 2)?;
                    if discriminant == test {
                        return self.arg(index, 2 + pair * 2);
                    }
                }
                self.arg(index, count - 1)
            }
            "SwitchIntegerWithDefault" => {
                let discriminant = self.arg(index, 0)?;
                if count < 2 {
                    bail!("SwitchIntegerWithDefault requires a discriminant and default branch");
                }
                // The specification defines matching by equality to branch n;
                // it does not require converting the discriminant to an integer.
                let selected = (0..count - 2)
                    .find(|branch| discriminant == *branch as f64)
                    .map(|branch| branch + 1)
                    .unwrap_or(count - 1);
                self.arg(index, selected)
            }
            "SwitchInteger" => {
                let discriminant = self.arg(index, 0)?;
                // A fractional value simply matches no integer-indexed branch.
                let selected =
                    (0..count.saturating_sub(1)).find(|branch| discriminant == *branch as f64);
                match selected {
                    Some(branch) => self.arg(index, branch + 1),
                    None => Ok(0.0),
                }
            }
            "Switch" => {
                if count == 0 || count % 2 == 0 {
                    bail!("Switch requires a discriminant and test/consequent pairs");
                }
                let discriminant = self.arg(index, 0)?;
                for pair in 0..(count - 1) / 2 {
                    if discriminant == self.arg(index, 1 + pair * 2)? {
                        return self.arg(index, 2 + pair * 2);
                    }
                }
                Ok(0.0)
            }
            "JumpLoop" => {
                if count == 0 {
                    return Ok(0.0);
                }
                let mut selected = 0;
                self.loop_trace.push(LoopTraceFrame {
                    node: index,
                    iteration: 0,
                    branch: selected,
                });
                loop {
                    if let Some(frame) = self.loop_trace.last_mut() {
                        frame.iteration += 1;
                        frame.branch = selected;
                    }
                    let iteration = self
                        .loop_trace
                        .last()
                        .map(|frame| frame.iteration)
                        .unwrap_or_default();
                    let branch_node = self.nodes[index]
                        .args
                        .get(selected)
                        .and_then(serde_json::Value::as_u64)
                        .map(|value| value as usize)
                        .unwrap_or(usize::MAX);
                    self.record_diagnostic(|| {
                        format!(
                            "JumpLoop node={index} iteration={iteration} selected_branch={selected} branch_node={branch_node}"
                        )
                    });
                    let result = self.arg(index, selected)?;
                    if self.context.execution_trace.is_some() {
                        self.record_execution_trace(index, "JumpLoop", "route", serde_json::json!({"slot":selected,"child":branch_node,"next":trace_value(result),"last":selected == count-1,"break":self.break_signal}));
                    }
                    self.record_diagnostic(|| {
                        format!(
                            "JumpLoop node={index} iteration={iteration} branch_result={result}"
                        )
                    });
                    if self.break_signal.is_some() {
                        self.loop_trace.pop();
                        return Ok(result);
                    }
                    if selected == count - 1 {
                        self.loop_trace.pop();
                        return Ok(result);
                    }
                    let Some(next) = usize::try_from(integer(result, "JumpLoop branch")?).ok()
                    else {
                        self.loop_trace.pop();
                        return Ok(0.0);
                    };
                    if next >= count {
                        self.loop_trace.pop();
                        return Ok(0.0);
                    }
                    selected = next;
                }
            }
            "While" => {
                if count != 1 && count != 2 {
                    bail!("While requires a body, or a legacy condition and body; got {count} arguments");
                }
                let (condition, body) = if count == 1 { (None, 0) } else { (Some(0), 1) };
                self.block_depth += 1;
                let loop_depth = self.block_depth;
                let result = (|| -> Result<f64> {
                    let mut last_result = 0.0;
                    loop {
                        if let Some(condition) = condition {
                            if self.arg(index, condition)? == 0.0 {
                                return Ok(last_result);
                            }
                        }
                        last_result = self.arg(index, body)?;
                        if let Some((target, value)) = self.break_signal {
                            if target == loop_depth {
                                self.break_signal = None;
                                return Ok(value);
                            }
                            if target < loop_depth {
                                return Ok(value);
                            }
                        }
                    }
                })();
                self.block_depth -= 1;
                result
            }
            "DoWhile" => {
                if count != 2 {
                    bail!("DoWhile requires body and test arguments, got {count}");
                }
                self.block_depth += 1;
                let loop_depth = self.block_depth;
                let result = (|| -> Result<f64> {
                    loop {
                        self.arg(index, 0)?;
                        if let Some((target, value)) = self.break_signal {
                            if target == loop_depth {
                                self.break_signal = None;
                                return Ok(value);
                            }
                            if target < loop_depth {
                                return Ok(value);
                            }
                        }
                        if self.arg(index, 1)? == 0.0 {
                            return Ok(0.0);
                        }
                    }
                })();
                self.block_depth -= 1;
                result
            }
            "Block" => {
                self.block_depth += 1;
                let result = unary(self)?;
                let depth = self.block_depth;
                self.block_depth -= 1;
                if let Some((target, value)) = self.break_signal {
                    if target == depth {
                        self.break_signal = None;
                        return Ok(value);
                    }
                }
                Ok(result)
            }
            "Break" => {
                if count != 2 {
                    bail!("Break requires a block count and return value");
                }
                let count = index_of(self.arg(index, 0)?)?;
                let value = self.arg(index, 1)?;
                if self.block_depth == 0 {
                    bail!("Break executed outside a Block");
                }
                if count == 0 || count > self.block_depth {
                    bail!(
                        "Break count {count} is outside the active block depth {}",
                        self.block_depth
                    );
                }
                self.break_signal = Some((self.block_depth - count + 1, value));
                Ok(value)
            }
            "Get" => {
                let block = self.arg(index, 0)?;
                let slot = self.arg(index, 1)?;
                let Ok(block) = integer(block, "block id") else {
                    return Ok(0.0);
                };
                let Ok(slot) = index_of(slot) else {
                    return Ok(0.0);
                };
                let value = self.memory_get(block, slot);

                if self.capture_draw_argument_values {
                    self.draw_memory_reads.push(MemoryReadTrace {
                        node: index,
                        block,
                        index: slot,
                        value,
                        last_write: self.last_memory_writes.get(&(block, slot)).copied(),
                        value_text: trace_value(value),
                        last_write_value_text: self
                            .last_memory_writes
                            .get(&(block, slot))
                            .map(|(_, value)| trace_value(*value)),
                        last_write_origin: self.context.memory_write_origins.read().ok().and_then(
                            |origins| {
                                origins
                                    .get(&memory_origin_key(self.context.entity_id, block, slot))
                                    .cloned()
                            },
                        ),
                    });
                }
                self.record_diagnostic(|| {
                    format!("Get node={index} block={block} index={slot} value={value}")
                });
                Ok(value)
            }
            "GetPointed" => {
                let args = self.all_args(index)?;
                let value = self
                    .pointed_location_values(args[0], args[1], args[2], false)?
                    .map(|(block, slot)| self.memory_get(block, slot))
                    .unwrap_or(0.0);
                self.record_diagnostic(|| format!("GetPointed node={index} value={value}"));
                Ok(value)
            }
            "GetShifted" => {
                let block = self.arg(index, 0)?;
                let x = self.arg(index, 1)?;
                let y = self.arg(index, 2)?;
                let stride = self.arg(index, 3)?;
                let Ok(block) = integer(block, "block id") else {
                    return Ok(0.0);
                };
                let Ok(slot) = index_of(x + y * stride) else {
                    return Ok(0.0);
                };
                let value = self.memory_get(block, slot);
                self.record_diagnostic(|| {
                    format!("GetShifted node={index} block={block} x={x} y={y} stride={stride} index={slot} value={value}")
                });
                Ok(value)
            }
            "Set" | "SetAdd" | "SetSubtract" | "SetMultiply" | "SetDivide" | "SetMod"
            | "SetRem" | "SetPower" => {
                let block = integer(self.arg(index, 0)?, "block id")?;
                let slot = index_of(self.arg(index, 1)?)?;
                let value = self.arg(index, 2)?;
                let old = self.memory_get(block, slot);
                let result = match name {
                    "Set" => value,
                    "SetAdd" => old + value,
                    "SetSubtract" => old - value,
                    "SetMultiply" => old * value,
                    "SetDivide" => old / value,
                    "SetMod" => modulus(old, value)?,
                    "SetRem" => old % value,
                    "SetPower" => old.powf(value),
                    _ => unreachable!(),
                };
                if self.capture_draw_argument_values {
                    self.draw_memory_operations.push(MemoryOperationTrace {
                        node: index,
                        function: name.to_owned(),
                        block,
                        index: slot,
                        old_value: trace_value(old),
                        operand: trace_value(value),
                        result: trace_value(result),
                    });
                }
                self.memory_set(block, slot, result, index)?;

                self.record_diagnostic(|| {
                    format!("{name} node={index} block={block} index={slot} old={old} new={result}")
                });
                Ok(result)
            }
            "SetShifted" | "SetAddShifted" | "SetSubtractShifted" | "SetMultiplyShifted"
            | "SetDivideShifted" | "SetModShifted" | "SetRemShifted" | "SetPowerShifted" => {
                let block = integer(self.arg(index, 0)?, "block id")?;
                let x = self.arg(index, 1)?;
                let y = self.arg(index, 2)?;
                let stride = self.arg(index, 3)?;
                let value = self.arg(index, 4)?;
                let slot = index_of(x + y * stride)?;
                let old = self.memory_get(block, slot);
                let result = match name {
                    "SetShifted" => value,
                    "SetAddShifted" => old + value,
                    "SetSubtractShifted" => old - value,
                    "SetMultiplyShifted" => old * value,
                    "SetDivideShifted" => old / value,
                    "SetModShifted" => modulus(old, value)?,
                    "SetRemShifted" => old % value,
                    "SetPowerShifted" => old.powf(value),
                    _ => unreachable!(),
                };
                self.memory_set(block, slot, result, index)?;
                self.record_diagnostic(|| {
                    format!("{name} node={index} block={block} x={x} y={y} stride={stride} index={slot} old={old} new={result}")
                });
                Ok(result)
            }
            "SetPointed" | "SetAddPointed" | "SetSubtractPointed" | "SetMultiplyPointed"
            | "SetDividePointed" | "SetModPointed" | "SetRemPointed" | "SetPowerPointed" => {
                let args = self.all_args(index)?;
                let (block, slot) = self
                    .pointed_location_values(args[0], args[1], args[2], true)?
                    .context("pointed write did not resolve an address")?;
                let value = args[3];
                let old = self.memory_get(block, slot);
                let result = match name {
                    "SetPointed" => value,
                    "SetAddPointed" => old + value,
                    "SetSubtractPointed" => old - value,
                    "SetMultiplyPointed" => old * value,
                    "SetDividePointed" => old / value,
                    "SetModPointed" => modulus(old, value)?,
                    "SetRemPointed" => old % value,
                    "SetPowerPointed" => old.powf(value),
                    _ => unreachable!(),
                };
                self.memory_set(block, slot, result, index)?;
                self.record_diagnostic(|| {
                    format!("{name} node={index} target={block}[{slot}] old={old} new={result}")
                });
                Ok(result)
            }
            "IncrementPre"
            | "IncrementPost"
            | "DecrementPre"
            | "DecrementPost"
            | "IncrementPrePointed"
            | "IncrementPostPointed"
            | "DecrementPrePointed"
            | "DecrementPostPointed" => {
                let pointed = name.ends_with("Pointed");
                let (block, slot) = if pointed {
                    let args = self.all_args(index)?;
                    self.pointed_location_values(args[0], args[1], args[2], true)?
                        .context("pointed update did not resolve an address")?
                } else {
                    (
                        integer(self.arg(index, 0)?, "block id")?,
                        index_of(self.arg(index, 1)?)?,
                    )
                };
                let old = self.memory_get(block, slot);
                let new = if name.starts_with("Increment") {
                    old + 1.0
                } else {
                    old - 1.0
                };
                self.memory_set(block, slot, new, index)?;
                self.record_diagnostic(|| {
                    format!("{name} node={index} block={block} index={slot} old={old} new={new}")
                });
                // Sonolus names the returned state: Pre returns the value
                // before the update, Post returns the value after it.
                Ok(if name.contains("Pre") { old } else { new })
            }
            "IncrementPreShifted"
            | "IncrementPostShifted"
            | "DecrementPreShifted"
            | "DecrementPostShifted" => {
                let block = integer(self.arg(index, 0)?, "block id")?;
                let x = self.arg(index, 1)?;
                let y = self.arg(index, 2)?;
                let stride = self.arg(index, 3)?;
                let slot = index_of(x + y * stride)?;
                let old = self.memory_get(block, slot);
                let new = if name.starts_with("Increment") {
                    old + 1.0
                } else {
                    old - 1.0
                };
                self.memory_set(block, slot, new, index)?;
                self.record_diagnostic(|| {
                    format!("{name} node={index} block={block} x={x} y={y} stride={stride} index={slot} old={old} new={new}")
                });
                Ok(if name.contains("Pre") { old } else { new })
            }
            "Draw" => {
                let args = self.all_args(index)?;
                if args.len() != 11 && args.len() != 14 {
                    bail!("Draw requires 11 or 14 arguments, got {}", args.len());
                }
                // A controlled Sonolus v1.1.4 Watch oracle showed that the
                // exact sprite sentinel -1 draws nothing and execution
                // continues. Keep this narrowly scoped; other negative IDs
                // still follow the normal integer/u32 validation below.
                if args[0] == -1.0 {
                    return Ok(0.0);
                }
                let sprite_id = integer(args[0], "sprite id")?;
                let sprite_id =
                    u32::try_from(sprite_id).context("sprite id is outside u32 range")?;
                self.display_list.sprites.push(SpriteDraw {
                    sprite_id,
                    corners: [
                        [args[1], args[2]],
                        [args[3], args[4]],
                        [args[5], args[6]],
                        [args[7], args[8]],
                    ],
                    z: [
                        args[9],
                        *args.get(11).unwrap_or(&args[9]),
                        *args.get(12).unwrap_or(&args[9]),
                        *args.get(13).unwrap_or(&args[9]),
                    ],
                    alpha: args[10],
                    provenance: Some(DrawProvenance {
                        entity_id: self.context.entity_id,
                        archetype: self.context.entity_archetype.clone(),
                        callback: self.context.callback_name.clone(),
                        callback_node: self.context.callback_node,
                        draw_node: index,
                    }),
                    trace: self.trace_draws.then(|| DrawTrace {
                        argument_nodes: self.nodes[index]
                            .args
                            .iter()
                            .map(|argument| {
                                argument
                                    .as_u64()
                                    .map(|node| node as usize)
                                    .unwrap_or(usize::MAX)
                            })
                            .collect(),
                        node_values: self.draw_argument_values.clone(),
                        node_value_texts: self.draw_argument_value_texts.clone(),
                        memory_reads: self.draw_memory_reads.clone(),
                        memory_writes: self
                            .last_memory_writes
                            .iter()
                            .map(|(&(block, index), &(node, value))| MemoryWriteTrace {
                                node,
                                block,
                                index,
                                value,
                                value_text: trace_value(value),
                            })
                            .collect(),
                        memory_operations: self.draw_memory_operations.clone(),
                    }),
                });
                Ok(0.0)
            }
            "Spawn" => {
                if count == 0 {
                    bail!("Spawn requires an archetype identifier");
                }
                let mut args = self.all_args(index)?.into_iter();
                let archetype_id = integer(args.next().unwrap_or_default(), "archetype id")?;
                self.spawn_queue.push(SpawnRequest {
                    archetype_id,
                    data: args.collect(),
                });
                Ok(0.0)
            }
            "Play" => {
                let args = self.all_args(index)?;
                if args.len() != 2 {
                    bail!("Play requires 2 arguments, got {}", args.len());
                }
                self.audio_events.push(AudioEffectEvent::Play {
                    clip_id: integer(args[0], "effect clip id")?,
                    minimum_distance: args[1],
                    requested_at: self.context.time,
                });
                Ok(0.0)
            }
            "PlayLooped" => {
                let args = self.all_args(index)?;
                if args.len() != 1 {
                    bail!("PlayLooped requires 1 argument, got {}", args.len());
                }
                let clip_id = integer(args[0], "effect clip id")?;
                let instance_id = self.allocate_looped_effect_id()?;
                self.audio_events.push(AudioEffectEvent::StartLoop {
                    instance_id,
                    clip_id,
                    requested_at: self.context.time,
                });
                Ok(instance_id as f64)
            }
            "StopLooped" => {
                let args = self.all_args(index)?;
                if args.len() != 1 {
                    bail!("StopLooped requires 1 argument, got {}", args.len());
                }
                self.audio_events.push(AudioEffectEvent::StopLoop {
                    instance_id: integer(args[0], "looped effect instance id")?,
                    requested_at: self.context.time,
                });
                Ok(0.0)
            }
            "PlayScheduled" => {
                let args = self.all_args(index)?;
                if args.len() != 3 {
                    bail!("PlayScheduled requires 3 arguments, got {}", args.len());
                }
                let clip_id = integer(args[0], "effect clip id")?;
                if clip_id < 0 {
                    bail!("effect clip id must be nonnegative, got {clip_id}");
                }
                let time = args[1];
                let minimum_distance = args[2];
                self.scheduled_effects.push(ScheduledEffect {
                    clip_id,
                    time,
                    minimum_distance,
                    requested_at: self.context.time,
                    has_required_lead_time: time - self.context.time >= 0.5,
                });
                Ok(0.0)
            }
            "PlayLoopedScheduled" => {
                let args = self.all_args(index)?;
                if args.len() != 2 {
                    bail!(
                        "PlayLoopedScheduled requires 2 arguments, got {}",
                        args.len()
                    );
                }
                let clip_id = integer(args[0], "effect clip id")?;
                if clip_id < 0 {
                    bail!("effect clip id must be nonnegative, got {clip_id}");
                }
                let start_time = args[1];
                let instance_id = self.allocate_looped_effect_id()?;
                self.scheduled_looped_effects.push(ScheduledLoopedEffect {
                    instance_id,
                    clip_id,
                    start_time,
                    requested_at: self.context.time,
                    has_required_lead_time: start_time - self.context.time >= 0.5,
                });
                Ok(instance_id as f64)
            }
            "StopLoopedScheduled" => {
                let args = self.all_args(index)?;
                if args.len() != 2 {
                    bail!(
                        "StopLoopedScheduled requires 2 arguments, got {}",
                        args.len()
                    );
                }
                let instance_id = integer(args[0], "looped effect instance id")?;
                let end_time = args[1];
                self.scheduled_looped_effect_stops
                    .push(ScheduledLoopedEffectStop {
                        instance_id,
                        end_time,
                        requested_at: self.context.time,
                        has_required_lead_time: end_time - self.context.time >= 0.5,
                    });
                Ok(0.0)
            }
            "DestroyParticleEffect" => {
                let args = self.all_args(index)?;
                if args.len() != 1 {
                    bail!(
                        "DestroyParticleEffect requires 1 argument, got {}",
                        args.len()
                    );
                }
                let particle_id = integer(args[0], "particle effect instance id")?;
                self.context
                    .particle_instances
                    .write()
                    .map_err(|_| anyhow::anyhow!("particle instance state lock was poisoned"))?
                    .instances
                    .remove(&particle_id);
                self.destroyed_particle_effects
                    .push(DestroyedParticleEffect { particle_id });
                self.particle_events.push(ParticleEffectEvent::Destroy {
                    instance_id: particle_id,
                });
                Ok(0.0)
            }
            "MoveParticleEffect" => {
                let args = self.all_args(index)?;
                if args.len() != 9 {
                    bail!(
                        "MoveParticleEffect requires 9 arguments, got {}",
                        args.len()
                    );
                }
                let instance_id = integer(args[0], "particle effect instance id")?;
                let corners = [
                    [args[1], args[2]],
                    [args[3], args[4]],
                    [args[5], args[6]],
                    [args[7], args[8]],
                ];
                if let Some(instance) = self
                    .context
                    .particle_instances
                    .write()
                    .map_err(|_| anyhow::anyhow!("particle instance state lock was poisoned"))?
                    .instances
                    .get_mut(&instance_id)
                {
                    instance.corners = corners;
                }
                self.particle_events.push(ParticleEffectEvent::Move {
                    instance_id,
                    corners,
                });
                Ok(0.0)
            }
            "SpawnParticleEffect" => {
                let args = self.all_args(index)?;
                if args.len() != 11 {
                    bail!(
                        "SpawnParticleEffect requires 11 arguments, got {}",
                        args.len()
                    );
                }
                let effect_id = integer(args[0], "particle effect id")?;
                if effect_id < 0 {
                    // A missing particle binding is represented by -1 in
                    // WatchData generated by engines. The SDB reference
                    // returns 0 when the requested effect ID is absent; do
                    // not allocate an instance for this sentinel.
                    return Ok(0.0);
                }
                let corners = [
                    [args[1], args[2]],
                    [args[3], args[4]],
                    [args[5], args[6]],
                    [args[7], args[8]],
                ];
                let duration = args[9];
                let is_looped = args[10] != 0.0;

                // The specification requires a unique instance identifier but
                // leaves its numeric origin unspecified. Allocate monotonically
                // from zero so identifiers are unique and deterministic.
                let instance = {
                    let mut state = self.context.particle_instances.write().map_err(|_| {
                        anyhow::anyhow!("particle instance state lock was poisoned")
                    })?;
                    let instance_id = state.next_instance_id;
                    if instance_id > (1_i64 << 53) {
                        bail!("particle instance identifier space exhausted");
                    }
                    state.next_instance_id = instance_id
                        .checked_add(1)
                        .context("particle instance identifier space exhausted")?;
                    let instance = ParticleEffectInstance {
                        instance_id,
                        effect_id,
                        corners,
                        duration,
                        is_looped,
                        spawned_at: self.context.time,
                    };
                    state.instances.insert(instance_id, instance.clone());
                    instance
                };
                self.particle_events
                    .push(ParticleEffectEvent::Spawn(instance.clone()));
                Ok(instance.instance_id as f64)
            }
            "Copy" => {
                if count != 5 {
                    bail!("Copy requires exactly five arguments, got {count}");
                }
                let source_block = integer(self.arg(index, 0)?, "Copy source block")?;
                let source_index = index_of(self.arg(index, 1)?)?;
                let destination_block = integer(self.arg(index, 2)?, "Copy destination block")?;
                let destination_index = index_of(self.arg(index, 3)?)?;
                let count = integer(self.arg(index, 4)?, "Copy count")?;
                let count = usize::try_from(count).context("Copy count must be non-negative")?;
                source_index
                    .checked_add(count)
                    .context("Copy source range overflows address space")?;
                destination_index
                    .checked_add(count)
                    .context("Copy destination range overflows address space")?;

                // Snapshot the source before writing so both overlap directions
                // preserve the original source values as the Sonolus spec requires.
                let mut values = Vec::new();
                values
                    .try_reserve_exact(count)
                    .context("unable to allocate Copy source snapshot")?;
                for offset in 0..count {
                    values.push(self.memory_get(source_block, source_index + offset));
                }
                for (offset, value) in values.into_iter().enumerate() {
                    self.memory_set(destination_block, destination_index + offset, value, index)?;
                }
                Ok(0.0)
            }
            "And" => {
                let argument_count = self.nodes[index].args.len();
                if argument_count == 0 {
                    bail!("And requires at least one argument");
                }
                let mut result = 0.0;
                for argument in 0..argument_count {
                    result = self.arg(index, argument)?;
                    if result == 0.0 {
                        return Ok(0.0);
                    }
                }
                Ok(result)
            }
            "Or" => {
                if count == 0 {
                    bail!("Or requires at least one argument");
                }
                for argument in 0..count {
                    let value = self.arg(index, argument)?;
                    if value != 0.0 {
                        return Ok(value);
                    }
                }
                Ok(0.0)
            }
            "Add" | "Multiply" | "Min" | "Max" | "Mod" | "Rem" => {
                let args = self.all_args(index)?;
                let first = *args
                    .first()
                    .context("variadic function requires an argument")?;
                Ok(match name {
                    "Add" => args.iter().sum(),
                    "Multiply" => args.iter().product(),
                    "Min" => args.iter().copied().fold(first, f64::min),
                    "Max" => args.iter().copied().fold(first, f64::max),
                    "Mod" => args
                        .windows(2)
                        .try_fold(first, |a, pair| modulus(a, pair[1]))?,
                    "Rem" => args.windows(2).fold(first, |a, pair| a % pair[1]),
                    _ => unreachable!(),
                })
            }
            "Subtract" => {
                let args = self.all_args(index)?;
                let first = *args
                    .first()
                    .context("variadic function requires an argument")?;
                Ok(args
                    .iter()
                    .skip(1)
                    .fold(first, |result, value| result - value))
            }
            "Divide" | "Power" | "Equal" | "NotEqual" | "Greater" | "GreaterOr" | "Less"
            | "LessOr" => {
                let (left, right) = binary(self)?;
                Ok(match name {
                    "Divide" => left / right,
                    "Power" => left.powf(right),
                    "Equal" => truth(left == right),
                    "NotEqual" => truth(left != right),
                    "Greater" => truth(left > right),
                    "GreaterOr" => truth(left >= right),
                    "Less" => truth(left < right),
                    "LessOr" => truth(left <= right),
                    _ => unreachable!(),
                })
            }
            "Arctan2" => {
                if count != 2 {
                    bail!("Arctan2 requires exactly two arguments");
                }
                let (y, x) = binary(self)?;
                Ok(y.atan2(x))
            }
            "Abs" | "Arctan" | "Ceil" | "Cos" | "Floor" | "Log" | "Negate" | "Not" | "Round"
            | "Sin" | "Trunc" | "Frac" | "Sign" | "Radian" | "Degree" | "Arcsin" | "Arccos"
            | "Tan" | "Cosh" | "Sinh" | "Tanh" => {
                let value = unary(self)?;
                Ok(match name {
                    "Abs" => value.abs(),
                    "Arctan" => value.atan(),
                    "Ceil" => value.ceil(),
                    "Cos" => value.cos(),
                    "Floor" => value.floor(),
                    "Log" => value.ln(),
                    "Negate" => -value,
                    "Not" => truth(value == 0.0),
                    "Round" => value.round(),
                    "Sin" => value.sin(),
                    "Trunc" => value.trunc(),
                    "Frac" => value - value.trunc(),
                    "Sign" => value.signum(),
                    "Radian" => value.to_radians(),
                    "Degree" => value.to_degrees(),
                    "Arcsin" => value.asin(),
                    "Arccos" => value.acos(),
                    "Tan" => value.tan(),
                    "Cosh" => value.cosh(),
                    "Sinh" => value.sinh(),
                    "Tanh" => value.tanh(),
                    _ => unreachable!(),
                })
            }
            "Clamp" | "Lerp" | "LerpClamped" | "Unlerp" | "UnlerpClamped" | "Remap"
            | "RemapClamped" => {
                let args = self.all_args(index)?;
                if args.len() < 3 {
                    bail!("{name} requires at least three arguments");
                }
                let (x, a, b) = (args[0], args[1], args[2]);
                let result = match name {
                    "Clamp" => x.clamp(a.min(b), a.max(b)),
                    "Lerp" | "LerpClamped" => {
                        if args.len() != 3 {
                            bail!("{name} requires three arguments");
                        }
                        let t = if name == "LerpClamped" {
                            b.clamp(0.0, 1.0)
                        } else {
                            b
                        };
                        x + (a - x) * t
                    }
                    // Sonolus orders Unlerp arguments as (minimum, maximum, value),
                    // matching Lerp's endpoint-first convention.
                    "Unlerp" => (b - x) / (a - x),
                    "UnlerpClamped" => ((b - x) / (a - x)).clamp(0.0, 1.0),
                    "Remap" | "RemapClamped" if args.len() == 5 => {
                        let (from_min, from_max, to_min, to_max, value) =
                            (args[0], args[1], args[2], args[3], args[4]);
                        let t = (value - from_min) / (from_max - from_min);
                        let t = if name == "RemapClamped" {
                            t.clamp(0.0, 1.0)
                        } else {
                            t
                        };
                        to_min + (to_max - to_min) * t
                    }
                    _ => bail!("{name} requires five arguments"),
                };
                Ok(result)
            }
            name if name.starts_with("Ease") => {
                let value = unary(self)?;
                let suffix = name.strip_prefix("Ease").unwrap_or_default();
                let (direction, family) = ["InOut", "OutIn", "In", "Out"]
                    .into_iter()
                    .find_map(|direction| {
                        suffix
                            .strip_prefix(direction)
                            .map(|family| (direction, family))
                    })
                    .context(format!("unsupported Watch function {name} at node {index}"))?;
                if !matches!(
                    family,
                    "Sine"
                        | "Quad"
                        | "Cubic"
                        | "Quart"
                        | "Quint"
                        | "Expo"
                        | "Circ"
                        | "Back"
                        | "Elastic"
                ) {
                    bail!("unsupported Watch function {name} at node {index}");
                }
                Ok(ease_curve(family, direction, value))
            }
            _ => bail!("unsupported Watch function {name} at node {index}"),
        }
    }
}

/// Evaluate the documented Sonolus easing families. Compound curves compose
/// the corresponding In/Out halves and preserve extrapolation outside [0, 1].
pub(crate) fn ease_curve(family: &str, direction: &str, value: f64) -> f64 {
    fn ease_in(family: &str, x: f64) -> f64 {
        match family {
            "Sine" => 1.0 - (x * std::f64::consts::FRAC_PI_2).cos(),
            "Quad" => x * x,
            "Cubic" => x * x * x,
            "Quart" => x * x * x * x,
            "Quint" => x * x * x * x * x,
            "Expo" => {
                if x == 0.0 {
                    0.0
                } else {
                    (2.0_f64).powf(10.0 * x - 10.0)
                }
            }
            "Circ" => 1.0 - (1.0 - x * x).sqrt(),
            "Back" => {
                const C1: f64 = 1.70158;
                const C3: f64 = C1 + 1.0;
                C3 * x * x * x - C1 * x * x
            }
            "Elastic" => {
                if x == 0.0 {
                    0.0
                } else if x == 1.0 {
                    1.0
                } else {
                    -(2.0_f64).powf(10.0 * x - 10.0)
                        * ((x * 10.0 - 10.75) * (2.0 * std::f64::consts::PI / 3.0)).sin()
                }
            }
            _ => unreachable!("family validated by caller"),
        }
    }

    fn ease_out(family: &str, x: f64) -> f64 {
        match family {
            "Sine" => (x * std::f64::consts::FRAC_PI_2).sin(),
            "Quad" => 1.0 - (1.0 - x) * (1.0 - x),
            "Cubic" => 1.0 - (1.0 - x).powi(3),
            "Quart" => 1.0 - (1.0 - x).powi(4),
            "Quint" => 1.0 - (1.0 - x).powi(5),
            "Expo" => {
                if x == 1.0 {
                    1.0
                } else {
                    1.0 - (2.0_f64).powf(-10.0 * x)
                }
            }
            "Circ" => (1.0 - (x - 1.0) * (x - 1.0)).sqrt(),
            "Back" => {
                const C1: f64 = 1.70158;
                const C3: f64 = C1 + 1.0;
                1.0 + C3 * (x - 1.0).powi(3) + C1 * (x - 1.0).powi(2)
            }
            "Elastic" => {
                if x == 0.0 {
                    0.0
                } else if x == 1.0 {
                    1.0
                } else {
                    (2.0_f64).powf(-10.0 * x)
                        * ((x * 10.0 - 0.75) * (2.0 * std::f64::consts::PI / 3.0)).sin()
                        + 1.0
                }
            }
            _ => unreachable!("family validated by caller"),
        }
    }

    match direction {
        "In" => ease_in(family, value),
        "Out" => ease_out(family, value),
        "InOut" => {
            if value < 0.5 {
                ease_in(family, value * 2.0) / 2.0
            } else {
                ease_out(family, value * 2.0 - 1.0) / 2.0 + 0.5
            }
        }
        "OutIn" => {
            if value < 0.5 {
                ease_out(family, value * 2.0) / 2.0
            } else {
                ease_in(family, value * 2.0 - 1.0) / 2.0 + 0.5
            }
        }
        _ => unreachable!("direction validated by caller"),
    }
}

#[cfg(test)]
mod easing_tests {
    use super::ease_curve;

    #[test]
    fn documented_easings_preserve_nan_instead_of_clamping() {
        for family in [
            "Sine", "Quad", "Cubic", "Quart", "Quint", "Expo", "Circ", "Back", "Elastic",
        ] {
            for direction in ["In", "Out", "InOut", "OutIn"] {
                assert!(
                    ease_curve(family, direction, f64::NAN).is_nan(),
                    "{direction}{family}"
                );
            }
        }
    }
}

impl VmContext {
    pub fn scaled_time(&self, time: f64) -> Result<f64> {
        let (anchor_time, anchor_scaled) = self
            .time_map
            .iter()
            .rev()
            .find(|(at, _)| *at <= time)
            .copied()
            .or_else(|| self.time_map.first().copied())
            .context("scaled-time conversion requires a configured time map")?;
        Ok(anchor_scaled + (time - anchor_time) * self.time_to_timescale(time)?)
    }
    pub fn time_to_timescale(&self, time: f64) -> Result<f64> {
        Ok(self
            .timescale_map
            .iter()
            .rev()
            .find(|(at, _)| *at <= time)
            .map(|(_, scale)| *scale)
            .unwrap_or(1.0))
    }
    fn time_to_starting_time(&self, time: f64) -> Result<f64> {
        Ok(time - self.starting_time)
    }
    fn time_to_starting_scaled_time(&self, time: f64) -> Result<f64> {
        Ok(self.scaled_time(time)? - self.scaled_time(self.starting_time)?)
    }
    pub(crate) fn beat_to_time(&self, beat: f64) -> Result<f64> {
        if self.bpm_map.is_empty() {
            bail!("beat conversion requires a configured BPM map");
        }
        let mut elapsed = 0.0;
        let mut previous_beat = self.bpm_map[0].0;
        let mut bpm = self.bpm_map[0].1;
        for (next_beat, next_bpm) in self.bpm_map.iter().copied().skip(1) {
            if beat <= next_beat {
                return Ok(elapsed + (beat - previous_beat) * 60.0 / bpm);
            }
            elapsed += (next_beat - previous_beat) * 60.0 / bpm;
            previous_beat = next_beat;
            bpm = next_bpm;
        }
        Ok(elapsed + (beat - previous_beat) * 60.0 / bpm)
    }
    fn beat_to_starting_time(&self, beat: f64) -> Result<f64> {
        Ok(self.beat_to_time(beat)? - self.starting_time)
    }
    fn beat_to_starting_beat(&self, beat: f64) -> Result<f64> {
        Ok(beat - self.starting_beat)
    }
    fn bpm_at_beat(&self, beat: f64) -> Result<f64> {
        self.bpm_map
            .iter()
            .rev()
            .find(|(b, _)| *b <= beat)
            .map(|(_, v)| *v)
            .or_else(|| self.bpm_map.first().map(|(_, v)| *v))
            .context("BPM query requires a configured BPM map")
    }
}

fn integer(value: f64, label: &str) -> Result<i64> {
    if !value.is_finite()
        || value.fract() != 0.0
        || value < i64::MIN as f64
        || value > i64::MAX as f64
    {
        bail!("{label} must be a finite integer, got {value}");
    }
    Ok(value as i64)
}

fn modulus(left: f64, right: f64) -> Result<f64> {
    if right == 0.0 {
        bail!("modulus by zero");
    }
    Ok(((left % right) + right.abs()) % right.abs())
}

fn stream_value(events: &[(f64, f64)], key: f64) -> f64 {
    if events.is_empty() {
        return 0.0;
    }
    if key <= events[0].0 {
        return events[0].1;
    }
    for pair in events.windows(2) {
        let (left_key, left_value) = pair[0];
        let (right_key, right_value) = pair[1];
        if key <= right_key {
            return left_value
                + (right_value - left_value) * ((key - left_key) / (right_key - left_key));
        }
    }
    events.last().map(|(_, value)| *value).unwrap_or(0.0)
}

fn truth(value: bool) -> f64 {
    if value {
        1.0
    } else {
        0.0
    }
}

fn trace_value(value: f64) -> String {
    if value.is_nan() {
        "NaN".to_owned()
    } else if value == f64::INFINITY {
        "+Infinity".to_owned()
    } else if value == f64::NEG_INFINITY {
        "-Infinity".to_owned()
    } else {
        value.to_string()
    }
}

fn memory_origin_key(
    entity_id: Option<usize>,
    block: i64,
    slot: usize,
) -> (Option<usize>, i64, usize) {
    let owner = matches!(block, 4000 | 4001 | 4002)
        .then_some(entity_id)
        .flatten();
    (owner, block, slot)
}

fn index_of(value: f64) -> Result<usize> {
    if !value.is_finite() || value.fract() != 0.0 || value < 0.0 || value > usize::MAX as f64 {
        bail!("memory index must be a finite non-negative integer, got {value}");
    }
    Ok(value as usize)
}

#[cfg(test)]
mod memory_partition_tests {
    use super::Memory;

    #[test]
    fn entity_commit_split_matches_previous_clone_and_filter_rules() {
        let mut input = Memory::new();
        for block in [3999, 4000, 4001, 4002, 4003, 9999, 10000, 10001] {
            input.set(block, 3, block as f64 + 0.25);
        }
        input.set(4000, 7, -4.5);

        let mut expected_global = input.clone();
        for excluded in [4000, 4001, 4002, 10000] {
            expected_global.retain_other_than(excluded);
        }
        let mut expected_entity = input.clone();
        expected_entity.retain_only(4000);

        let (actual_global, actual_entity) =
            input.into_watch_entity_parts(4000, &[4001, 4002, 10000]);
        assert_eq!(actual_global.values, expected_global.values);
        assert_eq!(actual_entity.values, expected_entity.values);
    }

    #[test]
    fn disabling_vm_accounting_keeps_evaluation_result_and_omits_operation_map() {
        let nodes: Vec<crate::watch::EngineNode> = serde_json::from_value(serde_json::json!([
            {"value": 4}, {"value": 6}, {"func": "Add", "args": [0, 1]}
        ]))
        .unwrap();
        let mut vm = super::WatchVm::new(&nodes);
        assert_eq!(vm.execute(2).unwrap(), 10.0);
        assert_eq!(vm.function_counts.get("Add"), Some(&1));

        vm.function_counts.clear();
        vm.set_accounting(false);
        assert_eq!(vm.execute(2).unwrap(), 10.0);
        assert!(vm.function_counts.is_empty());
    }
}
