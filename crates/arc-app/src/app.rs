//! Host loop on `egui-winit` + `egui-wgpu` (docs/13 §2.1, ADR-0007 amendment).
//!
//! One loop for both modes: a top-level window, or a child window of a host-supplied `HWND`.
//! If embedding fails or the parent disappears, the app pops out into a top-level window in the
//! same process, so the session survives (docs/13 §2.5).

use crate::embed::{self, AppMsg, HostMsg};
use crate::shell::Shell;
use crate::poc::Workload;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::window::{Window, WindowId};

#[derive(Debug)]
pub enum UserEvent {
    Host(HostMsg),
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Host window to embed into (`--embed-parent`).
    pub parent: Option<isize>,
    /// Embed channel over stdin/stdout (`--embed-channel stdio`).
    pub embed_stdio: bool,
    /// PoC: report stats and exit after this many seconds (`--poc-seconds`).
    pub poc_seconds: Option<f64>,
    /// Editor mode: open this project and show the viewer/transport (`--project`).
    pub project: Option<std::path::PathBuf>,
}

/// Longest `poc.hang` request honoured by a PoC session.
const POC_HANG_MAX_MS: u64 = 5_000;

/// PoC-only messages are accepted only in a `--poc-seconds` session (audit F-31).
fn poc_enabled(poc_seconds: Option<f64>) -> bool {
    poc_seconds.is_some()
}

struct Gfx {
    window: Arc<Window>,
    state: egui_winit::State,
    painter: egui_wgpu::winit::Painter,
    ctx: egui::Context,
}

pub struct App {
    opts: Options,
    gfx: Option<Gfx>,
    workload: Workload,
    /// Four-panel editor shell (media / preview / inspector / timeline).
    shell: Shell,
    /// Parent currently in use; cleared by pop-out.
    parent: Option<isize>,
    started: Instant,
    popped_out: Option<String>,
    exit_requested: bool,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("parent", &self.parent)
            .field("popped_out", &self.popped_out)
            .finish()
    }
}

impl App {
    pub fn new(opts: Options) -> App {
        App {
            parent: opts.parent,
            opts,
            gfx: None,
            workload: Workload::new(),
            shell: Shell::default(),
            started: Instant::now(),
            popped_out: None,
            exit_requested: false,
        }
    }

