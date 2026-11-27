# Timeline T1 — VFR end-to-end acceptance (2026-11-27)

Branch: `feat/timeline-zoom-phase2b` (from `1c46a9a`). Scope: timeline media sampling and
preview/export frame selection. No layout, visual design, third-stage interaction, or zero-copy work.

**Status: T1 closed.** VFR is no longer "supported by the decoder, not proven by a fixture".

## 1. What was generated

`arc_media::fixtures::write_vfr_counter_video` adds a generated VFR fixture to the existing counter
fixture set. It writes a lossless FFV1 counter base with `write_counter_video`, then remuxes it with
`setpts` so the output has deliberate uneven frame durations:

| Property | Value |
|---|---|
| File | generated MP4 (`*.mp4`), FFV1, 320×180, 30 frames, no audio |
| Nominal rate | 30 fps |
| Frame numbers | 0…29, unique, encoded in the top band by the existing counter writer |
| Gap | **0.2 s at frame 10 and frame 20** |
| PTS examples | 0.000, 0.300, **0.533**, 0.833, **1.067**, 1.367 s |
| Reported stream rate | `avg_frame_rate = 150/7` (~21.43), `r_frame_rate = 30` |
| VFR flag | `StreamInfo.vfr = true` (avg ≠ nominal) |
| Duration | 1.4 s |

MP4 is required for this fixture: the MKV muxer writes the nominal `DefaultDuration`, so its
`avg_frame_rate` stays 30 and the shim does not flag VFR even with uneven PTS.

Expected PTS are derived by the same public helper, `vfr_counter_pts_us`, so the test does not
hand-copy a second table:

```
pts_us(n) = round((n / 30 + 0.2 * floor(n / 10)) * 1_000_000)
```

## 2. PTS mapping

`vfr_counter_maps_every_project_frame_to_expected_pts` samples the 30 fps VFR source from projects
at **24, 25, 30, 29.97 and 59.94 fps**. For every project frame it asserts:

- the decoded `FrameRef::time` equals the fixture's expected PTS (within the 1 ms container tick);
- the counter read from the RGBA frame equals the expected source frame index;
- at least one source frame/PTS repeats across consecutive project frames (the 0.233 s gap spans
  seven 30 fps project frames).

The test drives `frame_displayed_at` directly, so it covers the same method the thumbnail path uses.

## 3. Cache sharing and bounded work

The original VFR path keyed every project time by microseconds (`FrameKey::TimeUs`), so project
frames inside one long source interval each issued a decode. T1 changes this to an interval cache:

