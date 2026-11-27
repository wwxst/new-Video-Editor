# 07 — UI Architecture

**Final decision (2026-11-27):** **`egui` is the only desktop UI framework**, with custom `wgpu` canvases
for the timeline and viewer. The stack is fixed by [ADR-0015](adr/0015-final-ui-stack-and-design-system.md);
Slint and MLT are not adopted and are not open options. The shared component library and visual rules are
defined by [14 — UI Design System](14-ui-design-system.md).

The custom widget toolkit from v1 is removed ([ADR-0007](adr/0007-native-wgpu-ui.md)). The reason is
schedule, not taste: a polished widget layer is months of work that produces no video-editing capability,
and `egui` already provides docking, dialogs, tables, menus, text editing, and a `wgpu` integration.

## 1. Stack

```
arc-app
├── host loop: egui-winit + egui-wgpu   # top-level window, or child window when embedded (13 §2.1)
├── egui + egui_dock + egui_extras      # shell, panels, tables, dialogs
├── ui/                                 # tokens, theme, shared components, patterns (14)
├── custom timeline canvas              # egui::Painter first; dedicated pass if needed
├── custom viewer                       # egui_wgpu user texture
├── cosmic-text / glyphon               # canvas text, titles
└── rfd                                 # native file dialogs
```

The component library is inside `arc-app`, not a seventh crate; the six-crate rule and the dependency
direction in [01 §4](01-system-architecture.md) are unchanged.

One `wgpu::Device`/`Queue` for everything:

- The viewer presents the compositor's output as an `egui_wgpu` user texture — zero interop copies.
- The timeline paints in `egui` until profiling shows it is insufficient. Thousands of rectangles per frame
  are well within `egui`'s capability on desktop; a dedicated render pass is a measured optimization, not a
  starting point.
- Text on canvas (titles, subtitles, clip labels) uses `glyphon` against the same device.

**Accepted trade-offs:** `egui` does not look native, text input is less polished than Qt, and multi-window
is limited. The Arcline design system ([14](14-ui-design-system.md)) carries the product look. The
application shell is kept behind the existing `arc-app` UI module for testability and embedding; **no
alternate toolkit abstraction is built, and no toolkit swap is planned** ([ADR-0015](adr/0015-final-ui-stack-and-design-system.md)).

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
  file. One workspace in v1; presets (`Edit`, `Color`, `Agent Ops`, `Companion`) are saved layout snapshots. `Companion` is the compact
  Embedded-mode layout: viewer, timeline, activity strip ([13 §8](13-product-modes-and-live-sessions.md)).
- **Single-document window.** Tabs are a post-1.0 feature.
- **Agent panel** (dockable): chat, streamed replies, each tool call with its result and an *Undo* link,
  confirmation prompts for renders/exports/new save paths, model and key settings
  ([13 §7](13-product-modes-and-live-sessions.md)).
- **Activity strip** (shown whenever any agent is connected): connected clients, control state
  (`shared` / `human` / `agent`), last applied commands, and a **Take over** button. Agent edits are
  highlighted briefly on the timeline and attributed in the history panel.
- **Consent prompt** on the first connection from an external client ([13 §3](13-product-modes-and-live-sessions.md)).
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

- Presents the compositor texture through the shared `egui-wgpu` device as a user texture/custom paint
  callback — **no CPU readback in the UI path**. Zoom (fit / 50% / 100%), 1:1 pixel mode, pan, safe-area
  guides.
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
  hard-coded colors in widgets. The complete token set, component inventory, and state rules are in
  [14 — UI Design System](14-ui-design-system.md).
- **i18n**: `fluent` bundles, no concatenated strings, plural rules respected. English at v1.

## 8. Failure UX

- **GPU device loss**: "Restoring GPU…" overlay ≤ 2 s; session continues.
- **Decoder error** (error code or wrapper panic): the clip shows a damaged-media hatch; retry is one click;
  other clips continue.
- **Quarantined media** (failed import check, or implicated in a previous crash): badge on the asset;
  preview uses its proxy; the reason is shown in the inspector ([04 §1.2](04-media-pipeline.md)).
- **Decode-safe mode** (stuck-decoder limit reached, [04 §1.2](04-media-pipeline.md)): persistent banner;
  preview runs on proxies; restart restores normal decoding.
- **After an unclean exit**: the recovery dialog reports the recovered revision, any discarded journal
  records, and any newly quarantined asset ([08 §4.6](08-performance-reliability.md)).
- **Interchange export**: *File → Export Project for…* picks a target editor and packaging mode, then shows
  the compatibility report (unverified and baked items first) ([12](12-project-interchange.md)).
- **Export failure**: exports run in a child process, so a crash fails the job with a report; the session
  continues.
- **Engine degraded**: banner with "Export last good state" and "View crash report".
- **Missing media**: placeholders with asset names; relink is one click; the timeline stays fully editable.
- **Long operations**: jobs show progress in the status bar with cancel; nothing modal except file dialogs
  and destructive confirmations.

## 9. Decision history (research only)

- **v1 custom widget toolkit** — rejected in the 2026-11-24 architecture review; it would have cost months
  before the first video-editing capability.
- **Slint 1.19.1 + wgpu 30** — validated end to end on 2026-11-27: four panels, timeline, zero-copy
  preview, native child window, Chinese IME, 150 % DPI, and the royalty-free licence all worked. It was
  rejected for the editor because the retained-mode timeline was 10–13× slower than `egui` at 1,000 clips
  (5–7 % late frames), Skia could not be built with the current windows-gnu toolchain, and migration cost
  was high. The POC and measurements are archived on branch `poc/slint-ui`; they are not merged and no
  Slint dependency enters the product.
- The final decision and the long-term rules are in [ADR-0015](adr/0015-final-ui-stack-and-design-system.md).
  This section exists so the research is not lost; it is not a menu of alternatives.
