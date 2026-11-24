# 07 — UI Architecture

v2 decision: **`egui` for the application shell, custom `wgpu` canvases for the timeline and viewer.**
The custom widget toolkit from v1 is removed ([ADR-0007](adr/0007-native-wgpu-ui.md)). The reason is
schedule, not taste: a polished widget layer is months of work that produces no video-editing capability,
and `egui` already provides docking, dialogs, tables, menus, text editing, and a `wgpu` integration.

## 1. Stack

```
arc-app
├── eframe (winit + wgpu backend)
├── egui + egui_dock + egui_extras      # shell, panels, tables, dialogs
├── custom timeline canvas              # egui::Painter first; dedicated pass if needed
├── custom viewer                       # egui_wgpu user texture
├── cosmic-text / glyphon               # canvas text, titles
└── rfd                                 # native file dialogs
```

One `wgpu::Device`/`Queue` for everything:

- The viewer presents the compositor's output as an `egui_wgpu` user texture — zero interop copies.
- The timeline paints in `egui` until profiling shows it is insufficient. Thousands of rectangles per frame
  are well within `egui`'s capability on desktop; a dedicated render pass is a measured optimization, not a
  starting point.
- Text on canvas (titles, subtitles, clip labels) uses `glyphon` against the same device.

**Accepted trade-offs:** `egui` does not look native, text input is less polished than Qt, and multi-window
is limited. A theming layer (fonts, spacing, colors, icon set) narrows the gap for v1. The application shell
is kept behind a thin boundary so a future toolkit swap remains possible, but no abstraction is built
speculatively.

## 2. Application shell

```
┌──────────────────────────────────────────────────────────────────────────┐
│ menubar · project name · dirty · revision · job progress                 │
├──────────────┬───────────────────────────────────────────┬───────────────┤
│ Media bin    │                Viewer                     │  Inspector    │
│ (tree/list)  │   ┌───────────────────────────────┐       │  params,      │
│              │   │        composited frame       │       │  keyframes,   │
│  assets      │   └───────────────────────────────┘       │  metadata     │
│  effects     │   transport · timecode · quality badge    │               │
├──────────────┴───────────────────────────────────────────┴───────────────┤
│ Timeline: ruler · tracks · clips · playhead · markers · zoom             │
├──────────────────────────────────────────────────────────────────────────┤
│ status bar: cache · fps · audio latency · errors                         │
└──────────────────────────────────────────────────────────────────────────┘
```

- **Docking** via `egui_dock`: panels split, tab, and collapse; layouts are serializable to the workspace
  file. One workspace in v1; presets (`Edit`, `Color`, `Agent Ops`) are saved layout snapshots.
- **Single-document window.** Tabs are a post-1.0 feature.
- **Startup:** window and skeleton appear immediately; project load fills panels progressively
  ([01 §8](01-system-architecture.md)).

## 3. Inspector

Generated from effect manifests ([02 §3.4](02-domain-model-and-project-format.md)): the manifest drives
controls, ranges, units, and grouping. A new effect gets a working inspector the moment its manifest exists.
Keyframe editing (add/remove/move, interpolation menu) is a shared component used by every animatable
property. Because MCP and the inspector read the same manifest, tool schemas and UI controls cannot drift.

Numeric fields accept expressions (`+2`, `1.5*2`, `-6dB`) evaluated by a tiny arithmetic parser — a
professional-editor expectation and cheap to implement.

## 4. Timeline canvas

Custom-painted, virtualized:

- **Data**: per visible time range, instanced rectangles for clip bodies, selection, fades, effect badges,
  plus glyph runs for labels. Only visible clips contribute draw work.
- **LOD**:
  | Zoom | What is drawn |
  |---|---|
  | > 40 px/s | labels, thumbnail strip, waveform, effect badges |
  | 8–40 px/s | labels, waveform envelope |
  | < 8 px/s | solid blocks + transition glyphs |
- **Thumbnails and waveforms** come from caches ([04 §6](04-media-pipeline.md)); missing tiles draw flat
  and never block a frame.
- **Hit testing**: per-track sorted clip array + binary search; trim handles are 6 px zones.
- **Gestures**: move (snapping to playhead, edges, markers), trim in/out (ripple modifier), slip/slide,
  split at playhead, razor, selection box, track mute/solo/lock, drag-to-relink. A completed gesture is one
  command batch; Esc aborts with no command.
- **Magnetic behavior is UI-side**: snapping and ripple presets expand into explicit commands before they
  reach the engine, so the recorded edit is deterministic.

Budget: 60 fps scroll and drag with 8 tracks and 2,000 visible clip-seconds; 8 ms CPU paint
([08 §2](08-performance-reliability.md)).

## 5. Viewer

- Presents the compositor texture; zoom (fit / 50% / 100%), 1:1 pixel mode, pan, safe-area guides.
- Overlays: timecode bar, quality badge when the governor degrades, optional histogram/waveform scope.
- Click-to-export-frame: a full-resolution grab lands in the media bin (the same command agents use).

## 6. Input and commands

- **Command registry**: every action is a named command with label, icon, shortcut defaults, and an
  enablement predicate. Menus, buttons, and the palette render from this registry.
- **Keymap**: platform-aware defaults (Cmd/Ctrl), user-rebindable, conflict detection, chords
  (`Ctrl+K Ctrl+S`), context scoping. Stored per workspace, importable/exportable.
- **Command palette** (`Ctrl/Cmd+K`): fuzzy search over the registry. This demonstrates in the product that
  the GUI is a protocol client — the palette invocation and the agent call share command semantics.
- **IME** for CJK text entry is an explicit Phase C acceptance criterion.

## 7. Accessibility, theming, i18n

- **Accessibility**: keyboard completeness for every command (Phase C gate); `egui`'s accesskit integration
  provides an OS accessibility tree as a best-effort. Screen-reader certification is post-1.0.
- **Theming**: token-based (color roles, spacing scale, type ramp); light, dark, high-contrast. No
  hard-coded colors in widgets.
- **i18n**: `fluent` bundles, no concatenated strings, plural rules respected. English at v1.

## 8. Failure UX

- **GPU device loss**: "Restoring GPU…" overlay ≤ 2 s; session continues.
- **Decoder error/panic**: the clip shows a damaged-media hatch; retry is one click; other clips continue.
- **Engine degraded**: banner with "Export last good state" and "View crash report".
- **Missing media**: placeholders with asset names; relink is one click; the timeline stays fully editable.
- **Long operations**: jobs show progress in the status bar with cancel; nothing modal except file dialogs
  and destructive confirmations.
