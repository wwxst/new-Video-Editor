# Timeline UI (mock data) — 2026-11-26

Implemented inside the TIMELINE card of the four-panel shell. Only the timeline module and the
shell's card hosting changed; Media, Preview, Inspector and the application architecture are
untouched. No engine, document or Agent code was modified.

**Revision (2026-11-26, later the same day):** the track-height rule was corrected. Lane heights are
fixed by content type and can never be resized by the user, so the earlier manual-resize behavior,
its 20–220 pt clamps and its edge drag handles were removed (details in "Fixed lane heights" below
and in the acceptance matrix row 9).

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
- **Fixed lane heights:** the height is a property of the content type, never of the lane instance:
  Text/Effect/Sticker 24 pt, Video/Image 76 pt, Audio 50 pt. All tracks of one type therefore share
  the same height, an empty track stays compact (24 pt) until its first clip, and there is no
  manual resize: no drag handles, no lane-edge hit zones, no 20–220 pt clamps. Horizontal zoom
  changes only the time scale; heights are unaffected. All geometry is in logical points, so
  DPI scaling is handled by egui.
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
| 3 | Heights differ only by content type | 24 / 24 / 24 / 76 / 76 / 76 / 50 / 50 pt lanes visible; unit test `default_heights_match_the_specification` |
| 4 | 4 pt lane spacing | `layout` gaps visible; `content_height` test (428 = 400 + 7×4); no divider lines drawn |
| 5 | Clip heights and layouts | 20 pt text/effect/sticker, 72 pt video/image (thumbnails + waveform), 46 pt audio (waveform); screenshots |
| 6 | Icons/controls aligned | Type icon, lock, eye/mute centred per lane in the control column; screenshots |
| 7 | No prominent borders / card tracks | Lanes are alternating fills inside one continuous canvas; clips are the only rounded blocks |
| 8 | Horizontal and vertical scrolling | Vertical: wheel (`timeline-scrolled.png`); horizontal: Shift+wheel (ruler and clips moved together); scrollbar drags covered by `dragging_the_scrollbars_changes_the_scroll_offsets` |
| 9 | Fixed, non-resizable track heights | Per-type constants 24/76/50 pt (`default_heights_match_the_specification`, `tracks_of_the_same_type_share_one_height`, `lane_height_follows_the_content_type_and_is_not_editable_state`); a lane-edge drag changes no height and does not scroll (`dragging_a_lane_edge_does_not_resize_it`); zoom invariance (`horizontal_zoom_does_not_change_track_heights`, and `timeline-zoom-heights.png` shows the same lane heights at 113 px/s as `run-fixed-heights.png` at 58 px/s) |
| 10 | Four-panel layout unchanged | All screenshots keep the media/preview/inspector/timeline arrangement; shells' panel code untouched |
| 11 | No height drag handles / no resize cursor | No lane-edge hit zones exist in `lanes`/`lane_content`; timeline edge drag test leaves heights untouched |
| 12 | DPI-aware rendering | All geometry is logical points resolved by egui's `pixels_per_point`; no physical-pixel constants in the timeline |

## Tests

`cargo test -p arc-app --profile dev` → **23 passed** (0 failed):

- model: fixed lane/clip heights per content type, equal height for every track of a type,
  dynamic typing (first clip, rejection, untyped after removal), mock ordering (audio below the
  main video) and content height;
- layout: ruler/canvas/control alignment, view start and zoom, max scroll for short viewports,
  empty timeline, playhead position, tick-step growth;
- interaction: splitter drag, scrollbar drags (both axes), lane-edge drag leaves heights and scroll
  untouched, horizontal zoom leaves heights and content height untouched.

The full workspace suite was not run in this pass (the build artifacts live in
`%TEMP%\arcline-target` with `CARGO_PROFILE_DEV_DEBUG=0`); CI will cover it. No engine or document
code was touched by this change.

## Boundaries kept

- Mock data only (`TimelineModel::mock`); no engine operations, no undo/split/delete/link behaviour,
  no drag-and-drop of real media, no persistence.
- Toolbar actions that would touch the document (undo, redo, split, delete, link) are visual only.
- The selection tool, snapping and linking toggles hold local UI state.
- Real integration points: replace `TimelineModel` with the document feed, route toolbar actions to
  `EngineHost` commands, and feed real thumbnails/waveforms into the clip painters.
