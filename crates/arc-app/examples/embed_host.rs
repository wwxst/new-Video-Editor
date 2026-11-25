//! Reference host for full-editor embedding (docs/13 §2.3–2.4, ADR-0014).
//!
//! A separate process that reserves a region of its own window, launches `arcline` as a native
//! child window in it, drives the embed channel, and measures the PoC go/no-go criteria:
//!
//! 1. embedded frame pacing vs a top-level baseline;
//! 2. parent/child relationship and resize following;
//! 3. a hung child (watchdog detection; host stays responsive);
//! 4. a crashed child (host survives; child window gone).
//!
//! Run: `cargo run -p arc-app --release --example embed_host` → JSON report on stdout,
//! exit code 0 = go, 3 = no-go.
#![allow(unsafe_code)]

use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Window, WindowId};

const CONVERSATION_PANE: i32 = 240; // host-owned UI left of the editor region
const PACING_SECS: f64 = 8.0;
const HANG_MS: u64 = 3000;
const WATCHDOG: Duration = Duration::from_secs(2);

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type")]
enum ChildMsg {
    #[serde(rename = "embed.ready")]
    Ready { hwnd: isize, pid: u32 },
    #[serde(rename = "embed.pong")]
    Pong { seq: u64 },
    #[serde(rename = "poc.stats")]
    Stats {
        #[serde(flatten)]
        stats: Value,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug)]
enum Ev {
    Child(ChildMsg),
    ChildExited,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Start,
    Embedded,
    Hang,
    Crash,
    Baseline,
    Done,
}

struct Host {
    proxy: EventLoopProxy<Ev>,
    window: Option<Arc<Window>>,
    hwnd: isize,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    child_hwnd: isize,
    phase: Phase,
    phase_start: Instant,
    seq: u64,
    sent: std::collections::HashMap<u64, Instant>,
    rtts_ms: Vec<f64>,
    last_pong: Instant,
    last_tick: Instant,
    max_gap_ms: f64,
    report: serde_json::Map<String, Value>,
    resized: bool,
    stats_requested: bool,
    hang_detect_ms: Option<f64>,
    resize_call_ms: Option<f64>,
    crash_checked: bool,
    next_ping: Instant,
}

fn arcline_exe() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("ARCLINE_BIN") {
        return p.into();
    }
    let exe = std::env::current_exe().unwrap_or_default();
    // target/<profile>/examples/embed_host.exe → target/<profile>/arcline.exe
    let dir = exe
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    dir.join(if cfg!(windows) {
        "arcline.exe"
    } else {
        "arcline"
    })
}

fn hwnd_of(w: &Window) -> isize {
    match w.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
        _ => 0,
    }
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetParent, IsWindow, IsWindowVisible};
    pub fn parent(h: isize) -> isize {
        // SAFETY: GetParent accepts any handle value and returns 0 on failure.
        unsafe { GetParent(h as _) as isize }
    }
    pub fn exists(h: isize) -> bool {
        // SAFETY: IsWindow accepts any handle value.
        unsafe { IsWindow(h as _) != 0 }
    }
    pub fn visible(h: isize) -> bool {
        // SAFETY: IsWindowVisible accepts any handle value.
        unsafe { IsWindowVisible(h as _) != 0 }
    }
}

impl Host {
    fn send(&mut self, v: Value) {
        if let Some(s) = self.stdin.as_mut() {
            let _ = writeln!(s, "{v}");
            let _ = s.flush();
        }
    }

