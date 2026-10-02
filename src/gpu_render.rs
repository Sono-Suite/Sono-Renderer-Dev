//! Offscreen wgpu pixel backend. Sonolus execution, binding, transforms, and
//! draw ordering remain on the CPU; this module only rasterizes prepared draws.

use anyhow::{bail, Context, Result};
use std::{
    collections::HashMap,
    sync::{mpsc, Arc, Mutex, OnceLock},
};
use wgpu::util::DeviceExt;

const SHADER: &str = r#"
struct Params {
  corners: array<vec4<f32>, 4>,
  atlas: vec4<f32>, // x, y, width, height
  frame: vec4<f32>, // width, height, aspect, alpha
  tint: vec4<f32>,
  sampling: vec4<f32>, // interpolation, background flag, padding
};
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var tex: texture_2d<f32>;

struct Out { @builtin(position) pos: vec4<f32>, };
@vertex fn vs(@builtin(vertex_index) index: u32) -> Out {
  let x0 = min(min(p.corners[0].x, p.corners[1].x), min(p.corners[2].x, p.corners[3].x));
  let x1 = max(max(p.corners[0].x, p.corners[1].x), max(p.corners[2].x, p.corners[3].x));
  let y0 = min(min(p.corners[0].y, p.corners[1].y), min(p.corners[2].y, p.corners[3].y));
  let y1 = max(max(p.corners[0].y, p.corners[1].y), max(p.corners[2].y, p.corners[3].y));
  let px0 = clamp(floor((x0 / p.frame.z + 1.0) * 0.5 * p.frame.x), 0.0, p.frame.x);
  let px1 = clamp(ceil((x1 / p.frame.z + 1.0) * 0.5 * p.frame.x), 0.0, p.frame.x);
  let py0 = clamp(floor((1.0-y1) * 0.5 * p.frame.y), 0.0, p.frame.y);
  let py1 = clamp(ceil((1.0-y0) * 0.5 * p.frame.y), 0.0, p.frame.y);
  var points = array<vec2<f32>, 6>(
    vec2(px0,py0), vec2(px1,py0), vec2(px0,py1),
    vec2(px0,py1), vec2(px1,py0), vec2(px1,py1));
  let q = points[index];
  var out: Out;
  out.pos = vec4(q.x / p.frame.x * 2.0 - 1.0, 1.0 - q.y / p.frame.y * 2.0, 0.0, 1.0);
  return out;
}

fn invbilinear(coord: vec2<f32>) -> vec2<f32> {
  let p00=p.corners[0].xy; let p10=p.corners[3].xy; let p01=p.corners[1].xy; let p11=p.corners[2].xy;
  let cross=p11-p10-p01+p00;
  var uv=vec2<f32>(0.5,0.5);
  for (var i=0u; i<12u; i=i+1u) {
    let point=p00+uv.x*(p10-p00)+uv.y*(p01-p00)+uv.x*uv.y*cross;
    let error=point-coord;
    if max(abs(error.x),abs(error.y)) < 1e-7 { return uv; }
    let du=p10-p00+uv.y*cross; let dv=p01-p00+uv.x*cross;
    let det=du.x*dv.y-dv.x*du.y;
    if abs(det)<1e-12 { return vec2(-1000.0,-1000.0); }
    uv.x -= (error.x*dv.y-dv.x*error.y)/det;
    uv.y -= (du.x*error.y-error.x*du.y)/det;
  }
  let point=p00+uv.x*(p10-p00)+uv.y*(p01-p00)+uv.x*uv.y*cross;
  if max(abs(point.x-coord.x),abs(point.y-coord.y))<1e-5 { return uv; }
  return vec2(-1000.0,-1000.0);
}
fn sample_at(x0: i32, y0: i32, fx: f32, fy: f32) -> vec4<f32> {
  let xlo=i32(p.atlas.x); let ylo=i32(p.atlas.y);
  let xhi=xlo+i32(p.atlas.z)-1; let yhi=ylo+i32(p.atlas.w)-1;
  let a=textureLoad(tex,vec2<i32>(clamp(x0,xlo,xhi),clamp(y0,ylo,yhi)),0);
  if p.sampling.x < 0.5 { return a; }
  let b=textureLoad(tex,vec2<i32>(clamp(x0+1,xlo,xhi),clamp(y0,ylo,yhi)),0);
  let c=textureLoad(tex,vec2<i32>(clamp(x0,xlo,xhi),clamp(y0+1,ylo,yhi)),0);
  let d=textureLoad(tex,vec2<i32>(clamp(x0+1,xlo,xhi),clamp(y0+1,ylo,yhi)),0);
  return floor(((a*(1.0-fx)+b*fx)*(1.0-fy)+(c*(1.0-fx)+d*fx)*fy)*255.0+0.5)/255.0;
}
@fragment fn fs(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
  if p.sampling.z > 0.5 { return vec4(p.tint.rgb, p.frame.w); }
  let coord=vec2((frag.x/p.frame.x*2.0-1.0)*p.frame.z, 1.0-frag.y/p.frame.y*2.0);
  let uv=invbilinear(coord);
  if uv.x < -1e-7 || uv.x > 1.0000001 || uv.y < -1e-7 || uv.y > 1.0000001 { discard; }
  var texel: vec4<f32>;
  if p.sampling.y > 0.5 {
    let tx=p.atlas.x+clamp(uv.x,0.0,1.0)*(p.atlas.z-1.0);
    let ty=p.atlas.y+(1.0-clamp(uv.y,0.0,1.0))*(p.atlas.w-1.0);
    texel=sample_at(i32(floor(tx)),i32(floor(ty)),fract(tx),fract(ty));
  } else {
    let tx=p.atlas.x+clamp(uv.x,0.0,1.0)*p.atlas.z-0.5;
    let ty=p.atlas.y+(1.0-clamp(uv.y,0.0,1.0))*p.atlas.w-0.5;
    texel=sample_at(i32(floor(tx)),i32(floor(ty)),fract(tx),fract(ty));
  }
  let rgb=texel.rgb*p.tint.rgb;
  return vec4(rgb, texel.a*p.frame.w*p.tint.a);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    corners: [[f32; 4]; 4],
    atlas: [f32; 4],
    frame: [f32; 4],
    tint: [f32; 4],
    sampling: [f32; 4],
}

