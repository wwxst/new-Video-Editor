# Phase C implementation plan — editor shell and preview

**Status:** plan, 2026-11-26. **Scope source:** [docs/10 §5](../10-roadmap.md) (Phase C deliverables
and exit criteria), [docs/07](../07-ui-architecture.md), [docs/03 §3/§10](../03-render-engine.md),
[HANDOVER §7](../../HANDOVER.md). **First slice implemented in this change:** C1.

Phase B is closed (frame-exact render, malformed-media corpus, HEVC; CI is green on all four
jobs, runs 7 and 9), so the editor is the next milestone. This plan turns the Phase C deliverables into ordered, testable slices. It changes no
architecture: the same six crates, the same dependency rules ([docs/01 §4](../01-system-architecture.md)),
no schema or protocol change.

## 1. Scope and constraints

- **Goal:** a user can import, arrange, trim, split and scrub a project entirely in the GUI, with a
  preview that reflects the compositor.
- **In scope:** preview pipeline and scheduler, editor shell (docking, media bin, inspector, viewer),
  timeline canvas, transport controls, IME for CJK titles, text/title rendering.
- **Out of scope (unchanged):** F-10 path allow-lists and F-15 panic containment (later batches);
  software HEVC (x265/GPL stays out); new project-schema fields; MCP/agent surface (Phase F).
- **Crate rule:** `arc-render` may not decode (no FFmpeg) and `arc-media` may not know about the UI.
  A pipeline that decodes *and* composes therefore lives in `arc-cli` (the session layer, which
  already depends on media + render and which `arc-app` may use), and the GUI consumes its frames.
- **CI constraint:** GPU tests must run on the WARP software adapter (GitHub runners); tests that
  create devices in parallel are serialized per binary (see `arc-render/tests/render.rs`).

## 2. Milestones

Ordered by dependency; each milestone is one slice with its own tests and a green suite.

### C1 — Preview frame pipeline (headless)

The engine side of the viewer: compose one RGBA frame for a timeline frame, with per-asset decoders
and a bounded decoded-frame cache. Implemented in `arc_cli::preview`, no GUI.

**Acceptance criteria**
1. `PreviewEngine::frame(n)` returns the same RGB pixels as the export of the same project and frame
   (FFV1 lossless round trip), at full quality.
2. `PreviewQuality::Half` returns a frame at half the sequence resolution while still decoding the
   source at its native size.
3. A repeated request for the same frame is a cache hit and performs no decode; a one-frame step
   decodes exactly one new source frame (instrumented counters in `PreviewStats`). The cache is
   bounded by **bytes** (LRU, default 256 MB), so a 4K frame does not blow the budget; eviction is
   covered by a test with a one-frame budget.
4. Unavailable assets and unsupported items (titles today) are reported in `warnings` and the frame
   is still produced (background for the missing source) — the export's behaviour.
5. Tests run on the WARP adapter; no device is created concurrently inside a test binary.

C1 caches **decoded source frames** (key: asset + source time), because the compositor applies
effects after decode. The GPU-side composited-frame cache of [docs/03 §3](../03-render-engine.md)
(keyed by clip, effect chain and parameters) is part of C2/C3, where the scheduler and the viewer
own GPU textures.

### C2 — Preview scheduler and playhead

A request scheduler between the UI thread and the pipeline: latest-wins coalescing, half quality
while scrubbing, full quality when the playhead settles, bounded queue, no UI-thread blocking.
**Status:** implemented in `arc_cli::preview_scheduler` (commit set for this slice; see
[reports/phase-c-c2-2026-11-26.md](../reports/phase-c-c2-2026-11-26.md)).

**Acceptance criteria**
1. With a fired sequence of playhead positions, only the latest pending position is rendered.
2. The scheduler switches to full quality after N ms of quiescence (injectable clock), and a
   placeholder audio clock (docs/10 §5) provides the playback time until Phase D's real clock.
3. A request that outlives a newer one is dropped; the returned frame always matches the last
   requested position.
4. Unit-testable with a stub pipeline (no GPU).

### C3 — Editor shell and viewer texture

