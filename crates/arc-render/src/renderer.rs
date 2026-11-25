//! The wgpu direct compositor (docs/03 §1–4, §9–10).
//!
//! For each frame: prepare every source (sRGB → linear premultiplied RGBA16F), run its effect
//! chain, draw it with its transform into its track layer (over, or additive inside a cross
//! dissolve), composite layers bottom → top, resolve to RGBA8 sRGB, read back.

use crate::plan::ItemPlan;
use arc_core::model::Transform;
use std::collections::{BTreeMap, HashMap};

const WORK: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const OUT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("no GPU adapter available: {0}")]
    NoAdapter(String),
    #[error("GPU device request failed: {0}")]
    Device(String),
    #[error("GPU readback failed: {0}")]
    Readback(String),
    #[error("source image has {got} bytes, expected {expected}")]
    BadImage { got: usize, expected: usize },
}

/// A decoded source image (RGBA8, sRGB, straight alpha).
#[derive(Debug, Clone, Copy)]
pub struct Image<'a> {
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
}

/// One item to draw: a source image plus the planned transform, opacity and effects.
#[derive(Debug, Clone)]
pub struct DrawItem<'a> {
    pub image: Image<'a>,
    pub transform: Transform,
    pub opacity: f64,
    pub additive: bool,
    pub effects: &'a [crate::plan::EffectPlan],
}

impl<'a> DrawItem<'a> {
    pub fn from_plan(plan: &'a ItemPlan, image: Image<'a>) -> Self {
        DrawItem {
            image,
            transform: plan.transform,
            opacity: plan.opacity,
            additive: plan.additive,
            effects: &plan.effects,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DrawLayer<'a> {
    pub opacity: f64,
    pub items: Vec<DrawItem<'a>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameReport {
    /// Effects present in the plan that this renderer does not implement yet (passed through).
    pub unsupported_effects: Vec<String>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
    size: [f32; 4],
}

struct Pipelines {
    quad_over: wgpu::RenderPipeline,
    quad_add: wgpu::RenderPipeline,
    prepare: wgpu::RenderPipeline,
    resolve: wgpu::RenderPipeline,
    effects: HashMap<&'static str, wgpu::RenderPipeline>,
}

/// GPU renderer. Owns its device (headless) or shares one (the GUI's egui-wgpu device).
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pipes: Pipelines,
    adapter_info: String,
    pool: Vec<(u32, u32, wgpu::TextureFormat, wgpu::Texture)>,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renderer")
            .field("adapter", &self.adapter_info)
            .finish()
    }
}

// Private helper: each argument is one field of the pipeline descriptor.
#[allow(clippy::too_many_arguments)]
fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    vs: &str,
    fs: &str,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
    strip: bool,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(fs),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(vs),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            topology: if strip {
                wgpu::PrimitiveTopology::TriangleStrip
            } else {
                wgpu::PrimitiveTopology::TriangleList
            },
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fs),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

const PREMUL_OVER: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
};
const ADDITIVE: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
};

/// Effects implemented on the GPU, by manifest id → fragment entry point.
const EFFECT_SHADERS: [(&str, &str); 5] = [
    ("core.color.correct", "fs_color_correct"),
    ("core.stylize.vignette", "fs_vignette"),
    ("core.key.chroma", "fs_chroma"),
    ("core.stylize.sharpen", "fs_sharpen"),
    ("core.blur.gaussian", "fs_blur"),
];