struct DrawOp {
    rect: [u32; 4],
    corners: [[f64; 2]; 4],
    alpha: f64,
    tint: [f64; 4],
    interpolation: bool,
    atlas: usize,
    background: bool,
    mask: bool,
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    adapter_info: wgpu::AdapterInfo,
}

pub(crate) struct GpuRenderer {
    gpu: Gpu,
    atlases: Mutex<HashMap<[u8; 20], Arc<wgpu::Texture>>>,
    adapter: String,
}

#[derive(Default)]
pub(crate) struct GpuFrameProfile {
    pub draw_preparation: std::time::Duration,
    pub encode_submit: std::time::Duration,
    pub wait_map: std::time::Duration,
    pub unpack: std::time::Duration,
    pub draw_calls: usize,
    pub atlas_uploads: usize,
    pub readback_bytes: u64,
    pub adapter: Option<String>,
}

static SHARED_GPU: OnceLock<std::result::Result<GpuRenderer, String>> = OnceLock::new();

impl GpuRenderer {
    pub(crate) fn new() -> Result<Self> {
        let gpu = gpu()?;
        Ok(Self {
            adapter: format!(
                "{} / {:?} / {:?}",
                gpu.adapter_info.name, gpu.adapter_info.backend, gpu.adapter_info.device_type
            ),
            gpu,
            atlases: Mutex::new(HashMap::new()),
        })
    }

    pub(crate) fn shared() -> Result<&'static Self> {
        match SHARED_GPU.get_or_init(|| Self::new().map_err(|error| format!("{error:#}"))) {
            Ok(renderer) => Ok(renderer),
            Err(error) => bail!("initializing shared wgpu backend: {error}"),
        }
    }
}

fn gpu() -> Result<Gpu> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
        apply_limit_buckets: false,
    }))
    .context("no wgpu adapter is available")?;
    let adapter_info = adapter.get_info();
    if adapter_info.device_type == wgpu::DeviceType::Cpu {
        eprintln!(
            "Warning: wgpu selected software/CPU adapter {:?} ({}); rendering may be slow",
            adapter_info.device_type, adapter_info.name
        );
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("Sono Renderer offscreen device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        ..Default::default()
    }))
    .context("requesting wgpu device")?;
    let validation_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Sonolus sprite raster shader"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("sprite draw layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("sprite pipeline layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Sonolus ordered sprite pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    if let Some(error) = pollster::block_on(validation_scope.pop()) {
        bail!("validating wgpu shader and pipeline: {error}");
    }
    Ok(Gpu {
        device,
        queue,
        pipeline,
        layout,
        adapter_info,
    })
}

