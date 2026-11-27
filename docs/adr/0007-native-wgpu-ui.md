# ADR-0007 — `egui` shell with custom GPU canvases

**Status:** Accepted, 2026-11-24. **Confirmed final by [ADR-0015](0015-final-ui-stack-and-design-system.md)
on 2026-11-27.** The "future toolkit swap / Slint fallback" wording below is historical; it is not an open
option.
**Revised after:** [architecture review](../../reports/architecture-review.md).
**Amended 2026-11-24 by [ADR-0014](0014-integration-levels-and-native-embedding.md):** the app loop uses
`egui-winit` + `egui-wgpu` directly instead of `eframe`, so the editor can run as a child window inside a host.
The `egui` decision itself is unchanged.

## Context

The UI must be fast (timeline paint at 60+ Hz with thousands of clips), share one GPU context with the
viewer, and support a professional docking layout. The original proposal specified a bespoke widget layer
on `wgpu`. Its own estimate put basic parity at 3–4 engineer-months, before the long tail: IME, text
editing, native dialogs, drag-and-drop, multi-window, HiDPI, and accessibility. For a solo developer, that
is months spent producing no video-editing capability.

## Decision

Use **`egui`** (`egui-winit` + `egui-wgpu` host loop — originally `eframe`, see amendment — with `egui_dock`
and `egui_extras`) for the application shell, and
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
- The shell is kept behind the existing `arc-app` UI boundary for testability and embedding. The earlier
  "future toolkit swap" rationale is closed by ADR-0015: no alternate-toolkit abstraction is built.
- Performance risk is bounded and measurable: if timeline painting misses budget, add a dedicated pass
  behind the existing canvas interface (days, not months).

## Alternatives considered

- **Custom retained-mode toolkit** (original decision): maximum control and polish ceiling, but months of
  undifferentiated work and a long polish tail; rejected for a solo project.
- **Qt**: best chrome, best docking, but heavy integration with a `wgpu` viewport, LGPL/commercial
  licensing complexity, and C++ interop cost.
- **`slint` / `iced` / `gpui`**: historical alternatives considered. Slint was fully validated in 2026
  (branch `poc/slint-ui`) but is not adopted; it is not a standing fallback. See ADR-0015 for the measured
  reasons and the final decision.
- **Electron / Tauri / WebView**: violates the native requirement and complicates GPU video surfaces.
