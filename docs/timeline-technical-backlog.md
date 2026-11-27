# Timeline technical backlog

Living list for work that is intentionally outside the visual acceptance slices. Updated
2026-11-27 after the timeline Phase 2 zoom supplement.

## T1 — VFR end-to-end verification (must do)

**Status:** open. **Priority:** high.

The thumbnail path already asks `VideoDecoder` for the frame displayed at the project time and
records the returned PTS. `VideoDecoder::frame_at`/`frame_displayed_at` use the actual stream PTS,
including for streams flagged VFR. However, there is no VFR asset in the repository, so the pipeline
is not proven end to end.

**Acceptance**

1. Add a generated VFR fixture (e.g. MPEG-TS/MP4 with deliberately uneven frame durations) to the
   generated-fixture set; do not use third-party footage.
2. Decode a project-frame range and assert every returned PTS equals the expected PTS from the
   fixture, including a repeated PTS where the VFR gap spans more than one project frame.
3. Confirm the cache shares one texture for repeated project frames that resolve to the same PTS.
4. Confirm no UI-thread decode and no unbounded pending requests.

Until then, VFR is “supported by the decoder, not proven by a fixture”; the sampler uses the
stream's reported average rate for tile spacing.

## T2 — Idle continuous repaint / CPU (optimize later)

**Status:** open. **Priority:** medium.

`App::render` calls `window.request_redraw()` after every frame, so the editor continuously repaints
even when nothing is animating. At a 2400×1392 window this keeps CPU busy at idle. The media caches
themselves settle; this is a shell-loop issue, not a decoder leak.

**Acceptance**

1. Request a redraw only for input, active animations, pending media poll/repaint requests, window
   resize or playback.
2. Verify idle CPU drops while zoom, scroll, playhead drag and thumbnail/waveform fill still update
   smoothly.
3. Re-run the thumbnail/waveform caches at 1f and confirm no starvation under the on-demand loop.

## T3 — Move decoder header probing off the UI thread

**Status:** open. **Priority:** medium/low.

`MediaHub::video_meta` opens a `VideoDecoder` once per source path to read width, height, duration,
nominal rate and the VFR flag. No frame is decoded, but the first open happens on the UI thread and
can cause a small hitch on slow storage.

**Acceptance**

1. Probe stream metadata on the picture worker (or a dedicated probe worker) and cache the result.
2. The painter asks only the cached metadata/request queue; no file I/O on the UI thread.
3. Keep the one-open-per-path behaviour and the bounded failure cache.

## Closed in the Phase 2 supplement

- **Project rate ≠ source rate.** Thumbnail tiles now step by project frames. CFR cache keys use the
  source frame index, so a 24 fps source in a 30 fps project repeats the previous source frame on
  the six project frames per second that have no new source frame. The decoder maps those times with
  `frame_displayed_at` (actual PTS, no fabricated frames). Verified with generated 24 fps and 30 fps
  counter videos.
- **30 fps motion PTS check.** A generated 30 fps counter fixture decodes each project frame and
  asserts both the embedded frame index and the decoded PTS; a 24 fps fixture in a 30 fps project
  asserts 24 distinct source frames with repeated project-frame hits and 24 cached textures for 30
  tiles.
