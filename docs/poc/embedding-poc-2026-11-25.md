# Embedding PoC — result (Phase P)

**Date:** 2026-11-25 · **Decision:** **GO** for out-of-process native child-window embedding (ADR-0014).
**Spec:** docs/13 §2.4 · **Code:** `crates/arc-app` (host loop), `crates/arc-app/examples/embed_host.rs`
(reference host).

## Setup

| Item | Value |
|---|---|
| OS | Windows 11 (Home, China edition) |
| GPU | Intel(R) Graphics (integrated) — the lower-bound reference class in docs/08 §2 |
| Toolchain | Rust 1.100.0, `x86_64-pc-windows-gnu`, MinGW-w64 GCC 16.2 (dlltool) |
| Stack | egui 0.37 + egui-winit 0.37 + egui-wgpu 0.37 → wgpu 30.0.2 (D3D12), winit 0.30.13 |
| Workload | dock layout (top/left/bottom/central panels), 8-track timeline canvas with ~2,000 shapes scrolling every frame, a 256×256 texture re-uploaded every frame, an IME-capable text field |

Run: `cargo run -p arc-app --release --example embed_host`. The host prints a JSON report and exits 0 on go
and 3 on no-go.

## Measured results

| Criterion (docs/13 §2.4) | Result | Pass |
|---|---|---|
| Child created as a native child of the host `HWND` | `GetParent(child) == host`, visible | ✅ |
| Frame pacing indistinguishable from top-level | embedded p50 17.05 / p95 17.35 / p99 17.67 ms vs top-level 17.03 / 17.25 / 17.35 ms | ✅ |
| Hitches (> 1.5× median) | 1 of 437 frames (0.2%), coincides with the scripted host resize | ✅ |
| Resize follows the host | host resized 1280×800 → 1180×740; child client area exactly matched the reserved region (940×740) | ✅ |
| Child hang (3 s UI-thread block) leaves the host usable | host event-loop max gap **11.7 ms**; host self-resize call 3.2 ms | ✅ |
| Watchdog detects the hang | **2.0 s** (threshold 2 s); child recovered afterwards | ✅ |
| Child crash leaves the host usable | host loop max gap 11.0 ms; child window gone | ✅ |
| Presentation on a child window (flip model) | rendered at vsync for the whole run | ✅ |
| Startup: spawn → `embed.ready` | 343 ms (debug-free release build, shader cache warm) | info |
| Embed channel ping RTT (stdio) | p50 11.0 ms, max 84 ms (n = 16) | info |

## Not yet verified (manual or out of scope for the automated run)

- **Keyboard focus round-trip and shortcut ownership** between host and child. The known Win32 hazard is
  attached input queues, which shows up through keyboard focus; the automated run did not click or type.
- **IME candidate placement** with CJK input inside the embedded text field.
- **DPI change** when dragging the host across monitors with different scaling.
- **Electron host** (Electron is not installed on this machine). The Win32 reference host proves the
  mechanism; an Electron host differs only in how it obtains its `HWND` (`getNativeWindowHandle()`).
- **Embed channel over the named pipe.** The PoC uses stdio; the message set is the same (docs/13 §2.2).

## Findings that affect the design

1. **No fallback mechanism needed.** The shared-texture composition spike (ADR-0014 decision 5) is not
   triggered.
2. **winit 0.30 supports `with_parent_window` on Windows and X11 only.** macOS is unsupported, which confirms
   that the v1.1 macOS port needs a different embedding mechanism (docs/13 §2.1, risk R21).
3. **`eframe` was not needed.** The `egui-winit` + `egui-wgpu` host loop is about 250 lines (`src/app.rs`)
   and covers top-level, child, and pop-out modes.
4. **Pop-out on parent loss works in-process.** The app recreates a top-level window when the parent
   disappears or `embed.detach` arrives. The watchdog path is implemented but its UI state reset (a fresh
   egui context) still needs polish in Phase J.

## Raw report

```json
{"checks":{"child_parented":true,"frame_pacing_matches_toplevel":true,"hitches_le_1pct":true,
 "host_responsive_during_child_hang":true,"host_survives_child_crash":true,"resize_followed":true,
 "watchdog_detects_hang_le_3s":true},
 "embedded":{"frames":437,"p50_ms":17.0467,"p95_ms":17.3454,"p99_ms":17.6734,"max_ms":32.346,"hitches":1,"width":940,"height":740},
 "toplevel":{"frames":417,"p50_ms":17.0274,"p95_ms":17.2527,"p99_ms":17.3456,"max_ms":17.4775,"hitches":0,"width":1280,"height":720},
 "hang":{"hang_ms":3000,"watchdog_detect_ms":2003.5,"host_max_loop_gap_ms":11.66,"host_resize_call_ms":3.18,"child_recovered":true},
 "crash":{"host_survived":true,"host_max_loop_gap_ms":10.99,"child_window_gone":true},
 "ping_rtt_ms":{"p50":10.99,"max":84.0,"n":16},"ready_ms":342.8,"go":true}
```
