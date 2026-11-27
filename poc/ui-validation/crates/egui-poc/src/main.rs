//! egui + wgpu timeline POC.
//!
//! Draws the shared scene (toolbar, ruler, six lanes, clips, waveforms, keyframes, playhead) and,
//! optionally, a moving video frame either through the current Arcline path (GPU -> CPU readback ->
//! `load_texture` -> upload) or through a zero-copy paint callback that samples the compositor's own
//! texture on the same device.
//!
//! ```text
//! egui-poc --clips 1000 --frames 240 --video-mode readback --out egui.json
//! ```

use egui::{Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, TextureId, Vec2};
use egui_wgpu::CallbackTrait;
use scene::Metrics;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

const LANE_LEFT: f32 = 112.0;
const TOOLBAR_H: f32 = 34.0;
const RULER_H: f32 = 22.0;
const VIDEO_W: u32 = 1920;
const VIDEO_H: u32 = 1080;

#[derive(Clone, Copy, PartialEq, Debug)]
enum VideoMode {
    None,
    Readback,
    Native,
}

struct Args {
    clips: usize,
    frames: usize,
    video: VideoMode,
    zoom: f32,
    width: u32,
    height: u32,
    out: String,
    interact: u8,
    idle_seconds: u64,
    run_seconds: u64,
    startup_marker: Option<String>,
    vsync: bool,
}

/// The compositor's output, on the window's device: a moving gradient standing in for a decoded,
/// composited frame.
struct FrameTex {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
    uniform: wgpu::Buffer,
}

const FRAME_SHADER: &str = r#"
struct U { phase: f32, w: f32, h: f32, _pad: f32 };
@group(0) @binding(0) var<uniform> u: U;
struct VsOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
  var p = array<vec2<f32>, 3>(vec2(-1.0, -3.0), vec2(-1.0, 1.0), vec2(3.0, 1.0));
  var out: VsOut;
  out.pos = vec4<f32>(p[i], 0.0, 1.0);
  out.uv = p[i] * 0.5 + 0.5;
  return out;
}
@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
  let t = u.phase;
  let band = 0.5 + 0.5 * sin(in.uv.x * 18.0 + t * 2.0);
  let g = 0.5 + 0.5 * sin(in.uv.y * 12.0 - t);
  return vec4<f32>(in.uv.x * 0.7 + band * 0.3, in.uv.y * 0.6 + g * 0.4, 0.35 + 0.25 * band, 1.0);
}
"#;

