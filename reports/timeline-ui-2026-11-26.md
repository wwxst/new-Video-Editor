# Timeline UI (mock data) — 2026-11-26

Implemented inside the TIMELINE card of the four-panel shell. Only the timeline module and the
shell's card hosting changed; Media, Preview, Inspector and the application architecture are
untouched. No engine, document or Agent code was modified.

## What was built

- **Chrome:** compact toolbar (selection, undo, redo, split, delete, link, snapping, zoom −/+, zoom
  readout), a time ruler aligned with the canvas, a 112 pt track-control column, and the lane canvas.
- **Six content types with dynamic typing:** a track has no fixed type; the first clip sets it, the
  type is derived from the clips, incompatible clips are rejected (`Track::add_clip`), and removing
  every clip makes the track untyped again. Track icons follow the content.
- **Lane geometry:** Text/Effect/Sticker 24 pt (clips 20), Video/Image 76 pt (clips 72), Audio
  50 pt (clips 46), with a 4 pt gap between lanes; alternating, borderless lane backgrounds; clips
  have their own rounded backgrounds and 2 pt vertical padding, never filling the lane.
- **Clips:** text/effect/sticker solid blocks with names; video with mock thumbnail strip, filename +
  duration and an embedded waveform strip; image with thumbnail strip and filename; audio with a
  waveform and filename + duration. Names clip to the block.
- **Ruler and playhead:** adaptive major/minor ticks (0.5 s … 5 min ladder, labels kept ≥ 78 pt
  apart), draggable playhead handle at the ruler, vertical playhead line through all lanes.
- **Scrolling and zoom:** wheel scrolls vertically, Shift+wheel horizontally, Ctrl+wheel zooms around
  the pointer; both scrollbar strips are draggable (hit area widened 6 pt inward); zoom 12–480 px/s.
- **Track controls:** type icon (tooltip names the lane), lock, visibility for visual lanes, mute for
  audio-capable lanes; only applicable controls are shown, vertically centred per lane.
- **Track resizing:** drag a lane's lower edge in the canvas; height clamps to 20–220 pt, the lane's
  controls and clips follow, and type defaults are restored when a track gets its first clip.
- **Empty state:** with no clips the chrome stays and the canvas shows "Drag media here to start a
  timeline"; no fake lanes or clips. (`ARCLINE_TIMELINE_EMPTY=1` is a mock aid used to inspect it.)

## Files

| Path | Change |
|---|---|
| `crates/arc-app/src/timeline/mod.rs` | New: `Timeline` state, `layout()`, toolbar, wheel/zoom, ruler/playhead, lanes, controls, scrollbars, tests |
| `crates/arc-app/src/timeline/model.rs` | New: `TrackKind`, `Clip`, `Track` (dynamic typing rules), `TimelineModel` + mock project, tests |
| `crates/arc-app/src/timeline/draw.rs` | New: vector drawing for toolbar icons, track/control icons, lanes, clips (thumbnails, waveforms), ruler, scrollbars |
| `crates/arc-app/src/shell.rs` | `card_content` hosts the timeline in the TIMELINE card; other panels unchanged |
| `crates/arc-app/src/main.rs` | `mod timeline;` |
| `reports/screenshots/*.png` | Captures used as evidence |

## Acceptance matrix

| # | Criterion | Evidence |
|---|---|---|
| 1 | Build and run | `cargo build -p arc-app` exit 0; app launched (screenshots) |
| 2 | Six track types render | `timeline-top.png` (text, effect, sticker, image, video) + `timeline-scrolled.png` (main video, audio × 2) |
| 3 | Different track heights | 24 / 24 / 24 / 76 / 76 / 76 / 50 / 50 pt lanes visible; unit test `default_heights_match_the_specification` |
| 4 | 4 pt lane spacing | `layout` gaps visible; `content_height` test (428 = 400 + 7×4); no divider lines drawn |
| 5 | Clip heights and layouts | 20 pt text/effect/sticker, 72 pt video/image (thumbnails + waveform), 46 pt audio (waveform); screenshots |
| 6 | Icons/controls aligned | Type icon, lock, eye/mute centred per lane in the control column; screenshots |
| 7 | No prominent borders / card tracks | Lanes are alternating fills inside one continuous canvas; clips are the only rounded blocks |
| 8 | Horizontal and vertical scrolling | Vertical: wheel (`timeline-scrolled.png`); horizontal: Shift+wheel (ruler and clips moved together); scrollbar drags covered by `dragging_the_scrollbars_changes_the_scroll_offsets` |
| 9 | Track height resizing | Drag test in the app (`timeline-resized.png`, music lane grew); implemented with 20–220 pt clamps |
| 10 | Four-panel layout unchanged | All screenshots keep the media/preview/inspector/timeline arrangement; shells' panel code untouched |

## Tests

`cargo test -p arc-app` → **19 passed** (0 failed):

- model: default lane/clip heights, dynamic typing (first clip, rejection, untyped after removal),
  mock ordering (audio below the main video) and content height;
- layout: ruler/canvas/control alignment, view start and zoom, max scroll for short viewports,
  empty timeline, playhead position, tick-step growth;
- interaction: splitter drag, scrollbar drags (both axes).

The full workspace suite was not run in this environment: the build drive is full (the build artifacts
live in `%TEMP%\arcline-target` with `CARGO_PROFILE_DEV_DEBUG=0`), so `cargo test --workspace` cannot
link; CI will cover it. No engine or document code was touched by this change.

## Boundaries kept

- Mock data only (`TimelineModel::mock`); no engine operations, no undo/split/delete/link behaviour,
  no drag-and-drop of real media, no persistence.
- Toolbar actions that would touch the document (undo, redo, split, delete, link) are visual only.
- The selection tool, snapping and linking toggles hold local UI state.
- Real integration points: replace `TimelineModel` with the document feed, route toolbar actions to
  `EngineHost` commands, and feed real thumbnails/waveforms into the clip painters.