`arc-app`: replace the PoC workload with the editor shell — docking layout, media bin, inspector
generated from `EffectManifest`, viewer that uploads `PreviewEngine` frames as an `egui_wgpu` user
texture; transport (play/pause/step), keyboard focus and IME.

**Acceptance criteria**
1. The app launches (`--editor`), opens a project, shows the media bin and inspector.
2. The viewer displays the composed frame at the playhead and updates on scrub/step.
3. IME composition works for CJK title text on Windows (manual check, recorded in the report).
4. A headless smoke test drives the shell's state machine (no window) for open/step/edit.
5. The viewer texture is the GPU-side composited-frame cache of
   [docs/03 §3](../03-render-engine.md) (key: clip, source frame, effect chain hash, resolved
   parameter hash; 2 GB budget), layered on the C1 decoded-source cache.

**Status (2026-11-26):** viewer + transport implemented (`arc-app` shell, `arc_cli::preview_transport`),
with a minimal composited-frame LRU; the docs/03 §3 per-clip GPU texture cache is **deferred** to the
next compositor step (needs per-item render targets in `arc-render`). First benchmark: 1080p, 4 tracks,
8 effects → 29.92 fps full / 87.84 fps half on the Intel iGPU — 0.3 % below the 30 fps exit criterion,
recorded as a gap. Details: [reports/phase-c-c3-2026-11-26.md](../reports/phase-c-c3-2026-11-26.md).

### C4 — Timeline canvas

Paint, virtualization and LOD, hit testing, move/trim/split, snapping; gestures become command
batches through `EngineHost::apply_async`.

**Acceptance criteria**
1. Pure hit-test/snap functions are unit-tested against a golden timeline (tracks, clips, zoom).
2. A drag/trim/split gesture produces exactly one valid command batch; invalid drops are refused.
3. The canvas renders 100 clips at 60 fps in the reference machine's editor (measured).

### C5 — Titles and text renderer

`cosmic-text` + `glyphon` (or an equivalent) in `arc-render`; title clips render instead of being
skipped; LUT asset parameter.

**Acceptance criteria**
1. Title clips appear in preview and export; the export report no longer warns "titles are skipped".
2. CJK text renders with the system fallback font; an image-hash golden test pins a sample.
3. LUT manifests either gain an asset parameter or keep being reported unsupported (decided).

### C6 — Polish: command palette, keymap, dialogs

**Acceptance criteria:** palette lists commands from the same registry the engine validates; keymap
is data-driven and tested; native dialogs (open/save/export) go through the session layer.

## 3. Risks and mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| CPU decode limits preview rate (no GPU YUV upload yet, docs/03 §10) | preview below 30 fps on 4-track/8-effect timelines | measure in C1/C2; half-quality path while scrubbing; decode cache; GPU YUV upload as a follow-up optimization |
| GPU tests on WARP crash when devices are created concurrently (found in batch 3) | red CI | serialize per test binary (already the pattern in `arc-render/tests/render.rs`); keep preview tests in one serialized binary |
| Preview and export diverge | the viewer lies | C1 acceptance 1 (pixel equality with the export path) and the shared `plan_frame`; any future composition change must update both tests |
| UI-thread stalls on decode | unresponsive editor | C2 scheduler with worker threads and latest-wins; no decode on the UI thread |
| Half-quality scrub differs in pixel-space effect radii | the scrub view is not an exact downscale of the final frame | documented C1 characteristic: the compositor scales geometry but not blur/sharpen radii; C2 scales pixel-space parameters for the scrub path |
| Mid-stream decode failures (a frame fails after the file opened) | preview skips that item | the import check + malformed corpus cover the boundary; C1 warns on open failure and per-frame errors, but a deterministic frame-level failure fixture is still missing (noted in the follow-up tests) |
| Text rendering pulls new dependencies | licence/MSVC surface | evaluate `cosmic-text`/`glyphon` licences in C5 before adding; docs/11 checks |

## 4. Rollback

Every milestone is additive inside the existing crates; none changes the project schema, journal,
JSON-RPC or CLI contracts. A slice can be reverted independently (C1 is a new module plus tests; the
GUI shell lands behind the existing `--poc-seconds`/embedding paths until C3 is complete).
