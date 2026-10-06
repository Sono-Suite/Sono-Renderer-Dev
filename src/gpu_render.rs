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
fn triangle_uv(coord: vec2<f32>, ids: vec3<u32>) -> vec2<f32> {
  let a=p.corners[ids.x].xy; let ab=p.corners[ids.y].xy-a; let ac=p.corners[ids.z].xy-a; let ap=coord-a;
  let det=ab.x*ac.y-ab.y*ac.x;
  if abs(det)<1e-12 { return vec2(-1000.0); }
  let beta=(ap.x*ac.y-ap.y*ac.x)/det; let gamma=(ab.x*ap.y-ab.y*ap.x)/det;
  let bary=vec3(1.0-beta-gamma,beta,gamma);
  if min(min(bary.x,bary.y),bary.z)<-1e-7 { return vec2(-1000.0); }
  let w=bary*vec3(p.corners[ids.x].z,p.corners[ids.y].z,p.corners[ids.z].z);
  let sum=w.x+w.y+w.z;
  if abs(sum)<1e-12 { return vec2(-1000.0); }
  var uv=array<vec2<f32>,4>(vec2(0.0,0.0),vec2(0.0,1.0),vec2(1.0,1.0),vec2(1.0,0.0));
  return (w.x*uv[ids.x]+w.y*uv[ids.y]+w.z*uv[ids.z])/sum;
}
fn lightweight_uv(coord: vec2<f32>) -> vec2<f32> {
  let uv=triangle_uv(coord,vec3<u32>(0u,1u,2u));
  if uv.x>-999.0 { return uv; }
  return triangle_uv(coord,vec3<u32>(0u,2u,3u));
}
@fragment fn fs(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
  if p.sampling.z > 0.5 { return vec4(p.tint.rgb, p.frame.w); }
  let coord=vec2((frag.x/p.frame.x*2.0-1.0)*p.frame.z, 1.0-frag.y/p.frame.y*2.0);
  var uv=vec2<f32>(0.0);
  if p.sampling.w > 0.5 { uv=lightweight_uv(coord); } else { uv=invbilinear(coord); }
  if p.sampling.w > 0.5 {
    if uv.x < -999.0 { discard; }
  } else {
    if uv.x < -1e-7 || uv.x > 1.0000001 || uv.y < -1e-7 || uv.y > 1.0000001 { discard; }
  }
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

#[cfg_attr(test, derive(Clone))]
struct DrawOp {
    rect: [u32; 4],
    corners: [[f64; 2]; 4],
    alpha: f64,
    tint: [f64; 4],
    interpolation: bool,
    atlas: usize,
    background: bool,
    mask: bool,
    skin_mode: crate::skin_render_mode::SkinRenderMode,
}

enum FrameBase {
    Clear([u8; 3]),
    #[cfg(test)]
    UploadReference([u8; 3]),
}

#[cfg(test)]
thread_local! {
    static USE_UPLOAD_REFERENCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn with_uploaded_base_reference<T>(operation: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            USE_UPLOAD_REFERENCE.set(self.0);
        }
    }
    let _restore = Restore(USE_UPLOAD_REFERENCE.replace(true));
    operation()
}

// Test-only readback of the real render target, including its alpha channel.
#[cfg(test)]
thread_local! {
    static RGBA_CAPTURE: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
pub(crate) fn capture_rgba<T>(operation: impl FnOnce() -> T) -> (T, Vec<u8>) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            RGBA_CAPTURE.with(|s| *s.borrow_mut() = None);
        }
    }
    RGBA_CAPTURE.with(|s| *s.borrow_mut() = Some(Vec::new()));
    let _reset = Reset;
    let result = operation();
    let rgba = RGBA_CAPTURE.with(|s| s.borrow_mut().take().unwrap());
    (result, rgba)
}