- `VideoDecoder` now exposes **`FrameRef::next_time`** (the decoder's look-ahead PTS) through a tiny
  C accessor, `arc_av_video_next_pts`.
- `MediaHub` learns exact-rational `Time` intervals and keys decoded textures by **`FrameKey::Pts`**.
  The interval is `[pts - tick, next_pts - tick)`, mirroring `frame_displayed_at`, which adds one
  container tick to the requested time. This keeps the cache and decoder on the same side of a VFR
  boundary at sub-microsecond PTS values.
- A per-path `vfr_probe` guard allows one probe decode per unknown interval; other project frames
  wait for that probe instead of flooding the worker.
- Interval metadata is capped at **4096 spans per path** (oldest evicted first, ~160 KiB worst
  case). Eviction only costs a re-probe; textures and the LRU are untouched.

`vfr_thumbnail_cache_shares_one_texture_per_source_pts` requests 30 project frames at 30 fps:

| Metric | Result |
|---|---:|
| Project tiles requested | 30 |
| Distinct source PTS reached | 20 |
| Worker requests | **20** (one per source frame) |
| Cached thumbnails | **20** |
| Cache hits | **10** repeated tiles |
| `pending` maximum | ≤ 24 (`MAX_PENDING`) |
| `stats.decoded` before first UI poll | **0** |

The last row is the UI-thread guard: `MediaHub::video_frame` only enqueues; decoded frames are
counted and uploaded in `poll` on the UI thread. Header probing still opens a decoder on the UI
thread and remains backlog T3.

## 4. Thumbnail vs preview/export mapping

The first check found a real inconsistency in the **frame selector**:

- **Thumbnail:** `frame_displayed_at` returns the frame whose presentation interval contains the
  project time (one container tick of tolerance).
- **Preview/export:** used `frame_at`, which applies a **half-frame seek bias**. In the second half
  of a source frame, `frame_at` can advance to the next frame early.

The second source review found another inconsistency in the **sample time**:

- Preview/export sample the **start** of each project frame (`plan_frame` → `FrameRate::time_of_frame`).
- `paint_video` sampled the **middle** of the project-frame span: for 30 fps it used 50 ms for
  project frame 1. A 24 fps source has advanced by then, so the thumbnail showed source frame 1 while
  preview showed source frame 0.

**Fixes:**

1. Preview (`crates/arc-cli/src/preview.rs`) and export (`crates/arc-cli/src/export.rs`) now use
   `frame_displayed_at`, matching the thumbnail's interval rule. `frame_at` remains for
   seek/nearest compatibility.
2. `paint_video` (`crates/arc-app/src/timeline/mod.rs`) now calls
   `media::thumbnail_sample_time(frame_rate, tile, step_frames)`, which returns the exact rational
   `FrameRate::time_of_frame` start. `MediaHub::video_frame_at` carries that `Time` into the cache
   and derives CFR frame identity with `FrameRate::frame_at(…, Round::Floor)`, not `floor(t*fps)`.
   This also removes a 29.97/59.94 frame-start rounding hazard.

Concrete case: a 24 fps source in a 30 fps project at project frame 1 (`t = 1/30 s`). Frame 0 is
displayed until `1/24 s`; preview and thumbnail now both show source frame 0. Project frame 2
(66.7 ms) shows source frame 1.

Regressions:

- `preview_uses_the_displayed_source_frame_at_project_rate_mismatch` loops 24/25/30/29.97/59.94
  project rates with a 24 fps fixture and checks every sampled frame against the 24 fps source
  interval.
- `export_uses_the_same_displayed_source_frame_as_preview` exports project frame 1 and checks the
  decoded export counter is 0, matching preview.
- `thumbnail_source_frame_matches_displayed_at_cfr_rates_and_reverse` drives the real thumbnail
  worker/cache for 24/25/30/29.97/59.94 plus 24-in-30, in forward and reverse order, and compares
  the thumbnail's actual PTS *and* the fixture counter with `frame_displayed_at`.
- `thumbnail_source_frame_matches_displayed_at_vfr_boundaries_and_reverse` does the same around the
  VFR gap, the exact interval boundary, the last source frame, and reverse seeks.

## 5. CFR, VFR, project rates and extreme zoom

- **CFR:** 24/25/29.97/30/59.94 decode tests; the 24 fps-in-30 fps cache test; the new real-fixture
  1f thumbnail test checks 24/25/30/29.97/59.94 and reverse access.
- **VFR:** 24/25/30/29.97/59.94 project-rate mapping, repeated PTS, 20-texture cache, exact gap
  boundary, last frame (open-ended interval) and reverse access.
- **Different project rates:** the VFR mapping test loops 24/25/30/59.94; preview/thumbnail tests
  loop 24/25/30/29.97/59.94; ruler/zoom tests already cover the same rates.
- **Extreme zoom:** `thumbnail_tiles_step_by_whole_project_frames` now also asserts that at the
  200 px-per-project-frame ceiling a 128 px tile is exactly one project frame for 24/25/30/29.97/
  59.94 fps. No fractional step can skip or duplicate a frame at maximum zoom.
- **1f start-time unit test:** `thumbnail_sample_time_is_the_exact_project_frame_start` asserts
  project frame 1 at 30 fps is `1/30 s` (not the old 50 ms midpoint) for all five rates.
- **Interval metadata test:** `vfr_interval_metadata_is_half_open_and_bounded` checks the tick-shifted
  boundary and that 4100+ intervals stay capped at 4096.

## 6. Commands and results

Environment: Windows 11, GNU Rust 1.100, FFmpeg 9.1 LGPL on `PATH` and `ARCLINE_FFMPEG_DIR`.

| Command | Result |
|---|---|
| `cargo check -p arc-media -p arc-app -p arc-cli` | pass |
| `cargo test -p arc-media` | **35 passed**, 0 failed |
| `cargo test -p arc-app` | **63 passed**, 0 failed, 9 ignored |
| `cargo test -p arc-app -- --ignored` | **9 passed**, 0 failed (includes both VFR tests) |
| `cargo test -p arc-cli --test preview` | **17 passed**, 0 failed |
| `cargo test -p arc-cli --test render_e2e` | **6 passed**, 0 failed |

Full logs are kept locally as `reports/vfr-test-*.log` and `reports/vfr-check.log` (gitignored;
only this report and the test code are committed).

## 7. Remaining limitations

1. **Tile spacing is still based on the reported average rate**, not a full VFR index. The sampler
   steps by project frames using that average; the sample time, PTS mapping and cache are exact.
   Consequence: tile density in a very long VFR gap is conservative, but no wrong frame is shown and
   no duplicate decode occurs after the interval is learned.
2. **First probe per unknown interval costs one decode.** A direct seek into an unknown VFR region
   decodes once to learn its PTS and next PTS; subsequent project frames reuse the texture. This is
   bounded and acceptable for thumbnails; a future full VFR index would remove it.
3. **Header probing on the UI thread** remains T3, not T1. No frame decode happens there.
4. **VFR fixture is FFV1/MP4.** H.264/HEVC VFR streams were not separately generated; the detection
   and interval logic is codec-independent (container PTS + `avg_frame_rate`), and the FFV1 fixture
   keeps frame numbers lossless.
5. **No full-workspace test run** was repeated after the change. Targeted suites above are the
   regression evidence; the full workspace remains the broader CI gate.
6. Thumbnail visual density was not re-screenshotted: the UI layout and paint code are unchanged;
   only the source-frame/time mapping, cache key and interval metadata changed.
7. **`VfrIntervals` is capped, not discarded with the texture LRU.** After 4096 distinct source
   frames on one path, an old interval is evicted and re-probed if revisited; this is intentional
   and bounded, but a future full VFR index would make seeks O(1) without re-probing.

## 8. Files changed

- `crates/arc-media/csrc/arc_av.c` — `arc_av_video_next_pts`.
- `crates/arc-media/src/libav.rs` — `FrameRef::next_time`.
- `crates/arc-media/src/fixtures.rs` — VFR counter writer + expected PTS helper.
- `crates/arc-app/src/timeline/mod.rs` — `paint_video` samples the exact project-frame start through
  `thumbnail_sample_time`, not the frame midpoint.
- `crates/arc-app/src/timeline/media.rs` — `FrameKey::Pts`, exact-`Time` `VfrIntervals` with tick
  shift and 4096-span cap, `video_frame_at`, `thumbnail_sample_time`, CFR/VFR/reverse tests.
- `crates/arc-cli/src/preview.rs`, `crates/arc-cli/src/export.rs` — `frame_displayed_at`.
- `crates/arc-media/tests/decode.rs` — displayed-vs-seek-bias regression.
- `crates/arc-cli/tests/preview.rs` — 24 fps source in five project rates plus export consistency.
- `docs/timeline-technical-backlog.md` — T1 closed; T2/T3 unchanged.
