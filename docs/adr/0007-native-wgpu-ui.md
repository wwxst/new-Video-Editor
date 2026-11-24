# ADR-0007 — Native retained-mode UI on `wgpu`

**Status:** Accepted, with documented fallback · 2026-11-24

## Context

The UI must be fast (timeline paint at 60–120 Hz with thousands of clips), look native on three OSes, share
one GPU context with the viewer, and support a professional docking layout. The project explicitly requires
native technology and rejects web runtimes. The hardest surfaces — timeline and viewer — are custom-drawn
regardless of framework choice.

## Decision

Build our own **retained-mode widget layer over `wgpu`** (`arc-ui`): declarative tree rebuilt per frame,
diffed, painted in batched draw calls; layout via `taffy`; text via `cosmic-text`/`rustybuzz`; accessibility
via `accesskit`; windows/input via `winit`. Ship a **fixed widget inventory** for v1 ([07 §3.2](07-ui-architecture.md)).
Use `egui` only as an internal development shell in P0–P1, deleted at P2.

## Consequences

- One GPU context; no IPC for UI state; complete control of paint, text, and timeline LOD.
- Accessibility does not require a platform toolkit; `accesskit` supplies the OS tree.
- **The cost is schedule risk**: reaching a polished chrome takes an estimated 3–4 engineer-months, more
  than adopting a toolkit. Mitigations: fixed inventory, buying layout/text/a11y, an early internal shell.
- **Documented fallback**: if the widget layer misses the P2 gate, the contingency is a license-compatible
  toolkit (e.g., `slint` or a Qt binding) behind the same `arc-ui` widget interface. Reversal cost is
  bounded to the chrome; the timeline and viewer are custom either way. A reversal decision point is placed
  at the end of week 16.
- The UI is permanently independent of any third-party UI license — a long-term benefit for a commercial
  product.

## Alternatives considered

- **Qt (C++ or `cxx-qt`)**: most mature for pro apps (Resolve uses it), great docking, but LGPL/commercial
  licensing complexity, heavy binary, and painful interop with a `wgpu` viewer surface.
- **`egui`**: fastest to a working shell, excellent for tools, but immediate-mode costs at high widget
  counts, weaker docking, and styling ceilings for a polished product.
- **`slint`, `iced`, `gpui`**: each viable but younger ecosystems with less control over custom GPU
  compositing; retained as the fallback behind our widget interface.
- **Electron / Tauri / WebView**: violates the native requirement and adds a whole runtime, plus
  inconsistent video presentation on all platforms.