impl FrameTex {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> FrameTex {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("video-frame"),
            size: wgpu::Extent3d {
                width: VIDEO_W,
                height: VIDEO_H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("frame-shader"),
            source: wgpu::ShaderSource::Wgsl(FRAME_SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame-uniform"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("frame-pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("frame"),
                    bind_group_layouts: &[Some(&layout)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        FrameTex {
            texture,
            view,
            pipeline,
            bind,
            uniform,
        }
    }

    fn render(&self, device: &wgpu::Device, queue: &wgpu::Queue, tick: u32) {
        let phase = (tick as f32) * 0.05;
        let data: [f32; 4] = [phase, VIDEO_W as f32, VIDEO_H as f32, 0.0];
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&data));
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([enc.finish()]);
    }
}

/// The zero-copy path: a textured triangle drawn inside egui's own render pass, sampling the
/// compositor texture directly.
struct VideoQuad {
    pipeline: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
}

impl CallbackTrait for VideoQuad {
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        _resources: &egui_wgpu::CallbackResources,
    ) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

fn video_quad(device: &wgpu::Device, tex: &FrameTex, target: wgpu::TextureFormat) -> VideoQuad {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("egui-video"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
struct VsOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
  var p = array<vec2<f32>, 3>(vec2(-1.0, -3.0), vec2(-1.0, 1.0), vec2(3.0, 1.0));
  var out: VsOut;
  out.pos = vec4<f32>(p[i], 0.0, 1.0);
  out.uv = vec2<f32>(p[i].x * 0.5 + 0.5, 0.5 - p[i].y * 0.5);
  return out;
}
@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> { return textureSample(src, samp, in.uv); }
"#
            .into(),
        ),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("egui-video"),
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
        ],
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("egui-video"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&tex.view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("egui-video"),
        layout: Some(
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("egui-video"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            }),
        ),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            targets: &[Some(wgpu::ColorTargetState {
                format: target,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    VideoQuad { pipeline, bind }
}

struct Timeline {
    scene: scene::Scene,
    zoom: f32,
    scroll: f32,
    playhead: f32,
    playing: bool,
}

struct Gfx {
    window: Arc<Window>,
    state: egui_winit::State,
    painter: egui_wgpu::winit::Painter,
    ctx: egui::Context,
    device: wgpu::Device,
    queue: wgpu::Queue,
    frame_tex: FrameTex,
    quad: Option<Arc<VideoQuad>>,
    thumb: egui::TextureHandle,
    readback_tex: Option<egui::TextureHandle>,
    readback_rgba: Vec<u8>,
}

struct App {
    args: Args,
    timeline: Timeline,
    gfx: Option<Gfx>,
    metrics: Metrics,
    paint_metrics: Metrics,
    tick: u32,
    start: Instant,
    last_frame: Option<Instant>,
    first_frame_at: Option<Instant>,
    first_frame_written: bool,
}

fn parse() -> Args {
    let map = scene::parse_args();
    let video = match scene::arg_str(&map, "video-mode", "none") {
        "readback" => VideoMode::Readback,
        "native" => VideoMode::Native,
        _ => VideoMode::None,
    };
    Args {
        clips: scene::arg_usize(&map, "clips", 1000),
        frames: scene::arg_usize(&map, "frames", 240),
        video,
        zoom: scene::arg_f32(&map, "zoom", 60.0),
        width: scene::arg_usize(&map, "width", 1600) as u32,
        height: scene::arg_usize(&map, "height", 900) as u32,
        out: scene::arg_str(&map, "out", "egui.json").to_string(),
        interact: if scene::arg_str(&map, "interact", "scroll") == "zoom" {
            1
        } else {
            0
        },
        idle_seconds: scene::arg_usize(&map, "idle-seconds", 0) as u64,
        run_seconds: scene::arg_usize(&map, "run-seconds", 0) as u64,
        startup_marker: map.get("startup-marker").cloned(),
        vsync: scene::arg_usize(&map, "vsync", 0) == 1,
    }
}

fn main() {
    let args = parse();
    let scene = scene::generate(args.clips, 7);
    eprintln!(
        "egui-poc: {} clips, {:.0}s timeline, {} frames, video {:?}",
        scene.total_clips(),
        scene.duration,
        args.frames,
        args.video
    );
    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App {
        timeline: Timeline {
            scene,
            zoom: args.zoom,
            scroll: 0.0,
            playhead: 0.0,
            playing: args.idle_seconds == 0,
        },
        gfx: None,
        metrics: Metrics::default(),
        paint_metrics: Metrics::default(),
        tick: 0,
        start: Instant::now(),
        last_frame: None,
        first_frame_at: None,
        first_frame_written: false,
        args,
    };
    event_loop.run_app(&mut app).expect("run");
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        el.set_control_flow(ControlFlow::Poll);
        let attrs = Window::default_attributes()
            .with_title("egui-poc 鈥?Arcline UI validation")
            .with_inner_size(PhysicalSize::new(self.args.width, self.args.height));
        let window = Arc::new(el.create_window(attrs).expect("window"));
        let ctx = egui::Context::default();
        let mut painter = pollster::block_on(egui_wgpu::winit::Painter::new(
            ctx.clone(),
            egui_wgpu::WgpuConfiguration {
                surface: egui_wgpu::SurfaceConfig {
                    present_mode: if self.args.vsync {
                        wgpu::PresentMode::Fifo
                    } else {
                        wgpu::PresentMode::AutoNoVsync
                    },
                    desired_maximum_frame_latency: Some(1),
                },
                ..Default::default()
            },
            false,
            egui_wgpu::RendererOptions::default(),
        ));
        pollster::block_on(painter.set_window(egui::ViewportId::ROOT, Some(window.clone())))
            .expect("surface");
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            painter.max_texture_side(),
        );
        let rs = painter.render_state().expect("render state");
        let (device, queue) = (rs.device.clone(), rs.queue.clone());
        let frame_tex = FrameTex::new(&device, wgpu::TextureFormat::Rgba8UnormSrgb);
        let quad = if self.args.video == VideoMode::Native {
            Some(Arc::new(video_quad(&device, &frame_tex, rs.target_format)))
        } else {
            None
        };
        let mut thumb_data = Vec::new();
        scene::synthetic_frame(320, 180, 0, &mut thumb_data);
        let thumb = ctx.load_texture(
            "thumb",
            egui::ColorImage::from_rgba_unmultiplied([320, 180], &thumb_data),
            egui::TextureOptions::LINEAR,
        );
        self.gfx = Some(Gfx {
            window,
            state,
            painter,
            ctx,
            device,
            queue,
            frame_tex,
            quad,
            thumb,
            readback_tex: None,
            readback_rgba: Vec::new(),
        });
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(g) = self.gfx.as_mut() else { return };
        let response = g.state.on_window_event(&g.window, &event);
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) => {
                if let (Some(w), Some(h)) = (
                    std::num::NonZeroU32::new(size.width),
                    std::num::NonZeroU32::new(size.height),
                ) {
                    g.painter.on_window_resized(egui::ViewportId::ROOT, w, h);
                }
            }
            WindowEvent::RedrawRequested => self.frame(),
            _ => {
                if response.repaint {
                    g.window.request_redraw();
                }
            }
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if self.args.frames > 0 && self.tick as usize >= self.args.frames {
            el.exit();
            return;
        }
        if self.args.run_seconds > 0
            && self.start.elapsed() >= Duration::from_secs(self.args.run_seconds)
        {
            el.exit();
            return;
        }
        if self.args.idle_seconds > 0 {
            el.set_control_flow(ControlFlow::Wait);
            if let Some(first) = self.first_frame_at
                && first.elapsed() >= Duration::from_secs(self.args.idle_seconds)
            {
                el.exit();
            }
            return;
        }
        if let Some(g) = &self.gfx {
            g.window.request_redraw();
        }
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        let report = self.metrics.report(
            "egui",
            serde_json::json!({
                "clips": self.timeline.scene.total_clips(),
                "zoom": self.args.zoom,
                "video": format!("{:?}", self.args.video),
                "ui_and_paint_us_p50": Metrics::percentile(&self.paint_metrics.frames, 0.5),
                "ui_and_paint_us_p95": Metrics::percentile(&self.paint_metrics.frames, 0.95),
            }),
        );
        std::fs::write(
            &self.args.out,
            serde_json::to_string_pretty(&report).unwrap(),
        )
        .ok();
        println!("{}", serde_json::to_string(&report).unwrap());
    }
}

