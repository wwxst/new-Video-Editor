# 07 — UI Architecture

The UI is a client of the engine, like an agent. It is a high-quality *view* over the document and a
producer of commands. This keeps the editing surface honest: anything the GUI can do, the protocol can do.

## 1. Stack decision

**Own retained-mode widget layer on `wgpu`.** No Electron, no Qt, no WebView.

```
arc-ui
├── layout      (taffy flexbox/grid)
├── text        (cosmic-text shaping + glyph atlas management)
├── paint       (wgpu renderer: quads, glyphs, rounded rects, shadows, clips)
├── widgets     (buttons, sliders, trees, tabs, docks, popups, virtualized lists)
├── input       (mouse, pen, keyboard, IME, shortcuts, drag-and-drop, accessibility bridge)
└── style       (theme tokens, light/dark, density, motion)
```

Rationale:

- A professional NLE's most important surfaces — timeline and preview — are custom GPU canvases anyway.
  A framework buys you chrome, not the parts that matter.
- One GPU context, one event loop, one allocator. No cross-runtime IPC for a hover state.
- Full control of text rendering and timeline paint at 120 Hz; no DOM layout cost.
- `accesskit` provides the OS accessibility tree, which is the piece people assume requires a platform
  toolkit; it does not.

**The cost is real:** widgets take 3–4 engineer-months to reach parity with a stock toolkit for a *basic*
app, and years for an exhaustive one. Mitigation, in order:

1. **Phase 0–1 prototype shell in `egui`** (dev-only, feature-flagged) to exercise the engine before the
   widget layer exists. It is deleted at P2; nothing ships on it.