fn render(
    renderer: &GpuRenderer,
    atlases: &[(&[u8], u32, u32)],
    width: u32,
    height: u32,
    aspect: f64,
    initial_rgb: &[u8],
    draws: Vec<DrawOp>,
    mut profile: Option<&mut GpuFrameProfile>,
) -> Result<Vec<u8>> {
    let gpu = &renderer.gpu;
    let encode_start;
    if let Some(p) = profile.as_deref_mut() {
        p.draw_calls = draws.len();
        p.adapter = Some(renderer.adapter.clone());
    }
    let validation_scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen RGBA framebuffer"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let initial_rgba = rgb_to_rgba(initial_rgb);
    if initial_rgba.len() != width as usize * height as usize * 4 {
        bail!("initial framebuffer has invalid dimensions");
    }
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &initial_rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    use sha1::{Digest, Sha1};
    let mut resident = Vec::with_capacity(atlases.len());
    for (input, iw, ih) in atlases {
        let mut digest = Sha1::new();
        digest.update(iw.to_le_bytes());
        digest.update(ih.to_le_bytes());
        digest.update(input);
        let key: [u8; 20] = digest.finalize().into();
        if profile.is_some() {
            let cache_miss = renderer
                .atlases
                .lock()
                .map(|cache| !cache.contains_key(&key))
                .unwrap_or(false);
            if cache_miss {
                if let Some(p) = profile.as_deref_mut() {
                    p.atlas_uploads += 1;
                }
            }
        }
        let atlas = {
            let mut cache = renderer
                .atlases
                .lock()
                .map_err(|_| anyhow::anyhow!("wgpu atlas cache lock was poisoned"))?;
            if !cache.contains_key(&key) && cache.len() >= 8 {
                cache.clear();
            }
            cache
                .entry(key)
                .or_insert_with(|| {
                    let atlas = gpu.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("resident source atlas"),
                        size: wgpu::Extent3d {
                            width: *iw,
                            height: *ih,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    });
                    gpu.queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &atlas,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        input,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(*iw * 4),
                            rows_per_image: Some(*ih),
                        },
                        wgpu::Extent3d {
                            width: *iw,
                            height: *ih,
                            depth_or_array_layers: 1,
                        },
                    );
                    Arc::new(atlas)
                })
                .clone()
        };
        resident.push(atlas.create_view(&Default::default()));
    }
    let view = tex.create_view(&Default::default());
    let stride = width.checked_mul(4).context("frame row overflow")?;
    let padded =
        stride.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("framebuffer readback"),
        size: u64::from(padded) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encode_start = profile.as_ref().map(|_| std::time::Instant::now());
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("offscreen ordered frame"),
        });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ordered Sonolus draw list"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&gpu.pipeline);
        for d in draws {
            let [x, y, w, h] = d.rect;
            let params = Params {
                corners: d.corners.map(|p| [p[0] as f32, p[1] as f32, 0.0, 0.0]),
                atlas: [x as f32, y as f32, w as f32, h as f32],
                frame: [width as f32, height as f32, aspect as f32, d.alpha as f32],
                tint: d.tint.map(|v| v as f32),
                sampling: [
                    if d.interpolation || d.background {
                        1.0
                    } else {
                        0.0
                    },
                    if d.background { 1.0 } else { 0.0 },
                    if d.mask { 1.0 } else { 0.0 },
                    0.0,
                ],
            };
            let ub = gpu
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("ordered draw parameters"),
                    contents: bytemuck::bytes_of(&params),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ordered sprite draw"),
                layout: &gpu.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: ub.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&resident[d.atlas]),
                    },
                ],
            });
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..6, 0..1);
        }
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit(Some(encoder.finish()));
    if let (Some(start), Some(p)) = (encode_start, profile.as_deref_mut()) {
        p.encode_submit = start.elapsed();
        p.readback_bytes = u64::from(padded) * u64::from(height);
    }
    let wait_start = profile.as_ref().map(|_| std::time::Instant::now());
    let slice = readback.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .context("waiting for wgpu framebuffer")?;
    if let Some(error) = pollster::block_on(validation_scope.pop()) {
        bail!("wgpu frame rendering failed validation: {error}");
    }
    rx.recv()
        .context("receiving wgpu readback status")?
        .context("mapping wgpu framebuffer")?;
    if let (Some(start), Some(p)) = (wait_start, profile.as_deref_mut()) {
        p.wait_map = start.elapsed();
    }
    let unpack_start = profile.as_ref().map(|_| std::time::Instant::now());
    let mapped = slice
        .get_mapped_range()
        .context("getting mapped wgpu framebuffer")?;
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for row in 0..height as usize {
        for pixel in
            mapped[row * padded as usize..row * padded as usize + stride as usize].chunks_exact(4)
        {
            rgb.extend_from_slice(&pixel[..3]);
        }
    }
    drop(mapped);
    readback.unmap();
    if let (Some(start), Some(p)) = (unpack_start, profile.as_deref_mut()) {
        p.unpack = start.elapsed();
    }
    Ok(rgb)
}