impl App {
    fn frame(&mut self) {
        let video = self.args.video;
        let frames = self.args.frames;
        let width = self.args.width as f32;
        let Some(g) = self.gfx.as_mut() else { return };
        if self.tick as usize > frames {
            return;
        }
        let started = Instant::now();
        let input = g.state.take_egui_input(&g.window);

        if self.timeline.playing {
            self.timeline.playhead += 1.0 / 60.0;
            if self.timeline.playhead > self.timeline.scene.duration {
                self.timeline.playhead = 0.0;
                self.timeline.scroll = 0.0;
            }
        }
        let view_width = (width - LANE_LEFT).max(200.0);
        let visible = view_width / self.timeline.zoom;
        if self.timeline.playhead > self.timeline.scroll + visible * 0.8 {
            self.timeline.scroll = self.timeline.playhead - visible * 0.2;
        }
        // A zoom drag: in an immediate-mode UI this only changes the mapping scalar; nothing is
        // re-laid-out until the next paint.
        if self.args.interact == 1 {
            let dir = if (self.tick / 60) % 4 < 2 { 1.0 } else { -1.0 };
            self.timeline.zoom = (self.timeline.zoom * (1.0 + 0.01 * dir)).clamp(20.0, 200.0);
        }

        // Compositor frame on the window's device (the video the editor previews).
        if video != VideoMode::None {
            g.frame_tex.render(&g.device, &g.queue, self.tick);
        }
        // Arcline's current path: read the composed frame back to the CPU, then upload it to egui.
        if video == VideoMode::Readback {
            readback_frame(g, self.tick);
        }

        let quad = g.quad.clone();
        let thumb_id = g.thumb.id();
        let readback_id: Option<TextureId> = g.readback_tex.as_ref().map(|t| t.id());
        let ctx = g.ctx.clone();
        let timeline = &mut self.timeline;
        let ui_started = Instant::now();
        let mut out = ctx.run_ui(input, |ui| {
            draw_shell(ui, timeline, thumb_id, readback_id, video);
            if video == VideoMode::Native
                && let Some(quad) = &quad
            {
                ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                    preview_rect(ui.max_rect()),
                    VideoQuadRef(quad.clone()),
                ));
            }
        });
        let ui_us = ui_started.elapsed().as_micros() as u64;
        g.state
            .handle_platform_output(&g.window, out.platform_output);
        let clipped = g.ctx.tessellate(out.shapes, out.pixels_per_point);
        let paint_started = Instant::now();
        g.painter.paint_and_update_textures(
            egui::ViewportId::ROOT,
            out.pixels_per_point,
            [0.07, 0.07, 0.08, 1.0],
            &clipped,
            &mut out.textures_delta,
            Vec::new(),
            &g.window,
        );
        let paint_us = paint_started.elapsed().as_micros() as u64;
        self.tick += 1;
        if self.tick > 1 {
            self.metrics.record(started.elapsed().as_micros() as u64);
            self.paint_metrics.record(ui_us + paint_us);
        }
        let now = Instant::now();
        if let Some(prev) = self.last_frame.replace(now) {
            self.metrics
                .record_interval(now.duration_since(prev).as_micros() as u64);
        }
        if !self.first_frame_written {
            self.first_frame_written = true;
            self.first_frame_at = Some(now);
            if let Some(path) = &self.args.startup_marker {
                let _ = std::fs::write(
                    path,
                    serde_json::json!({
                        "first_frame_ms": self.start.elapsed().as_secs_f64() * 1000.0,
                        "frames": self.tick,
                    })
                    .to_string(),
                );
            }
        }
    }
}