    fn spawn(&mut self, embedded: bool) {
        let mut cmd = Command::new(arcline_exe());
        if embedded {
            cmd.args(["--embed-parent", &self.hwnd.to_string()]);
        } else {
            cmd.args(["--poc-seconds", &PACING_SECS.to_string()]);
        }
        cmd.args(["--embed-channel", "stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("embed_host: cannot start {}: {e}", arcline_exe().display());
                std::process::exit(2);
            }
        };
        let stdout = child.stdout.take();
        self.stdin = child.stdin.take();
        self.child = Some(child);
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            if let Some(out) = stdout {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    if let Ok(m) = serde_json::from_str::<ChildMsg>(&line) {
                        let _ = proxy.send_event(Ev::Child(m));
                    }
                }
            }
            let _ = proxy.send_event(Ev::ChildExited);
        });
        self.last_pong = Instant::now();
    }

    fn editor_bounds(&self) -> [i32; 4] {
        let s = self
            .window
            .as_ref()
            .map(|w| w.inner_size())
            .unwrap_or(PhysicalSize::new(1280, 800));
        [
            CONVERSATION_PANE,
            0,
            s.width as i32 - CONVERSATION_PANE,
            s.height as i32,
        ]
    }

    fn enter(&mut self, p: Phase) {
        self.phase = p;
        self.phase_start = Instant::now();
        self.max_gap_ms = 0.0;
    }

    fn ping(&mut self) {
        self.seq += 1;
        self.sent.insert(self.seq, Instant::now());
        let seq = self.seq;
        self.send(json!({"type": "embed.ping", "seq": seq}));
    }

    fn finish(&mut self, el: &ActiveEventLoop) {
        let r = &self.report;
        let get = |k: &str, f: &str| r.get(k).and_then(|v| v.get(f)).and_then(Value::as_f64);
        let (ep95, tp95) = (get("embedded", "p95_ms"), get("toplevel", "p95_ms"));
        let (eh, ef) = (get("embedded", "hitches"), get("embedded", "frames"));
        let mut checks = serde_json::Map::new();
        let pacing_ok = matches!((ep95, tp95), (Some(e), Some(t)) if e <= t * 1.10 + 0.5);
        let hitch_ok = matches!((eh, ef), (Some(h), Some(f)) if f > 0.0 && h / f <= 0.01);
        checks.insert("frame_pacing_matches_toplevel".into(), json!(pacing_ok));
        checks.insert("hitches_le_1pct".into(), json!(hitch_ok));
        checks.insert(
            "child_parented".into(),
            r.get("child_parented").cloned().unwrap_or(json!(false)),
        );
        checks.insert(
            "resize_followed".into(),
            r.get("resize_followed").cloned().unwrap_or(json!(false)),
        );
        let hang_host = r
            .get("hang")
            .and_then(|h| h.get("host_max_loop_gap_ms"))
            .and_then(Value::as_f64);
        let hang_det = r
            .get("hang")
            .and_then(|h| h.get("watchdog_detect_ms"))
            .and_then(Value::as_f64);
        checks.insert(
            "host_responsive_during_child_hang".into(),
            json!(hang_host.is_some_and(|g| g <= 250.0)),
        );
        checks.insert(
            "watchdog_detects_hang_le_3s".into(),
            json!(hang_det.is_some_and(|d| d <= 3000.0)),
        );
        let crash_ok = r
            .get("crash")
            .and_then(|c| c.get("host_survived"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
            && r.get("crash")
                .and_then(|c| c.get("child_window_gone"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
        checks.insert("host_survives_child_crash".into(), json!(crash_ok));
        let go = checks.values().all(|v| v.as_bool() == Some(true));
        self.report.insert("checks".into(), Value::Object(checks));
        self.report.insert("go".into(), json!(go));
        self.report.insert(
            "not_automated".into(),
            json!([
                "IME candidate placement",
                "keyboard focus round-trip",
                "DPI change across monitors",
                "Electron host"
            ]),
        );
        println!(
            "{}",
            serde_json::to_string_pretty(&Value::Object(self.report.clone())).unwrap_or_default()
        );
        self.phase = Phase::Done;
        el.exit();
        std::process::exit(if go { 0 } else { 3 });
    }
}

impl ApplicationHandler<Ev> for Host {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let w = el
            .create_window(
                Window::default_attributes()
                    .with_title("Arcline reference host")
                    .with_inner_size(PhysicalSize::new(1280u32, 800u32)),
            )
            .unwrap_or_else(|e| {
                eprintln!("embed_host: {e}");
                std::process::exit(2)
            });
        self.hwnd = hwnd_of(&w);
        self.window = Some(Arc::new(w));
        self.spawn(true);
        self.enter(Phase::Start);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(_) if self.child_hwnd != 0 && self.phase == Phase::Embedded => {
                let b = self.editor_bounds();
                self.send(json!({"type": "embed.resize", "bounds": b}));
            }
            _ => {}
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, ev: Ev) {
        match ev {
            Ev::Child(ChildMsg::Ready { hwnd, pid }) => {
                self.child_hwnd = hwnd;
                if self.phase == Phase::Start {
                    let ready_ms = self.phase_start.elapsed().as_secs_f64() * 1000.0;
                    #[cfg(windows)]
                    {
                        self.report.insert(
                            "child_parented".into(),
                            json!(win::parent(hwnd) == self.hwnd),
                        );
                        self.report
                            .insert("child_visible".into(), json!(win::visible(hwnd)));
                    }
                    self.report.insert("ready_ms".into(), json!(ready_ms));
                    self.report.insert("child_pid".into(), json!(pid));
                    let b = self.editor_bounds();
                    self.send(json!({"type": "embed.attach", "bounds": b}));
                    self.enter(Phase::Embedded);
                }
            }
            Ev::Child(ChildMsg::Pong { seq }) => {
                self.last_pong = Instant::now();
                if let Some(t) = self.sent.remove(&seq) {
                    if self.phase == Phase::Embedded {
                        self.rtts_ms.push(t.elapsed().as_secs_f64() * 1000.0);
                    }
                }
            }
            Ev::Child(ChildMsg::Stats { stats }) => match self.phase {
                Phase::Embedded => {
                    self.report.insert("embedded".into(), stats);
                    let mut r = std::mem::take(&mut self.rtts_ms);
                    r.sort_by(|a, b| a.total_cmp(b));
                    if !r.is_empty() {
                        self.report.insert(
                            "ping_rtt_ms".into(),
                            json!({"p50": r[r.len() / 2], "max": r[r.len() - 1], "n": r.len()}),
                        );
                    }
                    self.send(json!({"type": "poc.hang", "ms": HANG_MS}));
                    self.enter(Phase::Hang);
                    self.last_pong = Instant::now();
                }
                Phase::Baseline => {
                    self.report.insert("toplevel".into(), stats);
                    self.finish(el);
                }
                _ => {}
            },
            Ev::Child(ChildMsg::Other) => {}
            Ev::ChildExited => {
                if self.phase == Phase::Baseline && !self.report.contains_key("toplevel") {
                    eprintln!("embed_host: baseline child exited without stats");
                    self.finish(el);
                }
            }
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        let gap = (now - self.last_tick).as_secs_f64() * 1000.0;
        self.last_tick = now;
        if self.phase != Phase::Start {
            self.max_gap_ms = self.max_gap_ms.max(gap);
        }
        let t = self.phase_start.elapsed();
        match self.phase {
            Phase::Start => {
                if t > Duration::from_secs(20) {
                    eprintln!("embed_host: child never became ready");
                    self.finish(el);
                }
            }
            Phase::Embedded => {
                if now >= self.next_ping {
                    self.ping();
                    self.next_ping = now + Duration::from_millis(500);
                }
                if !self.resized && t > Duration::from_millis(2500) {
                    self.resized = true;
                    if let Some(w) = &self.window {
                        let _ = w.request_inner_size(PhysicalSize::new(1180u32, 740u32));
                    }
                }
                if self.resized
                    && !self.report.contains_key("resize_followed")
                    && t > Duration::from_millis(4000)
                {
                    let expected = self.editor_bounds();
                    #[cfg(windows)]
                    let ok = {
                        use windows_sys::Win32::Foundation::RECT;
                        use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;
                        let mut rc = RECT {
                            left: 0,
                            top: 0,
                            right: 0,
                            bottom: 0,
                        };
                        // SAFETY: rc is a valid out-pointer; the handle may be stale, in which case GetClientRect fails.
                        let r = unsafe { GetClientRect(self.child_hwnd as _, &mut rc) };
                        r != 0
                            && (rc.right - rc.left) == expected[2]
                            && (rc.bottom - rc.top) == expected[3]
                    };
                    #[cfg(not(windows))]
                    let ok = false;
                    self.report.insert("resize_followed".into(), json!(ok));
                }
                if !self.stats_requested && t.as_secs_f64() > PACING_SECS {
                    self.stats_requested = true;
                    self.send(json!({"type": "poc.stats"}));
                }
            }
            Phase::Hang => {
                if now >= self.next_ping {
                    self.ping();
                    self.next_ping = now + Duration::from_millis(250);
                }
                if self.resize_call_ms.is_none() && t > Duration::from_millis(500) {
                    let start = Instant::now();
                    if let Some(w) = &self.window {
                        let _ = w.request_inner_size(PhysicalSize::new(1220u32, 760u32));
                    }
                    self.resize_call_ms = Some(start.elapsed().as_secs_f64() * 1000.0);
                }
                if self.hang_detect_ms.is_none() && now - self.last_pong > WATCHDOG {
                    // detection time measured from the moment the hang began
                    self.hang_detect_ms = Some(t.as_secs_f64() * 1000.0);
                }
                if t > Duration::from_millis(HANG_MS + 1500) {
                    self.report.insert(
                        "hang".into(),
                        json!({
                            "hang_ms": HANG_MS,
                            "watchdog_detect_ms": self.hang_detect_ms,
                            "host_max_loop_gap_ms": self.max_gap_ms,
                            "host_resize_call_ms": self.resize_call_ms,
                            "child_recovered": now - self.last_pong < Duration::from_secs(1),
                        }),
                    );
                    if let Some(c) = self.child.as_mut() {
                        let _ = c.kill();
                    }
                    self.enter(Phase::Crash);
                }
            }
            Phase::Crash => {
                if !self.crash_checked && t > Duration::from_millis(2000) {
                    self.crash_checked = true;
                    #[cfg(windows)]
                    let gone = !win::exists(self.child_hwnd);
                    #[cfg(not(windows))]
                    let gone = false;
                    self.report.insert(
                        "crash".into(),
                        json!({"host_survived": true, "host_max_loop_gap_ms": self.max_gap_ms, "child_window_gone": gone}),
                    );
                    if let Some(mut c) = self.child.take() {
                        let _ = c.wait();
                    }
                    self.stdin = None;
                    self.child_hwnd = 0;
                    self.spawn(false);
                    self.enter(Phase::Baseline);
                }
            }
            Phase::Baseline => {
                if t > Duration::from_secs_f64(PACING_SECS + 20.0) {
                    eprintln!("embed_host: baseline timed out");
                    self.finish(el);
                }
            }
            Phase::Done => {}
        }
        el.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(10),
        ));
    }
}

fn main() {
    let el = EventLoop::<Ev>::with_user_event()
        .build()
        .unwrap_or_else(|e| {
            eprintln!("embed_host: {e}");
            std::process::exit(2)
        });
    let mut host = Host {
        proxy: el.create_proxy(),
        window: None,
        hwnd: 0,
        child: None,
        stdin: None,
        child_hwnd: 0,
        phase: Phase::Start,
        phase_start: Instant::now(),
        seq: 0,
        sent: Default::default(),
        rtts_ms: Vec::new(),
        last_pong: Instant::now(),
        last_tick: Instant::now(),
        max_gap_ms: 0.0,
        report: Default::default(),
        resized: false,
        stats_requested: false,
        hang_detect_ms: None,
        resize_call_ms: None,
        crash_checked: false,
        next_ping: Instant::now(),
    };
    let _ = el.run_app(&mut host);
}