    fn create_window(&mut self, el: &ActiveEventLoop) -> Result<(), String> {
        let title = self
            .opts
            .project
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| format!("Arcline — {}", s.to_string_lossy()))
            .unwrap_or_else(|| "Arcline".to_string());
        let mut attrs = Window::default_attributes()
            .with_title(title)
            .with_inner_size(PhysicalSize::new(1280u32, 720u32));
        if let Some(hwnd) = self.parent {
            let handle =
                embed::parent_handle(hwnd).ok_or("embedding is not supported on this platform")?;
            if !embed::is_window(hwnd) {
                return Err(format!("parent window {hwnd:#x} does not exist"));
            }
            attrs = attrs
                .with_decorations(false)
                .with_position(PhysicalPosition::new(0, 0));
            // SAFETY: the handle was checked with IsWindow above; winit only uses it as the
            // parent for CreateWindowEx. If the parent later dies, the watchdog pops out.
            #[allow(unsafe_code)]
            {
                attrs = unsafe { attrs.with_parent_window(Some(handle)) };
            }
        }
        let window = Arc::new(el.create_window(attrs).map_err(|e| e.to_string())?);
        let ctx = egui::Context::default();
        let mut painter = pollster::block_on(egui_wgpu::winit::Painter::new(
            ctx.clone(),
            egui_wgpu::WgpuConfiguration::default(),
            false,
            egui_wgpu::RendererOptions::default(),
        ));
        pollster::block_on(painter.set_window(egui::ViewportId::ROOT, Some(window.clone())))
            .map_err(|e| e.to_string())?;
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            painter.max_texture_side(),
        );
        self.workload = Workload::new();
        let hwnd = embed::hwnd_of(&window);
        self.gfx = Some(Gfx {
            window,
            state,
            painter,
            ctx,
        });
        if self.opts.embed_stdio {
            embed::send(&AppMsg::Ready {
                hwnd,
                pid: std::process::id(),
            });
        }
        Ok(())
    }

    /// Leave the host and continue in a top-level window (docs/13 §2.5).
    fn pop_out(&mut self, el: &ActiveEventLoop, reason: &str) {
        if self.parent.is_none() {
            return;
        }
        eprintln!("arcline: leaving embedded mode: {reason}");
        self.parent = None;
        self.popped_out = Some(reason.to_string());
        if let Some(mut g) = self.gfx.take() {
            g.painter.destroy();
        }
        if let Err(e) = self.create_window(el) {
            eprintln!("arcline: could not open a top-level window: {e}");
            el.exit();
        }
    }

    fn render(&mut self) {
        let embedded = self.parent.is_some();
        let Some(g) = self.gfx.as_mut() else { return };
        let input = g.state.take_egui_input(&g.window);
        let size = g.window.inner_size();
        let live = format!(
            "{} · {}×{} · {:.0} s",
            if embedded {
                "embedded"
            } else if self.popped_out.is_some() {
                "popped out"
            } else {
                "top-level"
            },
            size.width,
            size.height,
            self.started.elapsed().as_secs_f32()
        );
        let poc_mode =
            self.opts.poc_seconds.is_some() || self.opts.parent.is_some() || self.opts.embed_stdio;
        let mut out = if poc_mode {
            let workload = &mut self.workload;
            g.ctx.run_ui(input, |ui| {
                workload.ui(ui, "Arcline - embedding PoC", &live)
            })
        } else {
            let shell = &mut self.shell;
            g.ctx.run_ui(input, |ui| shell.ui(ui))
        };
        g.state
            .handle_platform_output(&g.window, out.platform_output);
        let clipped = g.ctx.tessellate(out.shapes, out.pixels_per_point);
        g.painter.paint_and_update_textures(
            egui::ViewportId::ROOT,
            out.pixels_per_point,
            [0.08, 0.08, 0.09, 1.0],
            &clipped,
            &mut out.textures_delta,
            Vec::new(),
            &g.window,
        );
        self.workload.tick();
        g.window.request_redraw();
    }

    fn stats(&self) -> crate::poc::FrameStats {
        let size = self
            .gfx
            .as_ref()
            .map(|g| g.window.inner_size())
            .unwrap_or_default();
        self.workload
            .stats(self.parent.is_some(), (size.width, size.height))
    }

    fn on_host(&mut self, el: &ActiveEventLoop, m: HostMsg) {
        match m {
            HostMsg::Attach { bounds, .. } | HostMsg::Resize { bounds, .. } => {
                if let Some(g) = &self.gfx {
                    let [x, y, w, h] = bounds;
                    g.window.set_outer_position(PhysicalPosition::new(x, y));
                    let _ = g
                        .window
                        .request_inner_size(PhysicalSize::new(w.max(1) as u32, h.max(1) as u32));
                }
            }
            HostMsg::Visibility { visible } => {
                if let Some(g) = &self.gfx {
                    g.window.set_visible(visible);
                }
            }
            HostMsg::Focus { grant } => {
                if grant {
                    if let Some(g) = &self.gfx {
                        g.window.focus_window();
                    }
                }
            }
            HostMsg::Ping { seq } => embed::send(&AppMsg::Pong { seq }),
            HostMsg::Detach { reason } => {
                if self.opts.embed_stdio
                    && reason == "channel closed"
                    && self.opts.poc_seconds.is_some()
                {
                    self.exit_requested = true;
                    el.exit();
                } else {
                    self.pop_out(el, &reason);
                }
            }
            // PoC messages are hung-watchdog test seams: a product binary must not let the host
            // block its UI thread on request, so they only work with --poc-seconds (audit F-31).
            HostMsg::PocHang { ms } => {
                if poc_enabled(self.opts.poc_seconds) {
                    std::thread::sleep(Duration::from_millis(ms.min(POC_HANG_MAX_MS)));
                } else {
                    eprintln!("arcline: ignoring poc.hang (not a PoC session; use --poc-seconds)");
                }
            }
            HostMsg::PocStats => {
                if poc_enabled(self.opts.poc_seconds) {
                    embed::send(&AppMsg::Stats(self.stats()));
                } else {
                    eprintln!("arcline: ignoring poc.stats (not a PoC session; use --poc-seconds)");
                }
            }
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        el.set_control_flow(ControlFlow::Wait);
        if let Err(e) = self.create_window(el) {
            if self.parent.is_some() {
                // embedding failed: fall back to a top-level window (L3 behaviour)
                eprintln!("arcline: embedding failed ({e}); opening a top-level window");
                self.parent = None;
                self.popped_out = Some(e);
                if let Err(e2) = self.create_window(el) {
                    eprintln!("arcline: {e2}");
                    el.exit();
                }
            } else {
                eprintln!("arcline: {e}");
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(g) = self.gfx.as_mut() else { return };
        let response = g.state.on_window_event(&g.window, &event);
        match event {
            WindowEvent::CloseRequested => {
                self.exit_requested = true;
                el.exit();
            }
            WindowEvent::Resized(size) => {
                if let (Some(w), Some(h)) =
                    (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
                {
                    g.painter.on_window_resized(egui::ViewportId::ROOT, w, h);
                }
            }
            WindowEvent::RedrawRequested => self.render(),
            _ => {
                if response.repaint {
                    g.window.request_redraw();
                }
            }
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Host(m) => self.on_host(el, m),
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if let Some(p) = self.parent {
            if !embed::is_window(p) {
                self.pop_out(el, "parent window disappeared");
            }
        }
        if let Some(secs) = self.opts.poc_seconds {
            if self.started.elapsed().as_secs_f64() >= secs && !self.exit_requested {
                if self.opts.embed_stdio {
                    embed::send(&AppMsg::Stats(self.stats()));
                } else {
                    println!(
                        "{}",
                        serde_json::to_string(&self.stats()).unwrap_or_default()
                    );
                }
                self.exit_requested = true;
                el.exit();
            }
        }
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        if let Some(mut g) = self.gfx.take() {
            g.painter.destroy();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F-31: PoC embed messages must be gated on a PoC session.
    #[test]
    fn poc_messages_are_gated_on_a_poc_session() {
        assert!(!poc_enabled(None));
        assert!(poc_enabled(Some(2.0)));
        assert_eq!(POC_HANG_MAX_MS, 5_000);
    }
}