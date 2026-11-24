# ADR-0007 — `egui` shell with custom GPU canvases

**Status:** Accepted, 2026-11-24 — replaces the original "custom retained-mode widget toolkit" decision.
**Revised after:** [architecture review](../../reports/architecture-review.md).

## Context

The UI must be fast (timeline paint at 60+ Hz with thousands of clips), share one GPU context with the
viewer, and support a professional docking layout. The original proposal specified a bespoke widget layer
on `wgpu`. Its own estimate put basic parity at 3–4 engineer-months, before the long tail: IME, text
editing, native dialogs, drag-and-drop, multi-window, HiDPI, and accessibility. For a solo developer, that
is months spent producing no video-editing capability.

## Decision

Use **`egui`** (`eframe` with the `wgpu` backend, `egui_dock`, `egui_extras`) for the application shell, and
**custom-painted canvases** for the two surfaces that matter:

- Viewer: the compositor's output is presented as an `egui_wgpu` user texture — one device, no copies.
- Timeline: painted with `egui::Painter` (shapes/meshes) first; a dedicated `wgpu` pass only if profiling
  proves it necessary.
- Canvas text: `cosmic-text` + `glyphon` on the same device.

The custom widget toolkit (`arc-ui`) is removed from the architecture.

## Consequences

- Weeks-to-months saved; docking, dialogs, tables, menus, and text editing exist on day one.
- One `wgpu` device/queue shared by UI, viewer, and compositor.
- Accepted trade-offs: `egui` does not look native, text input is less polished than Qt, multi-window is
  limited. Mitigated with token theming, custom canvases, and a keyboard-first workflow.
- The shell is kept behind a thin app-level boundary so a future toolkit swap remains possible — but no
  speculative abstraction is built.
- Performance risk is bounded and measurable: if timeline painting misses budget, add a dedicated pass
  behind the existing canvas interface (days, not months).

## Alternatives considered

- **Custom retained-mode toolkit** (original decision): maximum control and polish ceiling, but months of
  undifferentiated work and a long polish tail; rejected for a solo project.
- **Qt**: best chrome, best docking, but heavy integration with a `wgpu` viewport, LGPL/commercial
  licensing complexity, and C++ interop cost.
- **`slint` / `iced` / `gpui`**: viable, better looking than `egui`, but younger ecosystems and weaker
  custom-canvas ergonomics for a timeline editor. Retained as fallback if `egui` proves limiting.
- **Electron / Tauri / WebView**: violates the native requirement and complicates GPU video surfaces.
