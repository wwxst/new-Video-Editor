//! Slint feasibility POC for Arcline.
//!
//! This binary draws the same six-lane scene as the egui POC, but in a full four-panel shell:
//! MEDIA / PREVIEW / INSPECTOR / TIMELINE, with draggable splitters, a fixed track header, a
//! two-axis Flickable timeline, frame-level zoom, selection, Chinese text and IME pre-edit.
//!
//! The preview is rendered by our own wgpu pipeline into a texture and imported into Slint with
//! `Image::try_from(wgpu::Texture)`. The `readback` mode is the explicit CPU path used as a
//! control: it copies 8 MB out of the GPU and uploads it again, so the two timings are comparable.
//!
//! ```text
//! slint-poc --clips 1000 --frames 240 --video-mode native --out slint.json
//! slint-poc --scenario selected --shot shot.png --frames 120 --video-mode native
//! slint-poc --embed-parent 0x1234 --embed-seconds 8 --width 1200 --height 800
//! ```

slint::include_modules!();

#[cfg(windows)]
mod embed_host;

use scene::{Clip, Metrics, Scene, Track};
use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const VIDEO_W: u32 = 1920;
const VIDEO_H: u32 = 1080;

#[derive(Clone, Copy, PartialEq, Debug)]
enum VideoMode {
    None,
    Readback,
    Native,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Interact {
    Scroll,
    Zoom,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Scenario {
    Default,
    Selected,
    Resized,
    Scrolled,
    Zoomed,
    About,
}

#[derive(Clone, Debug)]
struct Args {
    clips: usize,
    frames: usize,
    video: VideoMode,
    interact: Interact,
    scenario: Scenario,
    width: u32,
    height: u32,
    out: PathBuf,
    shot: Option<PathBuf>,
    shot_delay_ms: u64,
    embed_parent: Option<isize>,
    embed_host: bool,
    embed_seconds: u64,
    idle_seconds: u64,
    startup_marker: Option<PathBuf>,
    result: Option<PathBuf>,
    scale: Option<f32>,
    fallback: bool,
    cull: bool,
    timeline_only: bool,
    project: String,
}

fn parse_args() -> Args {
    let map = scene::parse_args();
    let video = match scene::arg_str(&map, "video-mode", "native") {
        "none" => VideoMode::None,
        "readback" => VideoMode::Readback,
        "native" => VideoMode::Native,
        _ => VideoMode::Native,
    };
    let interact = match scene::arg_str(&map, "interact", "scroll") {
        "zoom" => Interact::Zoom,
        _ => Interact::Scroll,
    };
    let scenario = match scene::arg_str(&map, "scenario", "default") {
        "selected" => Scenario::Selected,
        "resized" => Scenario::Resized,
        "scrolled" => Scenario::Scrolled,
        "zoomed" => Scenario::Zoomed,
        "about" => Scenario::About,
        _ => Scenario::Default,
    };
    let embed_parent = map.get("embed-parent").and_then(|s| {
        let s = s.trim();
        if let Some(hex) = s.strip_prefix("0x") {
            isize::from_str_radix(hex, 16).ok()
        } else {
            s.parse::<isize>().ok()
        }
    });
    Args {
        clips: scene::arg_usize(&map, "clips", 1000),
        frames: scene::arg_usize(&map, "frames", 240),
        video,
        interact,
        scenario,
        width: scene::arg_usize(&map, "width", 1600) as u32,
        height: scene::arg_usize(&map, "height", 1000) as u32,
        out: PathBuf::from(scene::arg_str(&map, "out", "slint.json")),
        shot: map.get("shot").map(PathBuf::from),
        shot_delay_ms: scene::arg_usize(&map, "shot-delay-ms", 900) as u64,
        embed_parent,
        embed_host: map.contains_key("embed-host"),
        embed_seconds: scene::arg_usize(&map, "embed-seconds", 8) as u64,
        idle_seconds: scene::arg_usize(&map, "idle-seconds", 0) as u64,
        startup_marker: map.get("startup-marker").map(PathBuf::from),
        result: map.get("result").map(PathBuf::from),
        scale: map.get("scale").and_then(|s| s.parse().ok()),
        fallback: scene::arg_usize(&map, "fallback", 0) == 1,
        cull: scene::arg_usize(&map, "cull", 0) == 1,
        timeline_only: scene::arg_str(&map, "ui", "full") == "timeline",
        project: scene::arg_str(&map, "project", "Arcline · Slint 技术验证").to_string(),
    }
}

// ---------------------------------------------------------------------------------------------
// GPU: one device, shared with Slint through the unstable-wgpu-30 backend hook.
// ---------------------------------------------------------------------------------------------

struct Gpu {
    #[allow(dead_code)]
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Gpu {
    fn acquire(force_fallback: bool) -> Option<Gpu> {
        let backends = if cfg!(target_vendor = "apple") {
            wgpu::Backends::METAL
        } else if cfg!(windows) {
            wgpu::Backends::DX12
        } else {
            wgpu::Backends::VULKAN
        };
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: force_fallback,
            compatible_surface: None,
            ..Default::default()
        }))
        .inspect_err(|e| eprintln!("slint-poc: no adapter: {e}"))
        .ok()?;
        let info = adapter.get_info();
        eprintln!(
            "slint-poc: adapter {} ({:?}, {:?})",
            info.name, info.backend, info.device_type
        );
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("slint-poc"),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .inspect_err(|e| eprintln!("slint-poc: no device: {e}"))
        .ok()?;
        Some(Gpu {
            instance,
            adapter,
            device,
            queue,
        })
    }
}

#[derive(Clone)]
struct FrameTex {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
    uniform: wgpu::Buffer,
}

const SHADER: &str = r#"
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
  let horizon = smoothstep(0.42, 0.46, in.uv.y);
  let sky = vec3<f32>(0.12, 0.18, 0.34) + vec3<f32>(0.25, 0.16, 0.10) * in.uv.y;
  let water = vec3<f32>(0.04, 0.15, 0.22) + vec3<f32>(0.10, 0.22, 0.26) * band;
  let ground = mix(sky, water, horizon);
  return vec4<f32>(ground + vec3<f32>(0.10, 0.06, 0.02) * g, 1.0);
}
"#;