fn base_clear_color(rgb: [u8; 3]) -> wgpu::Color {
    // The old RGB upload added alpha 255. Keep that opaque destination for
    // blending, including pixels not covered by any draw. Rgba8Unorm is linear;
    // tests verify all 256 byte values against the previous upload path.
    wgpu::Color {
        r: f64::from(rgb[0]) / 255.0,
        g: f64::from(rgb[1]) / 255.0,
        b: f64::from(rgb[2]) / 255.0,
        a: 1.0,
    }
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

pub(crate) type AtlasIdentity = [u8; 20];

/// Content identity is computed when immutable session assets are loaded, not
/// once per rendered frame. Dimensions remain part of the key so differently
/// shaped images cannot reuse an incompatible GPU texture.
pub(crate) fn atlas_identity(rgba: &[u8], width: u32, height: u32) -> AtlasIdentity {
    use sha1::{Digest, Sha1};
    let mut digest = Sha1::new();
    digest.update(width.to_le_bytes());
    digest.update(height.to_le_bytes());
    digest.update(rgba);
    digest.finalize().into()
}

#[derive(Default)]
pub(crate) struct GpuFrameProfile {
    pub draw_preparation: std::time::Duration,
    pub render_target_setup: std::time::Duration,
    pub initial_rgba_conversion: std::time::Duration,
    pub initial_framebuffer_upload: std::time::Duration,
    pub atlas_hashing: std::time::Duration,
    pub atlas_cache_upload: std::time::Duration,
    pub readback_buffer_setup: std::time::Duration,
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

    fn atlas_texture(
        &self,
        rgba: &[u8],
        width: u32,
        height: u32,
        identity: AtlasIdentity,
    ) -> Result<(Arc<wgpu::Texture>, bool)> {
        let mut cache = self
            .atlases
            .lock()
            .map_err(|_| anyhow::anyhow!("wgpu atlas cache lock was poisoned"))?;
        if let Some(texture) = cache.get(&identity) {
            return Ok((texture.clone(), false));
        }
        if cache.len() >= 8 {
            cache.clear();
        }
        let texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("resident source atlas"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
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
        let texture = Arc::new(texture);
        cache.insert(identity, texture.clone());
        Ok((texture, true))
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
                    has_dynamic_offset: true,
                    min_binding_size: std::num::NonZeroU64::new(
                        std::mem::size_of::<Params>() as u64
                    ),
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
    atlases: &[(&[u8], u32, u32, AtlasIdentity)],
    width: u32,
    height: u32,
    aspect: f64,
    base: FrameBase,
    draws: Vec<DrawOp>,
    mut profile: Option<&mut GpuFrameProfile>,
) -> Result<Vec<u8>> {
    let gpu = &renderer.gpu;
    let encode_start;
    let target_setup_start = profile.as_ref().map(|_| std::time::Instant::now());
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
    if let (Some(start), Some(p)) = (target_setup_start, profile.as_deref_mut()) {
        p.render_target_setup = start.elapsed();
    }
    let load = match base {
        FrameBase::Clear(rgb) => wgpu::LoadOp::Clear(base_clear_color(rgb)),
        #[cfg(test)]
        FrameBase::UploadReference(rgb) => {
            upload_reference_base(gpu, &tex, width, height, rgb);
            wgpu::LoadOp::Load
        }
    };
    let mut resident = Vec::with_capacity(atlases.len());
    for (input, iw, ih, identity) in atlases {
        let atlas_cache_start = profile.as_ref().map(|_| std::time::Instant::now());
        let (atlas, uploaded) = renderer.atlas_texture(input, *iw, *ih, *identity)?;
        if uploaded {
            if let Some(p) = profile.as_deref_mut() {
                p.atlas_uploads += 1;
            }
        }
        resident.push(atlas.create_view(&Default::default()));
        if let (Some(start), Some(p)) = (atlas_cache_start, profile.as_deref_mut()) {
            p.atlas_cache_upload += start.elapsed();
        }
    }
    let view = tex.create_view(&Default::default());
    let stride = width.checked_mul(4).context("frame row overflow")?;
    let padded =
        stride.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let readback_setup_start = profile.as_ref().map(|_| std::time::Instant::now());
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("framebuffer readback"),
        size: u64::from(padded) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    if let (Some(start), Some(p)) = (readback_setup_start, profile.as_deref_mut()) {
        p.readback_buffer_setup = start.elapsed();
    }
    encode_start = profile.as_ref().map(|_| std::time::Instant::now());
    let parameter_alignment = u64::from(gpu.device.limits().min_uniform_buffer_offset_alignment);
    let parameter_size = std::mem::size_of::<Params>() as u64;
    let parameter_stride = parameter_size.div_ceil(parameter_alignment) * parameter_alignment;
    let parameter_buffer_size = parameter_stride
        .checked_mul(draws.len() as u64)
        .context("draw parameter buffer size overflow")?
        .max(parameter_size);
    let parameter_buffer_len = usize::try_from(parameter_buffer_size)
        .context("draw parameter buffer does not fit host address space")?;
    let mut parameter_bytes = vec![0; parameter_buffer_len];
    let mut draw_bindings = Vec::with_capacity(draws.len());
    for (draw_index, draw) in draws.iter().enumerate() {
        let [x, y, w, h] = draw.rect;
        let params = Params {
            corners: {
                let weights =
                    if draw.skin_mode == crate::skin_render_mode::SkinRenderMode::Lightweight {
                        crate::skin_render_mode::projective_weights(&draw.corners)
                    } else {
                        [0.0; 4]
                    };
                std::array::from_fn(|i| {
                    [
                        draw.corners[i][0] as f32,
                        draw.corners[i][1] as f32,
                        weights[i] as f32,
                        0.0,
                    ]
                })
            },
            atlas: [x as f32, y as f32, w as f32, h as f32],
            frame: [
                width as f32,
                height as f32,
                aspect as f32,
                draw.alpha as f32,
            ],
            tint: draw.tint.map(|v| v as f32),
            sampling: [
                if draw.interpolation || draw.background {
                    1.0
                } else {
                    0.0
                },
                if draw.background { 1.0 } else { 0.0 },
                if draw.mask { 1.0 } else { 0.0 },
                if draw.skin_mode == crate::skin_render_mode::SkinRenderMode::Lightweight {
                    1.0
                } else {
                    0.0
                },
            ],
        };
        let byte_offset = draw_index * parameter_stride as usize;
        parameter_bytes[byte_offset..byte_offset + std::mem::size_of::<Params>()]
            .copy_from_slice(bytemuck::bytes_of(&params));
        let dynamic_offset = u32::try_from(draw_index as u64 * parameter_stride)
            .context("draw parameter dynamic offset exceeds wgpu's u32 limit")?;
        draw_bindings.push((draw.atlas, dynamic_offset));
    }
    let parameter_buffer = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ordered draw parameter array"),
            contents: &parameter_bytes,
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let atlas_bindings = resident
        .iter()
        .map(|view| {
            gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ordered sprite atlas binding"),
                layout: &gpu.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &parameter_buffer,
                            offset: 0,
                            size: std::num::NonZeroU64::new(parameter_size),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                ],
            })
        })
        .collect::<Vec<_>>();
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
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&gpu.pipeline);
        for (atlas, dynamic_offset) in draw_bindings {
            pass.set_bind_group(0, &atlas_bindings[atlas], &[dynamic_offset]);
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
            #[cfg(test)]
            RGBA_CAPTURE.with(|s| {
                if let Some(rgba) = s.borrow_mut().as_mut() {
                    rgba.extend_from_slice(pixel);
                }
            });
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

#[cfg(test)]
pub(crate) fn render_display_list_rgb(
    renderer: &GpuRenderer,
    list: &crate::runtime::DisplayList,
    width: u32,
    height: u32,
    aspect: f64,
    skin: &crate::formats::SkinAssets,
    skin_identity: AtlasIdentity,
    bindings: &std::collections::BTreeMap<u32, String>,
    runtime: &[f64; 16],
    background: Option<(
        &crate::formats::BackgroundAssets,
        [[f64; 2]; 4],
        AtlasIdentity,
    )>,
    particles: Option<(
        &crate::formats::ParticleAssets,
        &[crate::particles::ParticleSpriteDraw],
        &[f64; 16],
        AtlasIdentity,
    )>,
    profile: Option<&mut GpuFrameProfile>,
) -> Result<Vec<u8>> {
    render_display_list_rgb_with_skin_mode(
        renderer,
        list,
        width,
        height,
        aspect,
        skin,
        skin_identity,
        bindings,
        runtime,
        background,
        particles,
        profile,
        crate::skin_render_mode::SkinRenderMode::Standard,
    )
}

pub(crate) fn render_display_list_rgb_with_skin_mode(
    renderer: &GpuRenderer,
    list: &crate::runtime::DisplayList,
    width: u32,
    height: u32,
    aspect: f64,
    skin: &crate::formats::SkinAssets,
    skin_identity: AtlasIdentity,
    bindings: &std::collections::BTreeMap<u32, String>,
    runtime: &[f64; 16],
    background: Option<(
        &crate::formats::BackgroundAssets,
        [[f64; 2]; 4],
        AtlasIdentity,
    )>,
    particles: Option<(
        &crate::formats::ParticleAssets,
        &[crate::particles::ParticleSpriteDraw],
        &[f64; 16],
        AtlasIdentity,
    )>,
    mut profile: Option<&mut GpuFrameProfile>,
    skin_mode: crate::skin_render_mode::SkinRenderMode,
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
    let mut atlases = vec![(skin.rgba.as_slice(), skin.width, skin.height, skin_identity)];
    let mut base = [0u8; 3];
    if let Some((bg, quad, bg_identity)) = background {
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
        let color = crate::formats::parse_background_color(&bg.data.color, false)?;
        let mask = crate::formats::parse_background_color(&bg.configuration.mask, true)?;
        base.copy_from_slice(&color[..3]);
        let bg_atlas = atlases.len();
        atlases.push((bg.rgba.as_slice(), bg.width, bg.height, bg_identity));
        draws.push(DrawOp {
            rect: [0, 0, bg.width, bg.height],
            corners: quad,
            alpha: 1.0,
            tint: [1.0; 4],
            interpolation: true,
            atlas: bg_atlas,
            background: true,
            mask: false,
            skin_mode: crate::skin_render_mode::SkinRenderMode::Standard,
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
            skin_mode: crate::skin_render_mode::SkinRenderMode::Standard,
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
            skin_mode,
        });
    }
    if let Some((assets, particle_draws, particle_runtime, particle_identity)) = particles {
        let particle_atlas = atlases.len();
        if assets.width == 0
            || assets.height == 0
            || assets.rgba.len() != assets.width as usize * assets.height as usize * 4
        {
            bail!("particle atlas dimensions do not match its RGBA pixel buffer");
        }
        atlases.push((
            assets.rgba.as_slice(),
            assets.width,
            assets.height,
            particle_identity,
        ));
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
            draws.push(DrawOp {
                rect: [s.x, s.y, s.width, s.height],
                corners: crate::runtime::gpu_transform_particle_corners(d, particle_runtime),
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
                skin_mode: crate::skin_render_mode::SkinRenderMode::Standard,
            });
        }
    }
    if let (Some(start), Some(p)) = (preparation_start, profile.as_deref_mut()) {
        p.draw_preparation = start.elapsed();
    }
    let base = FrameBase::Clear(base);
    #[cfg(test)]
    let base = match base {
        FrameBase::Clear(rgb) if USE_UPLOAD_REFERENCE.get() => FrameBase::UploadReference(rgb),
        base => base,
    };
    render(
        renderer,
        &atlases,
        width,
        height,
        aspect,
        base,
        draws,
        profile.as_deref_mut(),
    )
}
#[cfg(test)]
fn upload_reference_base(gpu: &Gpu, tex: &wgpu::Texture, width: u32, height: u32, base: [u8; 3]) {
    // Keep the former CPU RGB -> RGBA -> upload initialization only in tests.
    let mut rgb = vec![0u8; width as usize * height as usize * 3];
    for pixel in rgb.chunks_exact_mut(3) {
        pixel.copy_from_slice(&base);
    }
    let rgba: Vec<u8> = rgb
        .chunks_exact(3)
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect();
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &rgba,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn assert_exact_rgb(actual: &[u8], expected: &[u8], width: usize, context: &str) {
        assert_eq!(actual.len(), expected.len());
        if let Some(index) = actual.iter().zip(expected).position(|(a, b)| a != b) {
            panic!(
                "{context}: first difference at pixel ({}, {}), channel {}: {} != {}",
                index / 3 % width,
                index / 3 / width,
                index % 3,
                actual[index],
                expected[index]
            );
        }
    }

    #[test]
    fn clear_base_matches_uploaded_base_for_every_channel_byte() {
        let gpu = GpuRenderer::new().unwrap();
        // Each channel visits every possible byte. Width 3 also exercises padded
        // readback rows. Alternating colors checks fresh-frame independence.
        for value in 0..=255u8 {
            let base = [value, 255 - value, value.rotate_left(1)];
            let reference = render(
                &gpu,
                &[],
                3,
                2,
                1.5,
                FrameBase::UploadReference(base),
                Vec::new(),
                None,
            )
            .unwrap();
            let mut profile = GpuFrameProfile::default();
            let cleared = render(
                &gpu,
                &[],
                3,
                2,
                1.5,
                FrameBase::Clear(base),
                Vec::new(),
                Some(&mut profile),
            )
            .unwrap();
            let expected = base.repeat(6);
            assert_exact_rgb(&reference, &expected, 3, &format!("uploaded base {base:?}"));
            assert_exact_rgb(&cleared, &reference, 3, &format!("cleared base {base:?}"));
            assert_eq!(profile.initial_rgba_conversion, std::time::Duration::ZERO);
            assert_eq!(
                profile.initial_framebuffer_upload,
                std::time::Duration::ZERO
            );
        }
    }

    #[test]
    fn clear_base_preserves_partial_coverage_and_ordered_blending() {
        let gpu = GpuRenderer::new().unwrap();
        let pixels = [219, 57, 103, 128];
        let atlases = [(pixels.as_slice(), 1, 1, atlas_identity(&pixels, 1, 1))];
        let partial = [[-0.5, -0.5], [-0.5, 0.5], [0.5, 0.5], [0.5, -0.5]];
        let sprite = DrawOp {
            rect: [0, 0, 1, 1],
            corners: partial,
            alpha: 0.5,
            tint: [1.0; 4],
            interpolation: false,
            atlas: 0,
            background: false,
            mask: false,
            skin_mode: crate::skin_render_mode::SkinRenderMode::Standard,
        };
        // Background sampling, full-screen mask, skin and tinted particle paths
        // all blend in the same pass. Compare with the former initialization,
        // keeping every draw parameter and its order identical.
        let draws = vec![
            DrawOp {
                background: true,
                interpolation: true,
                ..sprite.clone()
            },
            DrawOp {
                corners: [[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]],
                alpha: 119.0 / 255.0,
                tint: [0.2, 0.4, 0.6, 1.0],
                mask: true,
                ..sprite.clone()
            },
            sprite.clone(),
            DrawOp {
                tint: [0.3, 0.7, 0.9, 1.0],
                ..sprite
            },
        ];
        for value in 0..=255u8 {
            let base = [value, 255 - value, value.rotate_left(1)];
            // Also test genuinely untouched pixels without the full-screen mask.
            for with_mask in [false, true] {
                let ordered: Vec<_> = draws
                    .iter()
                    .filter(|d| with_mask || !d.mask)
                    .cloned()
                    .collect();
                let reference = render(
                    &gpu,
                    &atlases,
                    8,
                    8,
                    1.0,
                    FrameBase::UploadReference(base),
                    ordered.clone(),
                    None,
                )
                .unwrap();
                let cleared = render(
                    &gpu,
                    &atlases,
                    8,
                    8,
                    1.0,
                    FrameBase::Clear(base),
                    ordered,
                    None,
                )
                .unwrap();
                assert_exact_rgb(
                    &cleared,
                    &reference,
                    8,
                    &format!("base {base:?}, mask={with_mask}"),
                );
                if !with_mask {
                    assert_eq!(&cleared[..3], &base);
                    assert_ne!(&cleared[(4 * 8 + 4) * 3..(4 * 8 + 4) * 3 + 3], &base);
                }
            }
        }
    }

    #[test]
    fn clear_base_background_mask_and_skin_match_exact_cpu_pixels() {
        let gpu = GpuRenderer::new().unwrap();
        let transform = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let mut sprite_transform = [[0.0; 8]; 8];
        for (i, row) in sprite_transform.iter_mut().enumerate() {
            row[i] = 1.0;
        }
        let skin = crate::formats::SkinAssets {
            width: 1,
            height: 1,
            interpolation: false,
            rgba: vec![19, 83, 227, 255],
            sprites: [(
                "test".into(),
                crate::formats::SkinSpriteAsset {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                    transform: sprite_transform,
                },
            )]
            .into(),
        };
        let bindings = [(0, "test".into())].into();
        let list = crate::runtime::DisplayList {
            sprites: vec![crate::runtime::SpriteDraw {
                sprite_id: 0,
                corners: [[-0.5, -0.5], [-0.5, 0.5], [0.5, 0.5], [0.5, -0.5]],
                z: [0.0; 4],
                alpha: 1.0,
                provenance: None,
                trace: None,
            }],
        };
        for color in ["#123456", "#abc", "#fe0180"] {
            for mask in ["#0000", "#13579bff"] {
                let background = crate::formats::BackgroundAssets {
                    width: 1,
                    height: 1,
                    rgba: vec![79, 137, 201, 0],
                    data: crate::formats::BackgroundData {
                        aspect_ratio: None,
                        fit: "contain".into(),
                        color: color.into(),
                        scale_x: None,
                        scale_y: None,
                    },
                    configuration: crate::formats::BackgroundConfiguration {
                        blur: 0.0,
                        mask: mask.into(),
                    },
                };
                let quad = [[-0.5, -0.5], [-0.5, 0.5], [0.5, 0.5], [0.5, -0.5]];
                // These CPU cases use transparent background pixels, opaque
                // masks and opaque skin, for which exact agreement is valid.
                for background in [None, Some((&background, quad))] {
                    for display in [&crate::runtime::DisplayList::default(), &list] {
                        let cpu = display
                            .render_skin_ppm_with_runtime_transform_and_background(
                                8, 8, 1.0, &skin, &bindings, &transform, background,
                            )
                            .unwrap();
                        let actual = render_display_list_rgb(
                            &gpu,
                            display,
                            8,
                            8,
                            1.0,
                            &skin,
                            atlas_identity(&skin.rgba, 1, 1),
                            &bindings,
                            &transform,
                            background.map(|(bg, quad)| (bg, quad, atlas_identity(&bg.rgba, 1, 1))),
                            None,
                            None,
                        )
                        .unwrap();
                        assert_exact_rgb(
                            &actual,
                            &crate::offline::ppm_rgb_payload(&cpu, 8, 8).unwrap(),
                            8,
                            &format!("background {color}, mask {mask}"),
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn atlas_identity_is_stable_and_changes_with_pixels_or_dimensions() {
        let pixels: Vec<u8> = (0..24).collect();
        let original = atlas_identity(&pixels, 3, 2);
        assert_eq!(original, atlas_identity(&pixels, 3, 2));

        let mut changed_pixels = pixels.clone();
        changed_pixels[17] ^= 0x80;
        assert_ne!(original, atlas_identity(&changed_pixels, 3, 2));
        assert_ne!(original, atlas_identity(&pixels, 2, 3));
    }

    #[test]
    fn atlas_cache_reuses_identity_and_invalidates_changed_resources() {
        let gpu = GpuRenderer::shared().unwrap();
        let pixels: Vec<u8> = (32..56).collect();
        let identity = atlas_identity(&pixels, 3, 2);
        let (first, uploaded) = gpu.atlas_texture(&pixels, 3, 2, identity).unwrap();
        assert!(uploaded);

        let (cached, uploaded) = gpu.atlas_texture(&pixels, 3, 2, identity).unwrap();
        assert!(!uploaded);
        assert!(Arc::ptr_eq(&first, &cached));

        let mut changed_pixels = pixels.clone();
        changed_pixels[9] ^= 0x40;
        let changed_identity = atlas_identity(&changed_pixels, 3, 2);
        let (changed, uploaded) = gpu
            .atlas_texture(&changed_pixels, 3, 2, changed_identity)
            .unwrap();
        assert!(uploaded);
        assert!(!Arc::ptr_eq(&first, &changed));

        let reshaped_identity = atlas_identity(&pixels, 2, 3);
        let (reshaped, uploaded) = gpu.atlas_texture(&pixels, 2, 3, reshaped_identity).unwrap();
        assert!(uploaded);
        assert!(!Arc::ptr_eq(&first, &reshaped));
    }

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
        let gpu_rgb = render_display_list_rgb(
            gpu,
            &crate::runtime::DisplayList::default(),
            4,
            4,
            1.0,
            &skin,
            atlas_identity(&skin.rgba, skin.width, skin.height),
            &BTreeMap::new(),
            &transform,
            None,
            Some((
                &particles,
                std::slice::from_ref(&draw),
                &transform,
                atlas_identity(&particles.rgba, particles.width, particles.height),
            )),
            None,
        )
        .unwrap();
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