2. **Fixed widget inventory** for v1 ([§3.2](#32-widget-inventory)) — we build exactly this list, not a
   general-purpose GUI toolkit.
3. **Buy the hard parts**: `taffy` (layout), `cosmic-text` (shaping), `accesskit`, `winit`.

See ADR-0007 for the full alternative analysis (Qt, egui, Slint, iced, GPUI).

## 2. Application shell

```
┌──────────────────────────────────────────────────────────────────────────┐
│ menubar / workspace tabs                    project · dirty · revision  │
├──────────────┬───────────────────────────────────────────┬───────────────┤
│ Media bin    │                Viewer (preview)           │  Inspector    │
│ (tree/list)  │   ┌───────────────────────────────┐       │  (params,     │
│              │   │                               │       │   keyframes,  │
│  assets      │   │         frame canvas          │       │   metadata)   │
│  effects     │   │                               │       │               │
│  transitions │   └───────────────────────────────┘       │  Audio mixer  │
│              │   transport · scopes · quality badge     │  (tab)        │
├──────────────┴───────────────────────────────────────────┴───────────────┤
│ Timeline: ruler · tracks · clips · playhead · markers · zoom              │
├──────────────────────────────────────────────────────────────────────────┤
│ status bar: cache · fps · audio latency · job progress · errors          │
└──────────────────────────────────────────────────────────────────────────┘
```

- **Docking**: a split tree of panels (our own `DockTree`, serializable to the workspace file). Panels can
  float into OS windows (spawned as secondary `winit` windows sharing the adapter), tab together, and
  collapse. Layouts are saved per workspace (`Edit`, `Color`, `Audio`, `Agent Ops`).
- **Single-document window** in v1, with tabs reserved in the model (one `Session` per tab in P4+).
- **Startup**: window with skeleton and last layout appears immediately; project loading fills panels
  progressively ([01 §8](01-system-architecture.md)).

## 3. Widgets

### 3.1 Rendering model

Retained tree, immediate-mode *declaration*: widgets are rebuilt each frame from state via a declarative
description, diffed against the previous tree, and only changed subtrees are updated. Painting is batched
into a small number of draw calls (textured quads + SDF rounded rects + glyph atlas). Damage tracking
ensures idle frames cost near zero (present-on-demand, no continuous 60 Hz redraw when nothing changed).

Why not pure immediate mode: inspector trees with thousands of keyframes, virtualized lists, and
accessibility all want identity. Why not full retained mutation: declarative rebuild eliminates the
"widget drifted from state" bug class. The diff is the compromise professionals use (Figma/Flutter model).

### 3.2 Widget inventory (v1 — build exactly this)

Buttons, toggles, segmented controls, sliders + numeric fields (with expression-ish input: `+2`,
`1.5*2`, `-6dB`), color picker, dropdowns, combo boxes, tabs, collapsible sections, trees, tables,
virtualized lists, tooltips, popovers, modals, context menus, toasts, file dialogs (native where available,
`rfd`), progress bars, splitters, dock panels, scroll areas (wheel + trackpad + drag), text edit (single/multi
line with IME + selection + undo), badges, icons (SVG → atlas, `resvg`).

Explicitly *not* building: rich text documents, layout editors, charts (we draw our own timeline/scope
canvases), web views.

### 3.3 Inspector

Auto-generated from effect manifests ([02 §3.4](02-domain-model-and-project-format.md)): the manifest drives
the control set, ranges, units, and grouping. A new effect appears in the inspector with a working UI the
moment its manifest exists — no per-effect hand-written panel. Keyframe editing (add/remove/move, curve
selection, interpolation menu) is a shared component used by every parameter. This is also why MCP tools and
UI stay in sync: one manifest, two renderers.

## 4. Timeline renderer

The timeline is a custom canvas, not a widget tree — thousands of clips must paint in one pass.

- **Virtualization**: only the visible time range contributes draw data. Data is a set of instanced quads
  (clip body, selected state, fades, effect badges) plus a glyph run list for labels.
- **Level of detail**:
  | Zoom | What is drawn |
  |---|---|
  | > 40 px/s | full clip labels, thumbnails strip, waveform, per-effect badges, keyframe markers |
  | 8–40 px/s | labels, waveform envelope, no thumbnails |
  | < 8 px/s | solid blocks + transition glyphs only |
- **Thumbnails**: filmstrip from the thumbnail cache (2 Hz), requested asynchronously, drawn as a texture
  atlas per clip. Never blocks a frame; missing tiles draw as a flat color.
- **Waveforms**: min/max pyramid from the peak cache, drawn as a triangle strip; LOD switches at zoom.
- **Playhead and scroll**: playhead is the only per-frame-updating element during playback; scroll follows
  playhead with a "soft" mode. Scrolling never re-lays-out clips — it changes the viewport transform.
- **Hit testing**: spatial index per track (sorted clip array + binary search); drag/trim handles are 6 px
  hit zones with pointer-type awareness (pen exactness, touch fat-fingers are out of scope).
- **Interaction gestures**: move (with snapping to playhead/edges/markers, magnetic mode optional and
  always expandable to explicit commands), trim in/out (with "ripple" modifier), slip/slide, split at
  playhead, razor, selection box, range select, track mute/solo/lock toggles, drag-to-relink.
- **Undo affinity**: a completed gesture is one command batch; an aborted gesture (Esc) is nothing.

Budget: 60 fps scroll/live-drag with 2,000 visible clip-seconds and 8 tracks; 8 ms CPU paint budget
([08 §2](08-performance-reliability.md)).

## 5. Viewer

- Presents the renderer's output texture; zoom (fit / 50% / 100%), pixel-accurate 1:1 mode, pan, safe-area
  guides, alpha checkerboard.
- Overlays: scopes (waveform, vectorscope, histogram — GPU-computed, drawn in a panel), quality-level badge
  when the governor degrades, timecode bar, A/B compare with a wipe against a reference frame.
- Click-to-inspect: clicking exports a frame grab at full resolution to the media bin (useful for agents
  too — same command).

## 6. Input, shortcuts, command palette

- **Command registry**: every UI action is a named command with metadata (label, icon, shortcut defaults,
  enablement predicate). Menus, buttons, and the palette all render *from the registry*, so a new action is
  declared once.
- **Keymap**: platform-aware defaults (Cmd vs Ctrl), user-rebindable, conflict detection, chord support
  (`Ctrl+K Ctrl+S`), context scoping (timeline vs viewer vs global). Stored per workspace with import/export.
- **Command palette** (`Ctrl/Cmd+K`): fuzzy search over the registry; invoking a command is *identical* to
  the menu path and to the agent's RPC call (same command ID where meaningful). This is the clearest
  demonstration that the GUI is a protocol client.
- **IME** is handled properly for CJK text entry (candidate window positioning, composition preview) —
  a common failure in cross-platform GPU UIs, so it is in the P2 acceptance criteria.

## 7. Accessibility, theming, i18n

- **Accessibility**: `accesskit` tree maintained alongside the widget tree; keyboard-only operation for
  every command (no mouse-only paths); focus rings; reduced-motion setting honored (timeline animations off).
  Screen-reader support is a P6 gate, keyboard completeness is a P2 gate.
- **Theming**: token-based (color roles, spacing scale, type ramp). Light, dark, high-contrast. No hard-coded
  colors in widgets; a theme is a token map.
- **i18n**: `fluent` bundles, no concatenated strings, plural rules respected. English at v1; localization
  infrastructure ships so community translations are possible without code changes.

## 8. Failure UX

The UI is where recovery becomes trust:

- **GPU device loss**: overlay "Restoring GPU…" ≤ 2 s, session continues.
- **Decoder crash**: clip shows a damaged-media hatch; a "retry" action respawns the worker.
- **Engine degraded** (panic caught): banner with "Export last good state" and "View crash report";
  project remains exportable.
- **Missing media**: placeholder frames with the asset name; relink dialog is reachable in one click; the
  timeline is fully editable with offline media.
- **Slow operation**: jobs run with progress in the status bar and a cancel action; nothing modal except
  file dialogs and destructive confirmations.
