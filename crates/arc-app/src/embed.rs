//! Embed channel and parent-window handling (docs/13 §2.1–2.2).
//!
//! The channel is NDJSON. In the PoC it runs over the child's stdin/stdout (`--embed-channel
//! stdio`); production moves the same messages onto the per-user named pipe (docs/13 §3).
#![allow(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};

/// Messages host → Arcline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HostMsg {
    #[serde(rename = "embed.attach")]
    Attach {
        bounds: [i32; 4],
        #[serde(default)]
        dpi: Option<f32>,
    },
    #[serde(rename = "embed.resize")]
    Resize {
        bounds: [i32; 4],
        #[serde(default)]
        dpi: Option<f32>,
    },
    #[serde(rename = "embed.visibility")]
    Visibility { visible: bool },
    #[serde(rename = "embed.focus")]
    Focus { grant: bool },
    #[serde(rename = "embed.ping")]
    Ping { seq: u64 },
    #[serde(rename = "embed.detach")]
    Detach { reason: String },
    /// PoC only: block the UI thread to test the hang watchdog.
    #[serde(rename = "poc.hang")]
    PocHang { ms: u64 },
    /// PoC only: report frame statistics now.
    #[serde(rename = "poc.stats")]
    PocStats,
}

/// Messages Arcline → host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AppMsg {
    #[serde(rename = "embed.ready")]
    Ready { hwnd: isize, pid: u32 },
    #[serde(rename = "embed.pong")]
    Pong { seq: u64 },
    #[serde(rename = "embed.focus_release")]
    FocusRelease,
    #[serde(rename = "embed.detach")]
    Detach { reason: String },
    #[serde(rename = "poc.stats")]
    Stats(crate::poc::FrameStats),
}

pub fn send(msg: &AppMsg) {
    let mut out = std::io::stdout().lock();
    if let Ok(line) = serde_json::to_string(msg) {
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
}

/// Spawn a thread reading host messages from stdin and forwarding them into the event loop.
pub fn spawn_stdin_reader(proxy: winit::event_loop::EventLoopProxy<crate::app::UserEvent>) {
    std::thread::Builder::new()
        .name("embed-channel".into())
        .spawn(move || {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<HostMsg>(&line) {
                    Ok(m) => {
                        if proxy.send_event(crate::app::UserEvent::Host(m)).is_err() {
                            break;
                        }
                    }
                    Err(e) => eprintln!("embed-channel: ignoring malformed message: {e}"),
                }
            }
            // host closed the channel: treat as detach
            let _ = proxy.send_event(crate::app::UserEvent::Host(HostMsg::Detach {
                reason: "channel closed".into(),
            }));
        })
        .ok();
}

/// Build the raw parent handle for `WindowAttributes::with_parent_window`.
#[cfg(windows)]
pub fn parent_handle(hwnd: isize) -> Option<winit::raw_window_handle::RawWindowHandle> {
    let h = std::num::NonZeroIsize::new(hwnd)?;
    Some(winit::raw_window_handle::RawWindowHandle::Win32(
        winit::raw_window_handle::Win32WindowHandle::new(h),
    ))
}

#[cfg(not(windows))]
pub fn parent_handle(_hwnd: isize) -> Option<winit::raw_window_handle::RawWindowHandle> {
    None
}

/// Native handle of a winit window (Windows only; 0 elsewhere).
pub fn hwnd_of(window: &winit::window::Window) -> isize {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
        _ => 0,
    }
}

/// Whether `parent` is still a live window (watchdog input; docs/13 §2.5).
#[cfg(windows)]
pub fn is_window(hwnd: isize) -> bool {
    // SAFETY: IsWindow accepts any value and only reads the window table.
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::IsWindow(hwnd as _) != 0 }
}

#[cfg(not(windows))]
pub fn is_window(_hwnd: isize) -> bool {
    true
}
