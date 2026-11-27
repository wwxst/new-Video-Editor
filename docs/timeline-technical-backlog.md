# Timeline technical backlog

Living list for work that is intentionally outside the visual acceptance slices. Updated
2026-11-27 after the timeline Phase 2 zoom supplement and T1 VFR verification.

## T1 — VFR end-to-end verification (must do)

**Status:** **closed 2026-11-27**. **Priority:** high.
**Report:** [timeline T1 VFR acceptance](../reports/timeline-t1-vfr-2026-11-27.md).

The thumbnail path asks `VideoDecoder` for the frame displayed at the project time and records the
returned PTS. T1 added a generated VFR fixture and proved the whole path, including cache sharing
for repeated project frames.

**Acceptance — all met**

1. **Generated VFR fixture.** `arc_media::fixtures::write_vfr_counter_video` builds a 30-frame,
   320×180 MP4/FFV1 counter with unique frame numbers and 0.2 s gaps at frames 10 and 20. It is
   generated from the existing counter writer, not third-party footage. MP4 is required: the MKV
   muxer hides the VFR flag by writing the nominal default duration.
2. **PTS mapping.** `vfr_counter_maps_every_project_frame_to_expected_pts` decodes project frames
   at 24, 25, 30, 29.97 and 59.94 fps and asserts both the actual PTS and the embedded counter equal the
   fixture's expected source frame. The 0.233 s gap spans seven 30 fps project frames; the same
   source PTS is observed for those repeated project frames.
3. **Cache sharing.** `vfr_thumbnail_cache_shares_one_texture_per_source_pts` requests 30 project
   frames at 30 fps. They resolve to 20 distinct source frames, so the hub reports 20 requests and
   20 textures (10 repeated tiles are cache hits). VFR intervals are learned from the decoder's
   look-ahead PTS; `FrameKey::Pts` is the cache key.
4. **UI-thread/bounded guards.** `MediaHub::video_frame` only enqueues work; the test asserts
   `decoded == 0` before the UI poll and `pending <= MAX_PENDING` (24). A per-path `vfr_probe`
   guard allows one probe decode per unknown interval, so a long gap cannot flood the worker.
5. **1f thumbnail sample time.** `paint_video` now samples the **start** of the project-frame span
   via `thumbnail_sample_time`/`FrameRate::time_of_frame`, matching `plan_frame` in preview/export.
   The old midpoint sampled 50 ms for project frame 1 at 30 fps and disagreed with a 24 fps source.
   `thumbnail_sample_time_is_the_exact_project_frame_start` covers 24/25/30/29.97/59.94.
6. **Real-fixture three-way checks.** `thumbnail_source_frame_matches_displayed_at_cfr_rates_and_reverse`
   drives the real thumbnail worker/cache for 24/25/30/29.97/59.94 and 24-in-30, forward and
   reverse, and compares actual thumbnail PTS + counter with `frame_displayed_at`.
   `thumbnail_source_frame_matches_displayed_at_vfr_boundaries_and_reverse` covers the VFR gap,
   exact tick-shifted boundary, last frame and reverse access.
7. **Exact interval metadata.** VFR intervals use rational `Time`, shifted by the decoder's tick so
   `[pts - tick, next - tick)` selects the same frame as `frame_displayed_at`; the last interval is
   open-ended. Metadata is capped at 4096 spans per path and the bound is tested.

**Timeline/preview consistency:** `VideoDecoder::frame_at` intentionally applies a half-frame seek
bias; `frame_displayed_at` returns the frame whose presentation interval contains the time. Preview,
export and the timeline now all use `frame_displayed_at` at exact project-frame starts. The regression
`preview_uses_the_displayed_source_frame_at_project_rate_mismatch` loops 24/25/30/29.97/59.94 and
checks a 24 fps source: project frame 1 shows source frame 0, not frame 1. The export regression
decodes project frame 1 and confirms counter 0.

**Remaining limitation:** tile spacing still uses the stream's reported average rate, not a full
VFR index. The cache is correct once an interval is decoded; the first tile in an unknown interval
costs one probe decode. This is acceptable for the current thumbnail path and is recorded in the
T1 report.

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