/// The quad, owned by the paint callback (egui requires `'static` callbacks).
struct VideoQuadRef(Arc<VideoQuad>);

impl CallbackTrait for VideoQuadRef {
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        _resources: &egui_wgpu::CallbackResources,
    ) {
        render_pass.set_pipeline(&self.0.pipeline);
        render_pass.set_bind_group(0, &self.0.bind, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

/// The current Arcline path: GPU frame -> readback -> egui texture upload.
fn readback_frame(g: &mut Gfx, tick: u32) {
    let padded = (VIDEO_W * 4).div_ceil(256) * 256;
    let buffer = g.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded as u64) * VIDEO_H as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = g.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &g.frame_tex.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(VIDEO_H),
            },
        },
        wgpu::Extent3d {
            width: VIDEO_W,
            height: VIDEO_H,
            depth_or_array_layers: 1,
        },
    );
    g.queue.submit([enc.finish()]);
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    g.device.poll(wgpu::PollType::wait_indefinitely()).ok();
    let _ = rx.recv();
    {
        let data = slice.get_mapped_range().expect("mapped range");
        g.readback_rgba.clear();
        g.readback_rgba.reserve((VIDEO_W * VIDEO_H * 4) as usize);
        for row in 0..VIDEO_H {
            let start = (row * padded) as usize;
            g.readback_rgba
                .extend_from_slice(&data[start..start + (VIDEO_W * 4) as usize]);
        }
    }
    buffer.unmap();
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [VIDEO_W as usize, VIDEO_H as usize],
        &g.readback_rgba,
    );
    g.readback_tex = Some(g.ctx.load_texture(
        format!("frame-{tick}"),
        image,
        egui::TextureOptions::LINEAR,
    ));
}

fn preview_rect(shell: Rect) -> Rect {
    Rect::from_min_size(
        egui::pos2(shell.right() - 336.0, shell.top() + 44.0),
        Vec2::new(320.0, 180.0),
    )
}