fn ppm(rgb: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    out.extend_from_slice(rgb);
    out
}

pub(crate) fn render_display_list(
    renderer: &GpuRenderer,
    list: &crate::runtime::DisplayList,
    width: u32,
    height: u32,
    aspect: f64,
    skin: &crate::formats::SkinAssets,
    bindings: &std::collections::BTreeMap<u32, String>,
    runtime: &[f64; 16],
    background: Option<(&crate::formats::BackgroundAssets, [[f64; 2]; 4])>,
    particles: Option<(
        &crate::formats::ParticleAssets,
        &[crate::particles::ParticleSpriteDraw],
        &[f64; 16],
    )>,
    mut profile: Option<&mut GpuFrameProfile>,
) -> Result<Vec<u8>> {
    let preparation_start = profile.as_ref().map(|_| std::time::Instant::now());
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || !aspect.is_finite()
        || aspect <= 0.0
    {
        bail!("frame dimensions and aspect ratio must be positive and within supported limits");
    }
    if skin.width == 0
        || skin.height == 0
        || skin.rgba.len() != skin.width as usize * skin.height as usize * 4
    {
        bail!("skin atlas dimensions do not match its RGBA pixel buffer");
    }
    let mut draws = Vec::new();
    let mut atlases = vec![(skin.rgba.as_slice(), skin.width, skin.height)];
    let mut initial = vec![0u8; width as usize * height as usize * 3];
    if let Some((bg, quad)) = background {
        if bg.configuration.blur != 0.0 {
            bail!("nonzero Sonolus background blur is not yet supported");
        }
        if bg.width == 0
            || bg.height == 0
            || bg.rgba.len() != bg.width as usize * bg.height as usize * 4
            || !quad.iter().flatten().all(|v| v.is_finite())
        {
            bail!("background asset or Runtime Background quad is invalid");
        }
        let base = crate::formats::parse_background_color(&bg.data.color, false)?;
        let mask = crate::formats::parse_background_color(&bg.configuration.mask, true)?;
        for pixel in initial.chunks_exact_mut(3) {
            pixel.copy_from_slice(&base[..3]);
        }
        let bg_atlas = atlases.len();
        atlases.push((bg.rgba.as_slice(), bg.width, bg.height));
        draws.push(DrawOp {
            rect: [0, 0, bg.width, bg.height],
            corners: quad,
            alpha: 1.0,
            tint: [1.0; 4],
            interpolation: true,
            atlas: bg_atlas,
            background: true,
            mask: false,
        });
        let full = [
            [-aspect, -1.0],
            [-aspect, 1.0],
            [aspect, 1.0],
            [aspect, -1.0],
        ];
        draws.push(DrawOp {
            rect: [0, 0, 1, 1],
            corners: full,
            alpha: mask[3] as f64 / 255.0,
            tint: [
                mask[0] as f64 / 255.0,
                mask[1] as f64 / 255.0,
                mask[2] as f64 / 255.0,
                1.0,
            ],
            interpolation: false,
            atlas: 0,
            background: false,
            mask: true,
        });
    }
    let mut order: Vec<_> = list.sprites.iter().collect();
    order.sort_by(|a, b| crate::runtime::compare_z_tuples(&a.z, &b.z));
    for d in order {
        let name = bindings
            .get(&d.sprite_id)
            .with_context(|| format!("Draw references unbound skin sprite ID {}", d.sprite_id))?;
        let s = skin
            .sprites
            .get(name)
            .with_context(|| format!("skin does not contain bound sprite {name:?}"))?;
        if s.width == 0
            || s.height == 0
            || s.x.checked_add(s.width).is_none_or(|x| x > skin.width)
            || s.y.checked_add(s.height).is_none_or(|y| y > skin.height)
        {
            bail!("skin sprite {name:?} has an invalid atlas rectangle");
        }
        let c = crate::runtime::gpu_transform_skin_corners(d, runtime, s);
        if !c.iter().flatten().all(|v| v.is_finite())
            || !d.alpha.is_finite()
            || !d.z.iter().all(|v| v.is_finite())
            || d.alpha <= 0.0
        {
            continue;
        }
        draws.push(DrawOp {
            rect: [s.x, s.y, s.width, s.height],
            corners: c,
            alpha: d.alpha.clamp(0.0, 1.0),
            tint: [1.0; 4],
            interpolation: skin.interpolation,
            atlas: 0,
            background: false,
            mask: false,
        });
    }
    if let Some((assets, particle_draws, particle_runtime)) = particles {
        let particle_atlas = atlases.len();
        if assets.width == 0
            || assets.height == 0
            || assets.rgba.len() != assets.width as usize * assets.height as usize * 4
        {
            bail!("particle atlas dimensions do not match its RGBA pixel buffer");
        }
        atlases.push((assets.rgba.as_slice(), assets.width, assets.height));
        for d in particle_draws {
            if d.alpha <= 0.0
                || !d.alpha.is_finite()
                || !d.corners.iter().flatten().all(|v| v.is_finite())
            {
                continue;
            }
            let s = assets
                .sprites
                .get(d.sprite_id)
                .with_context(|| format!("particle references missing sprite {}", d.sprite_id))?;
            if s.width == 0
                || s.height == 0
                || s.x.checked_add(s.width).is_none_or(|x| x > assets.width)
                || s.y.checked_add(s.height).is_none_or(|y| y > assets.height)
            {
                bail!("particle sprite has an invalid atlas rectangle");
            }
            let c = crate::runtime::gpu_transform_particle_corners(d, particle_runtime);
            draws.push(DrawOp {
                rect: [s.x, s.y, s.width, s.height],
                corners: c,
                alpha: d.alpha.clamp(0.0, 1.0),
                tint: [
                    d.color[0] as f64 / 255.0,
                    d.color[1] as f64 / 255.0,
                    d.color[2] as f64 / 255.0,
                    1.0,
                ],
                interpolation: assets.interpolation,
                atlas: particle_atlas,
                background: false,
                mask: false,
            });
        }
    }
    if let (Some(start), Some(p)) = (preparation_start, profile.as_deref_mut()) {
        p.draw_preparation = start.elapsed();
    }
    let rgb = render(
        renderer, &atlases, width, height, aspect, &initial, draws, profile,
    )?;
    Ok(ppm(&rgb, width, height))
}
fn rgb_to_rgba(rgb: &[u8]) -> Vec<u8> {
    rgb.chunks_exact(3)
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn particle_sprite_is_composited_in_the_single_gpu_frame_pass() {
        let corners = [[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]];
        let skin = crate::formats::SkinAssets {
            width: 1,
            height: 1,
            interpolation: false,
            rgba: vec![0; 4],
            sprites: BTreeMap::new(),
        };
        let particles = crate::formats::ParticleAssets {
            width: 1,
            height: 1,
            interpolation: false,
            rgba: vec![255, 32, 0, 128],
            sprites: vec![crate::formats::ParticleSpriteAsset {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            }],
            effects: BTreeMap::new(),
        };
        let draw = crate::particles::ParticleSpriteDraw {
            sprite_id: 0,
            corners,
            color: [128, 255, 255],
            alpha: 0.5,
            order: (0, 0, 0, 0),
        };
        let transform = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ];
        let gpu = GpuRenderer::shared().unwrap();
        let ppm = render_display_list(
            gpu,
            &crate::runtime::DisplayList::default(),
            4,
            4,
            1.0,
            &skin,
            &BTreeMap::new(),
            &transform,
            None,
            Some((&particles, std::slice::from_ref(&draw), &transform)),
            None,
        )
        .unwrap();
        let header_end = ppm
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'\n')
            .nth(2)
            .map(|(index, _)| index + 1)
            .unwrap();
        let gpu_rgb = &ppm[header_end..];
        let mut cpu_rgb = vec![0u8; 4 * 4 * 3];
        crate::runtime::DisplayList::composite_particle_sprites(
            &mut cpu_rgb,
            4,
            4,
            1.0,
            &particles,
            std::slice::from_ref(&draw),
            &transform,
        )
        .unwrap();
        let excess = gpu_rgb
            .iter()
            .zip(cpu_rgb)
            .filter(|(a, b)| a.abs_diff(*b) > 2)
            .count();
        assert!(
            excess == 0,
            "{excess} particle framebuffer channels differ by more than two"
        );
    }
}