impl Renderer {
    /// Create a renderer with its own device (CLI, export, tests).
    /// `ARCLINE_FORCE_FALLBACK_ADAPTER=1` selects the software adapter (CI determinism runs).
    pub fn new_headless() -> Result<Renderer, RenderError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let force_fallback =
            std::env::var("ARCLINE_FORCE_FALLBACK_ADAPTER").is_ok_and(|v| v == "1");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: force_fallback,
            compatible_surface: None,
            ..Default::default()
        }))
        .map_err(|e| RenderError::NoAdapter(e.to_string()))?;
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("arc-render"),
            ..Default::default()
        }))
        .map_err(|e| RenderError::Device(e.to_string()))?;
        Ok(Self::with_device(
            device,
            queue,
            format!("{} ({:?})", info.name, info.backend),
        ))
    }

    /// Share an existing device (the GUI passes egui-wgpu's device so the viewer needs no copies).
    pub fn with_device(device: wgpu::Device, queue: wgpu::Queue, adapter_info: String) -> Renderer {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("compositor"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/compositor.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("compositor"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("compositor"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let mut effects = HashMap::new();
        for (id, fs) in EFFECT_SHADERS {
            effects.insert(
                id,
                pipeline(&device, &pl, &module, "vs_full", fs, WORK, None, false),
            );
        }
        let pipes = Pipelines {
            quad_over: pipeline(
                &device,
                &pl,
                &module,
                "vs_quad",
                "fs_quad",
                WORK,
                Some(PREMUL_OVER),
                true,
            ),
            quad_add: pipeline(
                &device,
                &pl,
                &module,
                "vs_quad",
                "fs_quad",
                WORK,
                Some(ADDITIVE),
                true,
            ),
            prepare: pipeline(
                &device,
                &pl,
                &module,
                "vs_full",
                "fs_prepare",
                WORK,
                None,
                false,
            ),
            resolve: pipeline(
                &device,
                &pl,
                &module,
                "vs_full",
                "fs_resolve",
                OUT,
                None,
                false,
            ),
            effects,
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear-clamp"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Renderer {
            device,
            queue,
            layout,
            sampler,
            pipes,
            adapter_info,
            pool: Vec::new(),
        }
    }

    pub fn adapter_info(&self) -> &str {
        &self.adapter_info
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Effect ids with a GPU implementation.
    pub fn supported_effects() -> Vec<&'static str> {
        EFFECT_SHADERS.iter().map(|(id, _)| *id).collect()
    }

    fn texture(&mut self, w: u32, h: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
        if let Some(i) = self
            .pool
            .iter()
            .position(|(pw, ph, pf, _)| *pw == w && *ph == h && *pf == format)
        {
            return self.pool.swap_remove(i).3;
        }
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    fn recycle(&mut self, t: wgpu::Texture) {
        let s = t.size();
        self.pool.push((s.width, s.height, t.format(), t));
        if self.pool.len() > 32 {
            self.pool.remove(0);
        }
    }

    fn bind(&self, src: &wgpu::TextureView, p: Params) -> wgpu::BindGroup {
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buf, 0, bytemuck::bytes_of(&p));
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buf.as_entire_binding(),
                },
            ],
        })
    }

    fn pass(
        enc: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clear: Option<wgpu::Color>,
        pipe: &wgpu::RenderPipeline,
        bg: &wgpu::BindGroup,
        vertices: u32,
    ) {
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: match clear {
                        Some(c) => wgpu::LoadOp::Clear(c),
                        None => wgpu::LoadOp::Load,
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(pipe);
        rp.set_bind_group(0, bg, &[]);
        rp.draw(0..vertices, 0..1);
    }

    fn clear(enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, c: wgpu::Color) {
        let _ = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(c),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }

    fn effect_params(e: &crate::plan::EffectPlan, w: u32, h: u32) -> Vec<Params> {
        let g = |k: &str, d: f64| e.params.get(k).copied().unwrap_or(d) as f32;
        let size = [1.0 / w as f32, 1.0 / h as f32, w as f32, h as f32];
        let p = |a: [f32; 4], b: [f32; 4]| Params {
            a,
            b,
            c: [0.0; 4],
            size,
        };
        match e.effect.as_str() {
            "core.color.correct" => vec![p(
                [
                    g("exposure", 0.0),
                    g("contrast", 1.0),
                    g("saturation", 1.0),
                    g("temperature", 0.0),
                ],
                [0.0; 4],
            )],
            "core.stylize.vignette" => vec![p(
                [
                    g("amount", 0.5),
                    g("midpoint", 0.5),
                    g("roundness", 1.0),
                    w as f32 / h as f32,
                ],
                [0.0; 4],
            )],
            "core.key.chroma" => vec![p(
                [
                    g("key_r", 0.0),
                    g("key_g", 1.0),
                    g("key_b", 0.0),
                    g("tolerance", 0.3),
                ],
                [g("softness", 0.1), g("spill", 0.5), 0.0, 0.0],
            )],
            "core.stylize.sharpen" => {
                vec![p([g("amount", 0.5), g("radius", 1.0), 0.0, 0.0], [0.0; 4])]
            }
            "core.blur.gaussian" => {
                let r = g("radius", 8.0);
                vec![
                    p([r, 1.0, 0.0, 0.0], [0.0; 4]),
                    p([r, 0.0, 1.0, 0.0], [0.0; 4]),
                ]
            }
            _ => Vec::new(),
        }
    }

    /// Affine mapping of a source of `sw×sh` into a `w×h` frame: fit, then the clip transform.
    fn quad_params(t: &Transform, sw: u32, sh: u32, w: u32, h: u32, opacity: f64) -> Params {
        let (w, h, sw, sh) = (w as f64, h as f64, sw as f64, sh as f64);
        let fit = (w / sw).min(h / sh);
        let (qw, qh) = (sw * fit * t.scale.x, sh * fit * t.scale.y);
        let (cx, cy) = (w / 2.0 + t.position.x * w, h / 2.0 + t.position.y * h);
        let (s, c) = t.rotation_deg.to_radians().sin_cos();
        // pixel(u, v) = center + R · ((u − ax)·qw, (v − ay)·qh)
        let px = |u: f64, v: f64| {
            let (lx, ly) = ((u - t.anchor.x) * qw, (v - t.anchor.y) * qh);
            (cx + c * lx - s * ly, cy + s * lx + c * ly)
        };
        let ndc = |(x, y): (f64, f64)| (x / w * 2.0 - 1.0, 1.0 - y / h * 2.0);
        let o = ndc(px(0.0, 0.0));
        let ux = ndc(px(1.0, 0.0));
        let vy = ndc(px(0.0, 1.0));
        Params {
            a: [
                (ux.0 - o.0) as f32,
                (ux.1 - o.1) as f32,
                (vy.0 - o.0) as f32,
                (vy.1 - o.1) as f32,
            ],
            b: [o.0 as f32, o.1 as f32, 0.0, 0.0],
            c: [opacity as f32, 0.0, 0.0, 0.0],
            size: [0.0; 4],
        }
    }

    /// Render one frame to RGBA8 sRGB (`w×h×4` bytes, tightly packed).
    pub fn render(
        &mut self,
        w: u32,
        h: u32,
        background: [f32; 4],
        layers: &[DrawLayer<'_>],
    ) -> Result<(Vec<u8>, FrameReport), RenderError> {
        let mut report = FrameReport::default();
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        let target = self.texture(w, h, WORK);
        let target_view = target.create_view(&Default::default());
        let [r, g, b, a] = background.map(f64::from);
        Self::clear(
            &mut enc,
            &target_view,
            wgpu::Color {
                r: r * a,
                g: g * a,
                b: b * a,
                a,
            },
        );
        let mut retired = Vec::new();
        let full = Params {
            a: [2.0, 0.0, 0.0, -2.0],
            b: [-1.0, 1.0, 0.0, 0.0],
            c: [1.0, 0.0, 0.0, 0.0],
            size: [0.0; 4],
        };

        for layer in layers {
            let layer_tex = self.texture(w, h, WORK);
            let layer_view = layer_tex.create_view(&Default::default());
            Self::clear(&mut enc, &layer_view, wgpu::Color::TRANSPARENT);
            for item in &layer.items {
                let (sw, sh) = (item.image.width, item.image.height);
                let expected = sw as usize * sh as usize * 4;
                if item.image.rgba.len() != expected || sw == 0 || sh == 0 {
                    return Err(RenderError::BadImage {
                        got: item.image.rgba.len(),
                        expected,
                    });
                }
                let src = self.texture(sw, sh, wgpu::TextureFormat::Rgba8UnormSrgb);
                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &src,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    item.image.rgba,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(sw * 4),
                        rows_per_image: Some(sh),
                    },
                    wgpu::Extent3d {
                        width: sw,
                        height: sh,
                        depth_or_array_layers: 1,
                    },
                );
                let mut cur = self.texture(sw, sh, WORK);
                let bg = self.bind(&src.create_view(&Default::default()), full);
                Self::pass(
                    &mut enc,
                    &cur.create_view(&Default::default()),
                    Some(wgpu::Color::TRANSPARENT),
                    &self.pipes.prepare,
                    &bg,
                    3,
                );
                retired.push(src);
                for e in item.effects {
                    let passes = Self::effect_params(e, sw, sh);
                    let Some(pipe) = self.pipes.effects.get(e.effect.as_str()) else {
                        if !report.unsupported_effects.contains(&e.effect) {
                            report.unsupported_effects.push(e.effect.clone());
                        }
                        continue;
                    };
                    let pipe = pipe.clone();
                    for p in passes {
                        let next = self.texture(sw, sh, WORK);
                        let bg = self.bind(&cur.create_view(&Default::default()), p);
                        Self::pass(
                            &mut enc,
                            &next.create_view(&Default::default()),
                            Some(wgpu::Color::TRANSPARENT),
                            &pipe,
                            &bg,
                            3,
                        );
                        retired.push(std::mem::replace(&mut cur, next));
                    }
                }
                let qp = Self::quad_params(&item.transform, sw, sh, w, h, item.opacity);
                let bg = self.bind(&cur.create_view(&Default::default()), qp);
                let pipe = if item.additive {
                    &self.pipes.quad_add
                } else {
                    &self.pipes.quad_over
                };
                Self::pass(&mut enc, &layer_view, None, pipe, &bg, 4);
                retired.push(cur);
            }
            let mut lp = full;
            lp.c[0] = layer.opacity as f32;
            let bg = self.bind(&layer_view, lp);
            Self::pass(&mut enc, &target_view, None, &self.pipes.quad_over, &bg, 4);
            retired.push(layer_tex);
        }

        let out = self.texture(w, h, OUT);
        let bg = self.bind(&target_view, full);
        Self::pass(
            &mut enc,
            &out.create_view(&Default::default()),
            Some(wgpu::Color::BLACK),
            &self.pipes.resolve,
            &bg,
            3,
        );
        let padded = (w * 4).div_ceil(256) * 256;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: padded as u64 * h as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &out,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([enc.finish()]);
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| RenderError::Readback(e.to_string()))?;
        rx.recv()
            .map_err(|e| RenderError::Readback(e.to_string()))?
            .map_err(|e| RenderError::Readback(e.to_string()))?;
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        {
            let data = slice
                .get_mapped_range()
                .map_err(|e| RenderError::Readback(format!("{e:?}")))?;
            for row in 0..h as usize {
                let start = row * padded as usize;
                pixels.extend_from_slice(&data[start..start + w as usize * 4]);
            }
        }
        readback.unmap();
        for t in retired {
            self.recycle(t);
        }
        self.recycle(target);
        self.recycle(out);
        Ok((pixels, report))
    }
}

/// CPU reference for `core.color.correct` on one sRGB pixel (docs/09 §1: every effect has one).
/// Used by golden tests to bound GPU results.
pub fn color_correct_reference(rgb: [u8; 3], params: &BTreeMap<String, f64>) -> [u8; 3] {
    let g = |k: &str, d: f64| params.get(k).copied().unwrap_or(d);
    let lin = |c: u8| {
        let c = c as f64 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let enc = |c: f64| {
        let c = c.clamp(0.0, 1.0);
        let v = if c <= 0.003_130_8 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        };
        (v * 255.0).round() as u8
    };
    let mut c = rgb.map(lin).map(|x| x * 2f64.powf(g("exposure", 0.0)));
    let k = g("temperature", 0.0) / 100.0 * 0.1;
    c[0] *= 1.0 + k;
    c[2] *= 1.0 - k;
    let mid = 0.18;
    c = c.map(|x| mid * (x.max(0.0) / mid).powf(g("contrast", 1.0)));
    let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let s = g("saturation", 1.0);
    c.map(|x| enc((l + (x - l) * s).max(0.0)))
}