impl FrameTex {
    fn new(device: &wgpu::Device) -> FrameTex {
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
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("frame"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
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
            label: Some("frame"),
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
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
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

    fn draw(&self, device: &wgpu::Device, queue: &wgpu::Queue, tick: u32) {
        let data: [f32; 4] = [tick as f32 * 0.05, VIDEO_W as f32, VIDEO_H as f32, 0.0];
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&data));
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame"),
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

// ---------------------------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------------------------

struct Model {
    args: Args,
    scene: Scene,
    zoom: f32,
    selected_id: i32,
    selected_asset: i32,
    playhead: f32,
    playing: bool,
    tick: u32,
    metrics: Metrics,
    scroll_x: f32,
    v_scroll: f32,
    last_zoom_publish: f32,
    published_from: f32,
    published_to: f32,
    snapshot_taken: bool,
    first_frame: Option<Instant>,
    process_start: Instant,
}

impl Model {
    fn duration(&self) -> f32 {
        self.scene.duration.max(1.0)
    }

    fn selected_clip(&self) -> Option<(&Track, &Clip)> {
        if self.selected_id < 0 {
            None
        } else {
            self.scene.clip(self.selected_id as usize)
        }
    }
}

fn main() {
    let mut args = parse_args();

    // `--embed-host` creates a native Agent-client shell in this process and makes the Slint
    // window a true WS_CHILD of it before the backend creates the window.
    #[cfg(windows)]
    let mut host_hwnd = None;
    #[cfg(windows)]
    if args.embed_host {
        let (hwnd, cw, ch) = embed_host::create_host(args.width, args.height)
            .expect("create the native Agent host window");
        args.width = cw;
        args.height = ch
            .saturating_sub(embed_host::header_height() as u32)
            .max(120);
        args.embed_parent = Some(hwnd);
        host_hwnd = Some(hwnd);
    }

    let scene = scene::generate(args.clips, 7);
    eprintln!(
        "slint-poc: {} clips, {:.1}s timeline, {} frames, video {:?}, scenario {:?}",
        scene.total_clips(),
        scene.duration,
        args.frames,
        args.video,
        args.scenario
    );

    let gpu = Gpu::acquire(args.fallback);
    let mut selector = slint::BackendSelector::new().backend_name("winit".into());
    if let Some(g) = &gpu {
        selector = selector.require_wgpu_30(slint::wgpu_30::WGPUConfiguration::Manual {
            instance: g.instance.clone(),
            adapter: g.adapter.clone(),
            device: g.device.clone(),
            queue: g.queue.clone(),
        });
    } else {
        selector = selector.renderer_name("software".into());
    }

    #[cfg(windows)]
    if let Some(parent) = args.embed_parent {
        let (ew, eh) = (args.width, args.height);
        let top = if args.embed_host {
            embed_host::header_height()
        } else {
            0
        };
        selector = selector.with_winit_window_attributes_hook(move |attrs| {
            embed::child_window_attributes(attrs, parent, ew, eh, top)
        });
    }

    if let Err(e) = selector.select() {
        eprintln!("slint-poc: backend selection failed: {e}");
        std::process::exit(3);
    }

    let ui = AppWindow::new().expect("component");
    ui.set_requested_width(args.width as f32);
    ui.set_requested_height(args.height as f32);
    ui.window().set_size(slint::LogicalSize::new(
        args.width as f32,
        args.height as f32,
    ));
    ui.set_project_name(slint::SharedString::from(args.project.clone()));
    ui.set_show_preview(args.video != VideoMode::None);
    ui.set_timeline_only(args.timeline_only);

    let model = Rc::new(RefCell::new(Model {
        args: args.clone(),
        scene,
        zoom: 60.0,
        selected_id: if args.scenario == Scenario::Selected {
            3
        } else {
            -1
        },
        selected_asset: -1,
        playhead: 12.0,
        playing: true,
        tick: 0,
        metrics: Metrics::default(),
        scroll_x: 0.0,
        v_scroll: 0.0,
        last_zoom_publish: 60.0,
        published_from: -1.0,
        published_to: -1.0,
        snapshot_taken: false,
        first_frame: None,
        process_start: Instant::now(),
    }));

    {
        let mut m = model.borrow_mut();
        match args.scenario {
            Scenario::Selected => {
                m.playhead = 4.0;
                m.scroll_x = 0.0;
            }
            Scenario::Resized => {
                ui.set_left_frac(0.30);
                ui.set_right_frac(0.21);
                ui.set_bottom_frac(0.41);
            }
            Scenario::Scrolled => {
                m.playhead = 90.0;
                m.scroll_x = 86.0 * m.zoom;
                m.v_scroll = 54.0;
                ui.set_scroll_request_x(m.scroll_x);
                ui.set_scroll_request_y(m.v_scroll);
            }
            Scenario::Zoomed => {
                m.zoom = 300.0;
                m.playhead = 14.0;
                m.scroll_x = 12.0 * m.zoom;
                ui.set_zoom(300.0);
                ui.set_zoom_label(slint::SharedString::from("300 px/s"));
                ui.set_scroll_request_x(m.scroll_x);
            }
            Scenario::About => {
                ui.set_show_about(true);
            }
            Scenario::Default => {}
        }
    }

    // Font thumbnail and initial preview texture.
    let mut thumb_data = Vec::new();
    scene::synthetic_frame(320, 180, 0, &mut thumb_data);
    let thumb =
        slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&thumb_data, 320, 180);
    ui.set_thumb(slint::Image::from_rgba8(thumb));

    publish_scene(&ui, &model);
    publish_ticks(&ui, &model);
    refresh_clip_window(&ui, &model, true);
    update_selection(&ui, &model);
    ui.set_zoom_label(slint::SharedString::from(format!(
        "{:.0} px/s",
        model.borrow().zoom
    )));
    ui.set_fps(30.0);
    ui.set_fps_label(slint::SharedString::from("30 fps"));
    ui.set_track_count_label(slint::SharedString::from(format!(
        "{} 轨",
        model.borrow().scene.tracks.len()
    )));
    ui.set_tracks_height(model.borrow().scene.lanes_height());
    ui.set_duration(model.borrow().duration());
    ui.set_media_count_label(slint::SharedString::from(format!(
        "{} 项",
        media_items().len()
    )));
    ui.set_media(slint::ModelRc::new(slint::VecModel::from(media_items())));
    ui.set_dpi_label(slint::SharedString::from(format_scale(
        ui.window().scale_factor(),
    )));
    ui.set_renderer_label(slint::SharedString::from(if gpu.is_some() {
        "FemtoVG / wgpu 30"
    } else {
        "software renderer"
    }));

    // Interaction callbacks.
    {
        let weak = ui.as_weak();
        let model = model.clone();
        ui.on_select_clip(move |id| {
            if let Some(ui) = weak.upgrade() {
                model.borrow_mut().selected_id = id;
                update_selection(&ui, &model);
                publish_tracks(&ui, &model);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let model = model.clone();
        ui.on_select_asset(move |idx| {
            if let Some(ui) = weak.upgrade() {
                model.borrow_mut().selected_asset = idx;
                ui.set_selected_asset(idx);
                let items = media_items();
                if let Some(item) = items.get(idx as usize) {
                    ui.set_status(slint::SharedString::from(format!("媒体池：{}", item.name)));
                    ui.set_selected_label(item.name.clone());
                    ui.set_editable_label(item.name.clone());
                    ui.set_selected_kind(item.kind.clone());
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        let model = model.clone();
        ui.on_zoom_in(move || {
            if let Some(ui) = weak.upgrade() {
                {
                    let mut m = model.borrow_mut();
                    m.zoom = (m.zoom * 1.5).clamp(2.0, 400.0);
                }
                after_zoom(&ui, &model);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let model = model.clone();
        ui.on_zoom_out(move || {
            if let Some(ui) = weak.upgrade() {
                {
                    let mut m = model.borrow_mut();
                    m.zoom = (m.zoom / 1.5).clamp(2.0, 400.0);
                }
                after_zoom(&ui, &model);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let model = model.clone();
        ui.on_zoom_set(move |v| {
            if let Some(ui) = weak.upgrade() {
                {
                    let mut m = model.borrow_mut();
                    m.zoom = v.clamp(2.0, 400.0);
                }
                after_zoom(&ui, &model);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let model = model.clone();
        ui.on_toggle_play(move || {
            if let Some(ui) = weak.upgrade() {
                let playing = {
                    let mut m = model.borrow_mut();
                    m.playing = !m.playing;
                    m.playing
                };
                ui.set_playing(playing);
                ui.set_status(slint::SharedString::from(if playing {
                    "播放中"
                } else {
                    "已暂停"
                }));
            }
        });
    }
    {
        let weak = ui.as_weak();
        let model = model.clone();
        ui.on_step_frame(move |dir| {
            if let Some(ui) = weak.upgrade() {
                {
                    let mut m = model.borrow_mut();
                    m.playing = false;
                    m.playhead = (m.playhead + dir as f32 / 30.0).clamp(0.0, m.duration());
                }
                ui.set_playing(false);
                publish_playhead(&ui, &model);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let model = model.clone();
        ui.on_seek_frame(move |frame| {
            if let Some(ui) = weak.upgrade() {
                {
                    let mut m = model.borrow_mut();
                    m.playhead = (frame as f32 / 30.0).clamp(0.0, m.duration());
                }
                publish_playhead(&ui, &model);
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_label_edited(move |text| {
            if let Some(ui) = weak.upgrade() {
                ui.set_selected_label(text.clone());
                ui.set_editable_label(text);
                ui.set_status(slint::SharedString::from("素材名称已修改（POC 不落盘）"));
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_about_opened(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_status(slint::SharedString::from(
                    "About Slint 已打开：Royalty-free 2.0 的署名条件",
                ));
            }
        });
    }

    // GPU frame + preview.
    let frame_tex = gpu.as_ref().map(|g| FrameTex::new(&g.device));
    let gpu_for_tick = gpu.as_ref().map(|g| (g.device.clone(), g.queue.clone()));
    let frame_tex_for_tick = frame_tex.clone();
    let preview_image: Rc<RefCell<Option<slint::Image>>> = Rc::new(RefCell::new(None));

    if args.video != VideoMode::None {
        if let (Some((device, queue)), Some(f)) = (&gpu_for_tick, &frame_tex_for_tick) {
            let p0 = Instant::now();
            f.draw(device, queue, 0);
            let image = if args.video == VideoMode::Readback {
                readback_image(device, queue, &f.texture).ok()
            } else {
                slint::Image::try_from(f.texture.clone()).ok()
            };
            if let Some(image) = image {
                ui.set_preview(image.clone());
                *preview_image.borrow_mut() = Some(image);
                ui.set_show_preview(true);
            }
            model
                .borrow_mut()
                .metrics
                .record_prepare(p0.elapsed().as_micros() as u64);
        }
    }

    // Frame timing.
    let times: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
    let intervals: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
    let rendered = Arc::new(AtomicUsize::new(0));
    let started_frame = Rc::new(Cell::new(None::<Instant>));
    let last_frame_at = Rc::new(Cell::new(None::<Instant>));
    let first_frame_done = Rc::new(Cell::new(false));
    {
        let times = Arc::clone(&times);
        let intervals = Arc::clone(&intervals);
        let rendered = Arc::clone(&rendered);
        let started_frame = Rc::clone(&started_frame);
        let last_frame_at = Rc::clone(&last_frame_at);
        let first_frame_done = Rc::clone(&first_frame_done);
        let weak = ui.as_weak();
        let model = model.clone();
        ui.window()
            .set_rendering_notifier(move |state, _api| match state {
                slint::RenderingState::BeforeRendering => {
                    started_frame.set(Some(Instant::now()));
                }
                slint::RenderingState::AfterRendering => {
                    if let Some(t0) = started_frame.take()
                        && let Ok(mut t) = times.lock()
                    {
                        t.push(t0.elapsed().as_micros() as u64);
                    }
                    let now = Instant::now();
                    if let Some(prev) = last_frame_at.replace(Some(now))
                        && let Ok(mut v) = intervals.lock()
                    {
                        v.push(now.duration_since(prev).as_micros() as u64);
                    }
                    let n = rendered.fetch_add(1, Ordering::Relaxed) + 1;
                    if !first_frame_done.get() {
                        first_frame_done.set(true);
                        model.borrow_mut().first_frame = Some(Instant::now());
                        if let Some(ui) = weak.upgrade() {
                            let elapsed = model.borrow().process_start.elapsed();
                            if let Some(path) = &model.borrow().args.startup_marker {
                                let _ = std::fs::write(
                                    path,
                                    serde_json::json!({
                                        "first_frame_ms": elapsed.as_secs_f64() * 1000.0,
                                        "frames": n,
                                        "scale_factor": ui.window().scale_factor(),
                                    })
                                    .to_string(),
                                );
                            }
                            ui.set_dpi_label(slint::SharedString::from(format_scale(
                                ui.window().scale_factor(),
                            )));
                            if model.borrow().args.scale.is_some() {
                                let scale = model.borrow().args.scale.unwrap_or(1.0);
                                ui.window().dispatch_event(
                                    slint::platform::WindowEvent::ScaleFactorChanged {
                                        scale_factor: scale,
                                    },
                                );
                                // A real monitor change also changes the physical window size.
                                let w = (model.borrow().args.width as f32 * scale).round() as u32;
                                let h = (model.borrow().args.height as f32 * scale).round() as u32;
                                ui.window()
                                    .set_size(slint::PhysicalSize::new(w.max(1), h.max(1)));
                                ui.set_dpi_label(slint::SharedString::from(format_scale(scale)));
                            }
                        }
                    }
                    if model.borrow().args.frames > 0 && n >= model.borrow().args.frames {
                        let _ = slint::quit_event_loop();
                    }
                }
                _ => {}
            })
            .expect("rendering notifier");
    }

    // Drives the transport and the scroll/zoom stress modes.
    let weak = ui.as_weak();
    let model_for_tick = model.clone();
    let timer = slint::Timer::default();
    {
        let gpu_for_tick = gpu_for_tick.clone();
        let frame_tex_for_tick = frame_tex_for_tick.clone();
        let preview_image = preview_image.clone();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(8),
            move || {
                let Some(ui) = weak.upgrade() else { return };
                let (n, video, frames, playhead, scroll_target, zoom_changed) = {
                    let mut m = model_for_tick.borrow_mut();
                    m.tick = m.tick.wrapping_add(1);
                    if m.playing {
                        m.playhead += 1.0 / 60.0;
                        if m.playhead > m.duration() {
                            m.playhead = 0.0;
                        }
                    }
                    let n = m.tick;
                    let mut scroll_target = None;
                    let mut zoom_changed = false;
                    if m.args.interact == Interact::Scroll {
                        // Keep the playhead in the middle third, like the real timeline.
                        let target = (m.playhead - 2.0).max(0.0) * m.zoom;
                        if (target - m.scroll_x).abs() > 1.0 {
                            m.scroll_x = target;
                            scroll_target = Some(target);
                        }
                    } else if m.args.interact == Interact::Zoom && n % 30 == 0 {
                        let dir = if (n / 120) % 2 == 0 { 1.0 } else { -1.0 };
                        m.zoom = (m.zoom * (1.0 + 0.04 * dir)).clamp(2.0, 400.0);
                        zoom_changed = true;
                    }
                    (
                        n,
                        m.args.video,
                        m.args.frames,
                        m.playhead,
                        scroll_target,
                        zoom_changed,
                    )
                };
                if let Some(target) = scroll_target {
                    ui.set_scroll_request_x(target);
                }
                if zoom_changed {
                    after_zoom(&ui, &model_for_tick);
                }
                refresh_clip_window(&ui, &model_for_tick, false);
                if frames > 0 && n as usize > frames + 2 {
                    let _ = slint::quit_event_loop();
                    return;
                }
                publish_playhead(&ui, &model_for_tick);
                if (playhead - 12.0).abs() < 0.01 {
                    ui.set_status(slint::SharedString::from(
                        if model_for_tick.borrow().playing {
                            "播放中 · GPU 预览"
                        } else {
                            "已暂停 · GPU 预览"
                        },
                    ));
                }
                if video != VideoMode::None
                    && let (Some((device, queue)), Some(f)) = (&gpu_for_tick, &frame_tex_for_tick)
                {
                    let p0 = Instant::now();
                    f.draw(device, queue, n);
                    let image = if video == VideoMode::Readback {
                        readback_image(device, queue, &f.texture).ok()
                    } else {
                        slint::Image::try_from(f.texture.clone()).ok()
                    };
                    if let Some(image) = image {
                        ui.set_preview(image.clone());
                        *preview_image.borrow_mut() = Some(image);
                    }
                    model_for_tick
                        .borrow_mut()
                        .metrics
                        .record_prepare(p0.elapsed().as_micros() as u64);
                }
            },
        );
    }

    // Timers must outlive the `if` block: Slint stops a timer when it is dropped.
    let shot_timer = slint::Timer::default();
    let idle_timer = slint::Timer::default();
    let embed_timer = slint::Timer::default();
    let embed_quit_timer = slint::Timer::default();

    // Snapshots must run from a timer: `take_snapshot()` renders again, and calling it inside the
    // rendering notifier would re-enter FemtoVG's borrowed notifier slot.
    if args.shot.is_some() {
        let weak = ui.as_weak();
        let model = model.clone();
        shot_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(120),
            move || {
                let Some(ui) = weak.upgrade() else { return };
                let w = ui.as_weak();
                maybe_snapshot(&w, &model);
                maybe_finish(&w, &model);
            },
        );
    }

    // Idle mode: one frame, then no timer-driven repaints. The external harness measures CPU.
    if args.idle_seconds > 0 {
        timer.stop();
        let weak = ui.as_weak();
        let model = model.clone();
        idle_timer.start(
            slint::TimerMode::SingleShot,
            Duration::from_secs(args.idle_seconds),
            move || {
                if let Some(_ui) = weak.upgrade() {
                    model.borrow_mut().snapshot_taken = true;
                    let _ = slint::quit_event_loop();
                }
            },
        );
    }

    // Native embedding: report the actual parent/child relationship after the first frame.
    #[cfg(windows)]
    if let Some(parent) = args.embed_parent {
        let weak = ui.as_weak();
        let model = model.clone();
        embed_timer.start(
            slint::TimerMode::SingleShot,
            Duration::from_millis(350),
            move || {
                if let Some(ui) = weak.upgrade() {
                    let top = if model.borrow().args.embed_host {
                        embed_host::header_height()
                    } else {
                        0
                    };
                    let report = embed::report(&ui, parent, top);
                    let path = model
                        .borrow()
                        .args
                        .result
                        .clone()
                        .unwrap_or_else(|| PathBuf::from("embed.json"));
                    let _ = std::fs::write(&path, report.to_string());
                    eprintln!("slint-poc: embedding report -> {}", path.display());
                }
            },
        );
        if args.embed_seconds > 0 {
            embed_quit_timer.start(
                slint::TimerMode::SingleShot,
                Duration::from_secs(args.embed_seconds),
                || {
                    let _ = slint::quit_event_loop();
                },
            );
        }
    }

    ui.run().expect("event loop");

    #[cfg(windows)]
    if let Some(hwnd) = host_hwnd {
        embed_host::destroy_host(hwnd);
    }

    let times = times.lock().map(|t| t.clone()).unwrap_or_default();
    let intervals = intervals.lock().map(|t| t.clone()).unwrap_or_default();
    let mut metrics = model.borrow().metrics.clone();
    metrics.frames = times;
    metrics.intervals = intervals;
    let m = model.borrow();
    let panel = (
        ui.get_left_frac(),
        ui.get_right_frac(),
        ui.get_bottom_frac(),
    );
    let report = metrics.report(
        "slint",
        serde_json::json!({
            "clips": m.scene.total_clips(),
            "duration_s": m.scene.duration,
            "zoom": m.zoom,
            "video": format!("{:?}", m.args.video),
            "interact": format!("{:?}", m.args.interact),
            "scenario": format!("{:?}", m.args.scenario),
            "renderer": if gpu.is_some() { "femtovg-wgpu" } else { "software" },
            "glyph_family": "Arcline CJK (embedded Noto Sans SC subset)",
            "scale_factor": ui.window().scale_factor(),
            "window_px": [ui.window().size().width, ui.window().size().height],
            "panel_fractions": [panel.0, panel.1, panel.2],
            "view_scroll": [ui.get_view_scroll_x(), ui.get_view_scroll_y()],
            "selected_id": m.selected_id,
            "selected_asset": m.selected_asset,
            "editable_label": ui.get_editable_label().to_string(),
            "ime_preedit": ui.get_ime_preedit().to_string(),
            "embedded": m.args.embed_parent.is_some(),
            "cull": m.args.cull,
            "ui_mode": if m.args.timeline_only { "timeline" } else { "full" },
        }),
    );
    let json = serde_json::to_string_pretty(&report).unwrap();
    let _ = std::fs::write(&m.args.out, &json);
    if let Some(result) = &m.args.result {
        let _ = std::fs::write(result, &json);
    }
    println!("{json}");
    std::process::exit(0);
}

fn after_zoom(ui: &AppWindow, model: &Rc<RefCell<Model>>) {
    let zoom = model.borrow().zoom;
    ui.set_zoom(zoom);
    ui.set_zoom_label(slint::SharedString::from(format!("{:.0} px/s", zoom)));
    publish_ticks(ui, model);
    let scroll = {
        let mut m = model.borrow_mut();
        m.last_zoom_publish = zoom;
        m.scroll_x = m.playhead * zoom;
        m.published_from = -1.0;
        m.published_to = -1.0;
        m.scroll_x
    };
    ui.set_scroll_request_x(scroll);
    refresh_clip_window(ui, model, true);
    ui.set_status(slint::SharedString::from(format!(
        "缩放 {:.0} px/s · {}",
        zoom,
        if zoom >= 120.0 {
            "帧刻度"
        } else {
            "秒/帧混合刻度"
        }
    )));
}

fn publish_playhead(ui: &AppWindow, model: &Rc<RefCell<Model>>) {
    let m = model.borrow();
    ui.set_playhead_x(m.playhead * m.zoom);
    ui.set_timecode(slint::SharedString::from(format_timecode(m.playhead, 30.0)));
    drop(m);
}

fn update_selection(ui: &AppWindow, model: &Rc<RefCell<Model>>) {
    let m = model.borrow();
    ui.set_selected_id(m.selected_id);
    if let Some((track, clip)) = m.selected_clip() {
        ui.set_selected_label(slint::SharedString::from(&clip.label));
        ui.set_editable_label(slint::SharedString::from(&clip.label));
        ui.set_selected_kind(slint::SharedString::from(format!(
            "{} · {} 轨",
            track.kind.name(),
            track.name
        )));
        ui.set_selected_track(slint::SharedString::from(&track.name));
        ui.set_selected_time(slint::SharedString::from(format_timecode(clip.start, 30.0)));
        ui.set_selected_source(slint::SharedString::from("demo / 合成帧"));
        ui.set_selected_position(slint::SharedString::from(format!("{:.2} s", clip.start)));
        ui.set_selected_duration(slint::SharedString::from(format!(
            "{:.2} s · {} 帧",
            clip.duration,
            (clip.duration * 30.0).round() as i64
        )));
        ui.set_status(slint::SharedString::from(format!("已选中 {}", clip.label)));
    } else {
        ui.set_selected_label(slint::SharedString::from("未选择素材"));
        ui.set_editable_label(slint::SharedString::from(""));
        ui.set_selected_kind(slint::SharedString::from("—"));
        ui.set_selected_track(slint::SharedString::from("—"));
        ui.set_selected_time(slint::SharedString::from("—"));
        ui.set_selected_source(slint::SharedString::from("—"));
        ui.set_selected_position(slint::SharedString::from("—"));
        ui.set_selected_duration(slint::SharedString::from("—"));
    }
}

fn publish_scene(ui: &AppWindow, model: &Rc<RefCell<Model>>) {
    let m = model.borrow();
    let mut tracks = Vec::new();
    for t in &m.scene.tracks {
        tracks.push(TrackView {
            id: t.id as i32,
            name: slint::SharedString::from(&t.name),
            kind: slint::SharedString::from(t.kind.name()),
            top: t.top,
            height: t.kind.lane_height(),
            muted: t.muted,
            locked: t.locked,
            color: color_for(t.kind),
            clip_count: t.clips.len() as i32,
        });
    }
    let mut clips = Vec::new();
    for track in &m.scene.tracks {
        for c in &track.clips {
            let keys = c
                .keys
                .iter()
                .map(|k| KeyView {
                    x: (k.at / c.duration.max(0.001)).clamp(0.0, 1.0),
                    y: 6.0 + k.value * (track.kind.lane_height() - 12.0).max(1.0),
                })
                .collect::<Vec<_>>();
            clips.push(ClipView {
                id: c.id as i32,
                start: c.start,
                duration: c.duration,
                lane_y: track.top,
                lane_h: track.kind.lane_height(),
                label: slint::SharedString::from(&c.label),
                fill: slint::Color::from_rgb_u8(c.color[0], c.color[1], c.color[2]),
                has_thumb: c.thumb,
                wave_path: slint::SharedString::from(wave_path(&c.wave)),
                keys: slint::ModelRc::new(slint::VecModel::from(keys)),
            });
        }
    }
    ui.set_tracks(slint::ModelRc::new(slint::VecModel::from(tracks)));
    ui.set_clips(slint::ModelRc::new(slint::VecModel::from(clips)));
}

/// Publish only the clips that can be on screen. The caller keeps the previous window while the
/// user scrolls; a new model is built only when the viewport leaves the published range. This is
/// the difference between a retained scene graph and a scene graph that scales to thousands of
/// clips.
fn refresh_clip_window(ui: &AppWindow, model: &Rc<RefCell<Model>>, force: bool) {
    let (cull, visible_from, visible_to, published_from, published_to, from, to) = {
        let m = model.borrow();
        let scroll = ui.get_view_scroll_x().max(0.0);
        let viewport = (ui.window().size().width as f32 / ui.window().scale_factor().max(0.5)
            - 132.0)
            .max(200.0);
        let visible_from = scroll / m.zoom;
        let visible_to = visible_from + viewport / m.zoom;
        // A generous margin means the model is rebuilt every ~20 s of timeline travel, not every
        // frame. The visible clip positions stay stable between rebuilds because they are global
        // timeline coordinates, not viewport-relative ones.
        let margin = 30.0;
        let from = (visible_from - margin).max(0.0);
        let to = (visible_to + margin).min(m.duration());
        (
            m.args.cull,
            visible_from,
            visible_to,
            m.published_from,
            m.published_to,
            from,
            to,
        )
    };
    if !cull {
        return;
    }
    let inside = visible_from > published_from + 8.0 && visible_to < published_to - 8.0;
    if !force && inside {
        return;
    }
    publish_clips_window(ui, model, from, to);
    let mut m = model.borrow_mut();
    m.published_from = from;
    m.published_to = to;
}

fn publish_clips_window(ui: &AppWindow, model: &Rc<RefCell<Model>>, from: f32, to: f32) {
    let m = model.borrow();
    let mut clips = Vec::new();
    for track in &m.scene.tracks {
        for c in &track.clips {
            if c.end() < from || c.start > to {
                continue;
            }
            let keys = c
                .keys
                .iter()
                .map(|k| KeyView {
                    x: (k.at / c.duration.max(0.001)).clamp(0.0, 1.0),
                    y: 6.0 + k.value * (track.kind.lane_height() - 12.0).max(1.0),
                })
                .collect::<Vec<_>>();
            clips.push(ClipView {
                id: c.id as i32,
                start: c.start,
                duration: c.duration,
                lane_y: track.top,
                lane_h: track.kind.lane_height(),
                label: slint::SharedString::from(&c.label),
                fill: slint::Color::from_rgb_u8(c.color[0], c.color[1], c.color[2]),
                has_thumb: c.thumb,
                wave_path: slint::SharedString::from(wave_path(&c.wave)),
                keys: slint::ModelRc::new(slint::VecModel::from(keys)),
            });
        }
    }
    ui.set_clips(slint::ModelRc::new(slint::VecModel::from(clips)));
}

fn publish_tracks(ui: &AppWindow, model: &Rc<RefCell<Model>>) {
    let m = model.borrow();
    let tracks = m
        .scene
        .tracks
        .iter()
        .map(|t| TrackView {
            id: t.id as i32,
            name: slint::SharedString::from(&t.name),
            kind: slint::SharedString::from(t.kind.name()),
            top: t.top,
            height: t.kind.lane_height(),
            muted: t.muted,
            locked: t.locked,
            color: color_for(t.kind),
            clip_count: t.clips.len() as i32,
        })
        .collect::<Vec<_>>();
    ui.set_tracks(slint::ModelRc::new(slint::VecModel::from(tracks)));
}

fn publish_ticks(ui: &AppWindow, model: &Rc<RefCell<Model>>) {
    let m = model.borrow();
    let fps = 30.0f32;
    let zoom = m.zoom;
    let duration = m.duration();
    // A tick every ~78 px; sub-frame ticks only when a frame is at least 4 px wide.
    let mut step_frames = (78.0 / zoom * fps).ceil().max(1.0) as i64;
    let candidates = [1, 2, 5, 10, 15, 30, 60, 150, 300, 600, 1800, 3600];
    step_frames = *candidates
        .iter()
        .find(|&&c| c >= step_frames)
        .unwrap_or(&3600);
    let total_frames = (duration * fps).ceil() as i64;
    let subs = if zoom >= 120.0 && total_frames < 3600 {
        (step_frames / 5).max(1)
    } else {
        step_frames
    };
    let mut ticks = Vec::new();
    let mut f = 0i64;
    while f <= total_frames && ticks.len() < 700 {
        let major = f % step_frames == 0;
        let label = if major {
            let secs = f / 30;
            let frame = f % 30;
            if frame == 0 {
                format!("{:02}:{:02}", secs / 60, secs % 60)
            } else {
                format!("{:02}f", frame)
            }
        } else {
            String::new()
        };
        ticks.push(TickView {
            x: f as f32 / fps * zoom,
            label: slint::SharedString::from(label),
            major,
        });
        f += subs.max(1);
    }
    ui.set_ticks(slint::ModelRc::new(slint::VecModel::from(ticks)));
}

fn media_items() -> Vec<MediaView> {
    vec![
        MediaView {
            name: slint::SharedString::from("城市街景.mp4"),
            kind: slint::SharedString::from("视频"),
            duration: slint::SharedString::from("00:10 · 1080p30"),
            detail: slint::SharedString::from("H.264 · 24.6 Mbps"),
            color: slint::Color::from_rgb_u8(0x2e, 0x5b, 0x8a),
        },
        MediaView {
            name: slint::SharedString::from("市场人流.mp4"),
            kind: slint::SharedString::from("视频"),
            duration: slint::SharedString::from("00:07 · 1080p30"),
            detail: slint::SharedString::from("H.264 · 18.2 Mbps"),
            color: slint::Color::from_rgb_u8(0x3d, 0x6b, 0x4f),
        },
        MediaView {
            name: slint::SharedString::from("山海湖.png"),
            kind: slint::SharedString::from("图片"),
            duration: slint::SharedString::from("静帧 · 3840×2160"),
            detail: slint::SharedString::from("PNG · 8 bit"),
            color: slint::Color::from_rgb_u8(0x7a, 0x4a, 0x3a),
        },
        MediaView {
            name: slint::SharedString::from("旁白_A01.wav"),
            kind: slint::SharedString::from("音频"),
            duration: slint::SharedString::from("00:32 · 48 kHz"),
            detail: slint::SharedString::from("WAV · 24 bit"),
            color: slint::Color::from_rgb_u8(0x5a, 0x46, 0x7a),
        },
        MediaView {
            name: slint::SharedString::from("钢琴主题.wav"),
            kind: slint::SharedString::from("音乐"),
            duration: slint::SharedString::from("01:12 · 48 kHz"),
            detail: slint::SharedString::from("WAV · 24 bit"),
            color: slint::Color::from_rgb_u8(0x6b, 0x5f, 0x2e),
        },
        MediaView {
            name: slint::SharedString::from("海鸥_环境.wav"),
            kind: slint::SharedString::from("音效"),
            duration: slint::SharedString::from("00:18 · 48 kHz"),
            detail: slint::SharedString::from("WAV · 16 bit"),
            color: slint::Color::from_rgb_u8(0x3a, 0x6a, 0x72),
        },
    ]
}

fn color_for(kind: scene::Kind) -> slint::Color {
    match kind {
        scene::Kind::Video => slint::Color::from_rgb_u8(0x2e, 0x5b, 0x8a),
        scene::Kind::Overlay => slint::Color::from_rgb_u8(0x3d, 0x6b, 0x4f),
        scene::Kind::Title => slint::Color::from_rgb_u8(0x7a, 0x4a, 0x3a),
        scene::Kind::Audio => slint::Color::from_rgb_u8(0x5a, 0x46, 0x7a),
        scene::Kind::Music => slint::Color::from_rgb_u8(0x6b, 0x5f, 0x2e),
        scene::Kind::Sfx => slint::Color::from_rgb_u8(0x3a, 0x6a, 0x72),
    }
}

fn wave_path(wave: &[f32]) -> String {
    if wave.is_empty() {
        return String::new();
    }
    let mid = 50.0;
    let mut path = format!("M 0 {mid:.1}");
    for (i, v) in wave.iter().enumerate() {
        let x = (i as f32 + 0.5) / wave.len() as f32 * 100.0;
        let y = mid - v.clamp(-1.0, 1.0) * 42.0;
        path.push_str(&format!(" L {x:.1} {y:.1}"));
    }
    for (i, v) in wave.iter().enumerate().rev() {
        let x = (i as f32 + 0.5) / wave.len() as f32 * 100.0;
        let y = mid + v.clamp(-1.0, 1.0) * 42.0;
        path.push_str(&format!(" L {x:.1} {y:.1}"));
    }
    path.push_str(" Z");
    path
}

fn format_scale(scale: f32) -> String {
    format!("{:.0}% · {:.2}×", scale * 100.0, scale)
}

fn format_timecode(seconds: f32, fps: f32) -> String {
    let total = (seconds.max(0.0) * fps).round() as i64;
    let f = total % fps as i64;
    let s = total / fps as i64;
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        s / 3600,
        (s / 60) % 60,
        s % 60,
        f
    )
}

fn maybe_snapshot(ui: &slint::Weak<AppWindow>, model: &Rc<RefCell<Model>>) {
    let (want, delay) = {
        let m = model.borrow();
        let want = m.args.shot.clone().filter(|_| !m.snapshot_taken);
        let Some(first) = m.first_frame else { return };
        (want, first.elapsed())
    };
    let Some(path) = want else { return };
    if delay < Duration::from_millis(model.borrow().args.shot_delay_ms) {
        return;
    }
    if let Some(ui) = ui.upgrade() {
        model.borrow_mut().snapshot_taken = true;
        match ui.window().take_snapshot() {
            Ok(buffer) => match write_png(&path, &buffer) {
                Ok(()) => eprintln!(
                    "slint-poc: snapshot {}x{} -> {}",
                    buffer.width(),
                    buffer.height(),
                    path.display()
                ),
                Err(e) => eprintln!("slint-poc: snapshot write failed: {e}"),
            },
            Err(e) => eprintln!("slint-poc: snapshot failed: {e}"),
        }
        if model.borrow().args.frames == 0 && model.borrow().args.embed_parent.is_none() {
            let _ = slint::quit_event_loop();
        }
    }
}

fn maybe_finish(ui: &slint::Weak<AppWindow>, model: &Rc<RefCell<Model>>) {
    let m = model.borrow();
    if m.args.frames == 0 && m.args.shot.is_some() && m.snapshot_taken {
        drop(m);
        if ui.upgrade().is_some() {
            let _ = slint::quit_event_loop();
        }
    }
}

fn write_png(
    path: &std::path::Path,
    buffer: &slint::SharedPixelBuffer<slint::Rgba8Pixel>,
) -> Result<(), Box<dyn std::error::Error>> {
    let w = buffer.width();
    let h = buffer.height();
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for px in buffer.as_slice() {
        rgba.extend_from_slice(&[px.r, px.g, px.b, px.a]);
    }
    let file = File::create(path)?;
    let wtr = BufWriter::new(file);
    let mut encoder = png::Encoder::new(wtr, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&rgba)?;
    Ok(())
}

fn readback_image(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    tex: &wgpu::Texture,
) -> Result<slint::Image, String> {
    let padded = (VIDEO_W * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("slint-readback"),
        size: (padded as u64) * VIDEO_H as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
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
    queue.submit([enc.finish()]);
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| e.to_string())?;
    rx.recv()
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let mut rgba = Vec::with_capacity((VIDEO_W * VIDEO_H * 4) as usize);
    {
        let data = slice.get_mapped_range().map_err(|e| e.to_string())?;
        for row in 0..VIDEO_H {
            let start = (row * padded) as usize;
            rgba.extend_from_slice(&data[start..start + (VIDEO_W * 4) as usize]);
        }
    }
    buffer.unmap();
    let pixels =
        slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&rgba, VIDEO_W, VIDEO_H);
    Ok(slint::Image::from_rgba8(pixels))
}

#[cfg(windows)]
mod embed {
    use super::*;
    use raw_window_handle::{RawWindowHandle, Win32WindowHandle};
    use slint::winit_030::WinitWindowAccessor;
    use std::num::NonZeroIsize;
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_STYLE, GetClientRect, GetParent, GetWindowLongPtrW, SW_SHOW, SWP_FRAMECHANGED,
        SWP_SHOWWINDOW, SetParent, SetWindowLongPtrW, SetWindowPos, ShowWindow, WS_CAPTION,
        WS_CHILD, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME, WS_VISIBLE,
    };

    pub fn child_window_attributes(
        attrs: slint::winit_030::winit::window::WindowAttributes,
        parent: isize,
        width: u32,
        height: u32,
        top: i32,
    ) -> slint::winit_030::winit::window::WindowAttributes {
        use slint::winit_030::winit::dpi::{PhysicalPosition, PhysicalSize};
        let attrs = attrs
            .with_decorations(false)
            .with_resizable(false)
            .with_inner_size(PhysicalSize::new(width, height))
            .with_position(PhysicalPosition::new(0, top));
        let mut handle = Win32WindowHandle::new(
            NonZeroIsize::new(parent).expect("embed parent must not be zero"),
        );
        handle.hinstance = None;
        // SAFETY: the caller passes a live top-level HWND from the Agent host window.
        unsafe { attrs.with_parent_window(Some(RawWindowHandle::Win32(handle))) }
    }

    pub fn report(ui: &AppWindow, parent: isize, top: i32) -> serde_json::Value {
        use raw_window_handle::HasWindowHandle;
        let child = ui
            .window()
            .with_winit_window(|w| {
                w.window_handle().ok().and_then(|h| match h.as_raw() {
                    RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
                    _ => None,
                })
            })
            .flatten()
            .unwrap_or(0);
        let mut actual_parent = unsafe { GetParent(child as _) } as isize;
        // winit's `with_parent_window` is documented for a parent in the same process. For a
        // foreign Agent-client HWND, fall back to the classic Win32 reparenting sequence. In both
        // cases, strip the caption after creation: a child window must not look like a top-level.
        if actual_parent != parent && child != 0 {
            unsafe {
                SetParent(child as _, parent as _);
            }
        }
        if child != 0 && parent != 0 {
            let mut rect = RECT::default();
            unsafe {
                GetClientRect(parent as _, &mut rect);
                let style = GetWindowLongPtrW(child as _, GWL_STYLE);
                let chrome = (WS_POPUP
                    | WS_CAPTION
                    | WS_THICKFRAME
                    | WS_SYSMENU
                    | WS_MINIMIZEBOX
                    | WS_MAXIMIZEBOX) as isize;
                let style = (style & !chrome) | WS_CHILD as isize | WS_VISIBLE as isize;
                SetWindowLongPtrW(child as _, GWL_STYLE, style);
                let scale = ui.window().scale_factor().max(0.5);
                ui.window().set_size(slint::LogicalSize::new(
                    rect.right as f32 / scale,
                    (rect.bottom - top).max(1) as f32 / scale,
                ));
                SetWindowPos(
                    child as _,
                    std::ptr::null_mut(),
                    0,
                    top,
                    rect.right,
                    (rect.bottom - top).max(1),
                    SWP_FRAMECHANGED | SWP_SHOWWINDOW,
                );
                ShowWindow(child as _, SW_SHOW);
                actual_parent = GetParent(child as _) as isize;
            }
        }
        serde_json::json!({
            "standalone": false,
            "parent_hwnd": format!("0x{parent:x}"),
            "child_hwnd": format!("0x{child:x}"),
            "get_parent_hwnd": format!("0x{actual_parent:x}"),
            "is_child_of_host": actual_parent == parent,
            "size": [ui.window().size().width, ui.window().size().height],
            "scale_factor": ui.window().scale_factor(),
            "renderer": "femtovg-wgpu",
        })
    }
}
