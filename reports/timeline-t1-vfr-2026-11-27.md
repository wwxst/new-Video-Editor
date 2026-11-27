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
at **24, 25, 30 and 59.94 fps**. For every project frame it asserts:

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
- `MediaHub` learns `[pts, next_pts)` intervals and keys decoded textures by **`FrameKey::Pts`**.
- A per-path `vfr_probe` guard allows one probe decode per unknown interval; other project frames
  wait for that probe instead of flooding the worker.

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

## 4. Thumbnail vs preview mapping

The check found a real inconsistency:

- **Thumbnail:** `frame_displayed_at` returns the frame whose presentation interval contains the
  project time (one container tick of tolerance).
- **Preview/export:** used `frame_at`, which applies a **half-frame seek bias**. In the second half
  of a source frame, `frame_at` can advance to the next frame early.

Concrete case: a 24 fps source in a 30 fps project at project frame 1 (`t = 1/30 s`). Frame 0 is
displayed until `1/24 s`, but `frame_at` aims at `1/30 + 1/48` and returns frame 1. The thumbnail
correctly shows frame 0.

**Fix:** preview (`crates/arc-cli/src/preview.rs`) and export (`crates/arc-cli/src/export.rs`) now use
`frame_displayed_at`, matching the thumbnail mapping. `frame_at` remains for seek/nearest
compatibility and is covered by its existing tests.

Regression: `preview_uses_the_displayed_source_frame_at_project_rate_mismatch` builds a 24 fps
counter source in a 30 fps project and asserts project frame 1 → source frame 0, project frame 2 →
source frame 1. `preview_frame_matches_the_export` and the export end-to-end suite still pass.

## 5. CFR, VFR, project rates and extreme zoom

- **CFR:** existing 24/25/29.97/30 fps decode tests and the 24 fps-in-30 fps cache test still pass.
- **VFR:** 24/25/30/59.94 project-rate mapping, repeated PTS, 20-texture cache, bounded pending.
- **Different project rates:** the VFR mapping test loops 24/25/30/59.94; ruler/zoom tests already
  cover 24/25/30/29.97/59.94.
- **Extreme zoom:** `thumbnail_tiles_step_by_whole_project_frames` now also asserts that at the
  200 px-per-project-frame ceiling a 128 px tile is exactly one project frame for 24/25/30/29.97/
  59.94 fps. No fractional step can skip or duplicate a frame at maximum zoom.

## 6. Commands and results

Environment: Windows 11, GNU Rust 1.100, FFmpeg 9.1 LGPL on `PATH` and `ARCLINE_FFMPEG_DIR`.

| Command | Result |
|---|---|
| `cargo check -p arc-media -p arc-app -p arc-cli` | pass |
| `cargo test -p arc-media` | **35 passed**, 0 failed |
| `cargo test -p arc-app` | **61 passed**, 0 failed, 7 ignored |
| `cargo test -p arc-app -- --ignored` | **7 passed**, 0 failed (includes both VFR tests) |
| `cargo test -p arc-cli --test preview` | **16 passed**, 0 failed |
| `cargo test -p arc-cli --test render_e2e` | **6 passed**, 0 failed |

Full logs are kept locally as `reports/vfr-test-*.log` and `reports/vfr-check.log` (gitignored;
only this report and the test code are committed).

## 7. Remaining limitations

1. **Tile spacing is still based on the reported average rate**, not a full VFR index. The sampler
   currently steps by project frames using that average; the PTS/cache itself is exact. Consequence:
   tile density in a very long VFR gap is conservative, but no wrong frame is shown and no duplicate
   decode occurs after the interval is learned.
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
   only the source-frame/time mapping and cache key changed.

## 8. Files changed

- `crates/arc-media/csrc/arc_av.c` — `arc_av_video_next_pts`.
- `crates/arc-media/src/libav.rs` — `FrameRef::next_time`.
- `crates/arc-media/src/fixtures.rs` — VFR counter writer + expected PTS helper.
- `crates/arc-app/src/timeline/media.rs` — `FrameKey::Pts`, `VfrIntervals`, `vfr_probe`, cache
  handling, VFR/CFR/zoom tests.
- `crates/arc-cli/src/preview.rs`, `crates/arc-cli/src/export.rs` — `frame_displayed_at`.
- `crates/arc-media/tests/decode.rs` — displayed-vs-seek-bias regression.
- `crates/arc-cli/tests/preview.rs` — 24 fps source in 30 fps project regression.
- `docs/timeline-technical-backlog.md` — T1 closed; T2/T3 unchanged.