fn draw_shell(
    ui: &mut egui::Ui,
    tl: &mut Timeline,
    thumb: TextureId,
    readback: Option<TextureId>,
    video: VideoMode,
) {
    let rect = ui.max_rect();
    let p = ui.painter();
    p.rect_filled(
        rect,
        CornerRadius::ZERO,
        Color32::from_rgb(0x12, 0x13, 0x16),
    );

    let toolbar = Rect::from_min_size(rect.min, Vec2::new(rect.width(), TOOLBAR_H));
    p.rect_filled(
        toolbar,
        CornerRadius::ZERO,
        Color32::from_rgb(0x1b, 0x1d, 0x22),
    );
    p.text(
        toolbar.left_center() + Vec2::new(12.0, 0.0),
        Align2::LEFT_CENTER,
        format!("Arcline timeline POC 鈥?{} clips", tl.scene.total_clips()),
        FontId::proportional(12.0),
        Color32::from_gray(200),
    );

    let ruler = Rect::from_min_size(
        egui::pos2(rect.left() + LANE_LEFT, rect.top() + TOOLBAR_H),
        Vec2::new((rect.width() - LANE_LEFT).max(1.0), RULER_H),
    );
    p.rect_filled(
        ruler,
        CornerRadius::ZERO,
        Color32::from_rgb(0x16, 0x18, 0x1d),
    );
    let secs_per_px = 1.0 / tl.zoom;
    let first = tl.scroll;
    let last = tl.scroll + ruler.width() * secs_per_px;
    let mut t = first.floor();
    while t <= last {
        let x = ruler.left() + (t - tl.scroll) * tl.zoom;
        let major = (t as i64) % 5 == 0;
        p.line_segment(
            [
                egui::pos2(x, ruler.bottom() - if major { 10.0 } else { 5.0 }),
                egui::pos2(x, ruler.bottom()),
            ],
            Stroke::new(1.0, Color32::from_gray(if major { 150 } else { 80 })),
        );
        if major {
            p.text(
                egui::pos2(x + 3.0, ruler.top() + 3.0),
                Align2::LEFT_TOP,
                format!("{}:{:02}", (t as i64) / 60, (t as i64) % 60),
                FontId::proportional(10.0),
                Color32::from_gray(190),
            );
        }
        t += 1.0;
    }

    let lanes_top = ruler.bottom();
    let canvas = Rect::from_min_max(egui::pos2(rect.left() + LANE_LEFT, lanes_top), rect.max);
    let mut y = lanes_top;
    for (track, clips) in tl.scene.visible(first, last + secs_per_px * 200.0) {
        let lane = Rect::from_min_size(
            egui::pos2(rect.left(), y),
            Vec2::new(rect.width(), track.kind.lane_height()),
        );
        p.rect_filled(
            lane,
            CornerRadius::ZERO,
            Color32::from_rgb(0x17, 0x19, 0x1e),
        );
        p.text(
            egui::pos2(rect.left() + 8.0, y + 4.0),
            Align2::LEFT_TOP,
            format!("{}", track.kind.name()),
            FontId::proportional(11.0),
            Color32::from_gray(190),
        );
        for clip in clips {
            let x0 = canvas.left() + (clip.start - tl.scroll) * tl.zoom;
            let x1 = x0 + clip.duration * tl.zoom;
            let r = Rect::from_min_max(
                egui::pos2(x0.max(canvas.left() - 4.0), y + 2.0),
                egui::pos2(
                    x1.min(canvas.right() + 4.0),
                    y + track.kind.lane_height() - 2.0,
                ),
            );
            if r.width() <= 0.0 || r.right() < canvas.left() || r.left() > canvas.right() {
                continue;
            }
            let color = Color32::from_rgb(clip.color[0], clip.color[1], clip.color[2]);
            p.rect_filled(r, CornerRadius::same(3), color);
            if r.width() > 42.0 {
                p.text(
                    r.left_top() + Vec2::new(4.0, 2.0),
                    Align2::LEFT_TOP,
                    &clip.label,
                    FontId::proportional(9.5),
                    Color32::from_gray(235),
                );
            }
            if clip.thumb && r.width() > 30.0 && track.kind.lane_height() >= 50.0 {
                let th = Rect::from_min_size(
                    egui::pos2(r.left() + 2.0, r.top() + 14.0),
                    Vec2::new((r.width() - 4.0).min(64.0), (r.height() - 18.0).max(0.0)),
                );
                p.image(
                    thumb,
                    th,
                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            if !clip.wave.is_empty() {
                let mid = r.center().y
                    + if track.kind.lane_height() > 40.0 {
                        8.0
                    } else {
                        0.0
                    };
                let amp = (r.height() * 0.32).min(18.0);
                let step = r.width() / clip.wave.len() as f32;
                let points: Vec<egui::Pos2> = clip
                    .wave
                    .iter()
                    .enumerate()
                    .map(|(i, v)| egui::pos2(r.left() + i as f32 * step, mid - v * amp))
                    .collect();
                p.add(egui::Shape::line(
                    points,
                    Stroke::new(1.0, Color32::from_rgba_unmultiplied(220, 235, 255, 190)),
                ));
            }
            for key in &clip.keys {
                let cx = x0 + key.at * tl.zoom;
                if cx < canvas.left() || cx > canvas.right() {
                    continue;
                }
                let cy = r.top() + 6.0 + key.value * (r.height() - 12.0).max(1.0);
                let s = 3.0;
                p.add(egui::Shape::convex_polygon(
                    vec![
                        egui::pos2(cx, cy - s),
                        egui::pos2(cx + s, cy),
                        egui::pos2(cx, cy + s),
                        egui::pos2(cx - s, cy),
                    ],
                    Color32::from_rgb(255, 214, 92),
                    Stroke::NONE,
                ));
            }
        }
        y += track.kind.lane_height();
    }

    let px = canvas.left() + (tl.playhead - tl.scroll) * tl.zoom;
    if px >= canvas.left() && px <= canvas.right() {
        p.line_segment(
            [egui::pos2(px, ruler.top()), egui::pos2(px, rect.bottom())],
            Stroke::new(1.0, Color32::from_rgb(0xff, 0x5c, 0x5c)),
        );
        p.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(px - 5.0, ruler.top()),
                egui::pos2(px + 5.0, ruler.top()),
                egui::pos2(px, ruler.top() + 7.0),
            ],
            Color32::from_rgb(0xff, 0x5c, 0x5c),
            Stroke::NONE,
        ));
    }

    if video == VideoMode::Readback
        && let Some(id) = readback
    {
        let preview = preview_rect(rect);
        p.rect_filled(preview, CornerRadius::same(4), Color32::BLACK);
        p.image(
            id,
            preview,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    if video == VideoMode::Native {
        let preview = preview_rect(rect);
        p.rect_filled(preview, CornerRadius::same(4), Color32::BLACK);
    }

    let sb = Rect::from_min_size(
        egui::pos2(rect.left() + LANE_LEFT, rect.bottom() - 9.0),
        Vec2::new((rect.width() - LANE_LEFT).max(1.0), 9.0),
    );
    p.rect_filled(sb, CornerRadius::ZERO, Color32::from_rgb(0x1b, 0x1d, 0x22));
    let total = tl.scene.duration.max(1.0);
    let thumb_w = (sb.width() * (tl.scroll + 1.0) / total).clamp(20.0, sb.width());
    let start = sb.left() + (sb.width() - thumb_w) * (tl.scroll / total).clamp(0.0, 1.0);
    p.rect_filled(
        Rect::from_min_size(egui::pos2(start, sb.top()), Vec2::new(thumb_w, 9.0)),
        CornerRadius::same(3),
        Color32::from_gray(90),
    );

    let response = ui.interact(canvas, egui::Id::unique("canvas"), Sense::click_and_drag());
    if response.dragged() {
        tl.scroll = (tl.scroll - response.drag_delta().x / tl.zoom).max(0.0);
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.1 {
            tl.zoom = (tl.zoom * (scroll * 0.002).exp()).clamp(4.0, 4000.0);
        }
    }
}
