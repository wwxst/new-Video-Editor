# 03 — Render Engine

The render engine turns an immutable `Project` snapshot plus a time `t` into a frame, and turns a snapshot
plus an export spec into a file. Preview and export share the same graph; only the scheduler and quality
policy differ.

## 1. Device layer

`wgpu` provides Vulkan/Metal/D3D12 behind one API; Arcline wraps it in `arc-render::Device` to guarantee:

- **Capability tiers**, negotiated once at startup:
  - *Tier A* (desktop discrete/RDNA/Apple silicon): storage textures, fp16, 8× MSAA, multi-queue.
  - *Tier B* (integrated, older GPUs, WASM-less min spec): 4× MSAA, no fp16 fast paths.
  - *Tier C* (software: lavapipe / WARP): correctness only, used for CI goldens and screenshots.
- **Device loss recovery.** `DeviceLost` → drop all GPU resources, recreate device, rebuild shader cache,
  re-render current frame. The document is untouched because it lives on the CPU. Target: recovery ≤ 2 s,
  user sees a one-second overlay.
- **Deterministic resource ordering.** Pipeline and bind-group creation is cached and ordered by a stable
  key so that frame capture and golden tests are reproducible per driver.

The device layer knows nothing about timelines. It offers: `create_texture`, `create_pipeline`,
`submit(graph)`, `readback`.

## 2. The frame graph

A pull-based graph, rebuilt per frame (cheap: nodes are cached by structural key; only parameters change).

```
                          ┌──────────── Output(Target | Encode) ───────────┐
                          │ Resolve(color space → target)                  │
                          │  ├─ Overlay: subtitles / titles / safe guides  │
                          │  └─ Compositor                                 │
                          │       ├─ Track V3 ─ EffectChain ─ Transform ─┐ │
                          │       ├─ Track V2 ─ EffectChain ─ Transform ─┤ │
                          │       ├─ Track V1 ─ EffectChain ─ Transform ─┤ │
                          │       │        └─ Source(t) ─ decode/cache ─┘ │
                          │       └─ Transition evaluation between A/B     │
                          └────────────────────────────────────────────────┘
```

Node types:

| Node | Function | Notes |
|---|---|---|
| `Source` | fetch frame from cache or decoder at resolved source time | outputs RGBA16F linear texture |
| `Transform` | affine + opacity, evaluated from keyframes | single instanced quad pass |
| `Effect` | one effect pass (may internally chain several dispatches) | params in UBO, LUTs/curves in textures |
| `Blend` | per-track compositing mode (normal, add, screen, …) | fixed-function blend where possible |
| `Transition` | A/B mix over a window | reads both, one pass, weight curve |
| `Overlay` | titles, subtitles, UI-safe guides | shaped text → glyph atlas → quads |
| `Resolve` | working space → display/encode space | 3D LUT or matrix+transfer per mode |
| `Output` | present to swapchain or readback to encoder | dual output of video + alpha if needed |

Scheduling: nodes execute in dependency order with texture-pool reuse; a frame uses a fixed pool of
`N = ceil(inflight_frames) + 2` ping-pong textures per format. Typical 1080p project: ~40 textures ×
RGBA16F ≈ 640 MB peak — controlled by the memory manager ([08 §3](08-performance-reliability.md)) and half-res
intermediates where precision allows.

**Why pull-based, not a retained DAG:** a frame's graph depends on the time `t` (which clips are active,
which transitions are in-window, which keyframes matter). Rebuilding from the snapshot per frame is
O(active nodes) with cached pipeline objects; retaining a graph invites stale-state bugs. The cost is
microseconds; the correctness win is large.

## 3. Frame cache

Two-level, keyed by **content**, not by time:

```
key = H(node_kind, clip_id, effect_version, resolved_param_hash, source_frame_index, renderer_version)
```

- **L0 — GPU texture LRU** per render thread, sized by the memory budget (default 2 GB): frames and
  effect outputs that survive a seek.
- **L1 — CPU/disk**, used for thumbnails, scrubbing filmstrip, and export re-runs; disk entries are
  compressed (BCn or JPEG-XL for color-managed stills) under `cache/frames/`.

Invalidation is automatic: any parameter or trim change changes `resolved_param_hash` and/or
`source_frame_index` for affected nodes; untouched regions keep their entries. There is no manual
"clear cache" logic in the correctness path — only a user-facing "purge" button.

Scrub behavior: the UI requests frames at pointer rate; the render thread renders at half resolution while
scrubbing (a scrub-quality flag propagates into the graph, with edge-preserving downsample), and a
follow-up full-quality render is queued for the settled position.

## 4. Built-in effect set (v1)

Curated, not exhaustive. Every effect is a WGSL pass with a manifest ([02 §3.4](02-domain-model-and-project-format.md)):

| Category | Effects |
|---|---|
| Color | color correct (exposure/contrast/saturation/temp/tint), curves (RGB + luma), 3D LUT, white balance, lift-gamma-gain |
| Blur | gaussian (separable, quality tiers), directional, box |
| Stylize | sharpen (unsharp), glow/threshold bloom, vignette, film grain |
| Warp | crop, mirror, corner-pin (perspective), lens distortion correction |
| Key | chroma key (desaturate-to-alpha, spill suppression), luma key |
| Time | freeze frame, speed ramp (evaluated via source mapping, not a separate pass) |
| Utility | alpha handling, premultiply, test pattern |

Each has: parameter manifest, 2–4 quality tiers, golden-frame tests at tiers, and a documented GPU cost
class used by the preview quality governor.

## 5. Effect authoring model

An effect is (a) one or more WGSL entry points, (b) a JSON manifest, (c) optional CPU reference
implementation for tests and Tier-C fallback. The manifest is the single source of truth for UI controls,
MCP parameter schemas, and validation (`effect.describe` returns it verbatim).

```rust
// Shader ABI (stable, versioned)
struct ParamBlock { params: array<vec4<f32>, 8>, // packed by manifest layout
                    lut: texture_2d<f32> }        // optional
@fragment fn shade(in: VsOut, u: &ParamBlock) -> @location(0) vec4<f32>
```

The ABI is deliberately small so that OFX/WASM bridges and agent-generated WGSL can target it. Versioned
`shader_abi: 1` in the manifest; bumping it is a breaking change across the plugin ecosystem.

## 6. Preview scheduler and A/V sync

Playback is **audio-driven**. The audio clock is the master; the video scheduler targets the audio
presentation timestamp.

```
Audio thread:      [48k samples …] → device → clock t_audio
Video scheduler:   target = t_audio + output_latency_compensation
                   if frame_cache hit → present
                   else → request render; if deadline missed, repeat last frame (or drop)
Quality governor:  if p95 frame time > budget for 500 ms →
                     step down: full → half-res → effects-lite → proxy-resolution
                   if p95 < 60% of budget for 3 s → step back up one level
```

Quality ladder (predictable degradation, never a stutter spiral):

| Level | What changes |
|---|---|
| Q0 | full resolution, all effects, full-quality blur |
| Q1 | effect blur quality −1 tier, half-res intermediates for blur chains |
| Q2 | proxy media (if exists) at full preview res |
| Q3 | half preview resolution |
| Q4 | quarter preview resolution, effects capped at "lite" passes |

The governor's hysteresis is deliberately asymmetric (fast to degrade, slow to recover) so a heavy section
does not oscillate. Level and reason are exposed as UI state and via `preview.status` for agents.

**Frame presentation policy:** present on VRR-aware swapchain when available; otherwise fixed-rate
present with frame duplication. Never block the GPU waiting for a late decode; a late frame is dropped, and
the decoder gets higher priority for the next request.

## 7. Export pipeline

Export must be faster than real time by a large factor and must be reproducible.

```
Export spec → plan:
  1. Split timeline into segments aligned to keyframe/GOP boundaries of the target encoder
     (or fixed 2 s segments for intra-only codecs like ProRes).
  2. Render segments in parallel: N render processes (not threads) each with its own GPU context,
     bounded by --max-workers and VRAM probe. Each writes an elementary stream + timing sidecar.
  3. Audio rendered once offline by the same DSP as playback (guaranteed loudness parity),
     optionally two-pass loudness normalization (EBU R128).
  4. Mux: concatenate segments, interleave, write container, compute checksums.
  5. Verify: duration, frame count, loudness, black-frame and silence detection (optional), report.
```

Why processes for export segments: GPU drivers are the least reliable software in the stack; a hung
encoder or crashed driver in worker 3 of 8 should cost one segment, not the export. Each worker checkpoints
its segment; resume continues from completed segments.

Encoder selection is a capability probe, not a config guess: `arc-encode` probes NVENC/QSV/AMF/VideoToolbox
availability and version, picks the best available for the requested quality/speed preset, and records the
choice in the export report so a result can be attributed.

**Export targets (v1):**

| Container | Video | Audio | Notes |
|---|---|---|---|
| MP4/MOV | H.264, HEVC (10-bit), AV1 | AAC, ALAC, PCM | hardware preferred; software fallback |
| MOV | ProRes 422/422HQ/4444 | PCM | editorial/mezzanine deliverable |
| MKV | anything FFmpeg can mux | anything | power users |
| Image sequence | PNG, TIFF, DPX (selectable bit depth) | — | VFX handoff |
| WAV/AIFF/FLAC | — | audio-only | stem/export audio |

## 8. Determinism contract

This is the honest version, because cross-vendor GPU bit-exactness is not achievable and claiming it would
poison trust in the platform.

1. **Same device class + same driver + same Arcline version ⇒ bit-exact output.** Frames are reproducible;
   the same export run twice yields identical hashes for video and audio (asserted in CI on lavapipe, and
   on hardware for one Win/NVIDIA, one macOS/Apple, one Linux/AMD runner).
2. **Across devices ⇒ tolerance-bounded.** Tests compare with PSNR ≥ 50 dB (and max per-channel difference
   ≤ 2 LSB at 10-bit) for pure-GPU paths. Effects with iterative/non-associative math are excluded from the
   cross-device requirement and documented as such.
3. **Reference mode.** `--render-mode=reference` forces fp32, forbids fast-math, pins filter kernels, and
   runs on the CPU/lavapipe path at reduced speed. Golden tests use reference mode; it is the arbiter when
   hardware paths disagree.
4. **Attribution.** Every render records `{renderer_version, shader_cache_hash, driver_id, adapter_name,
   encode_backend}` in the export report. A frame can always be explained.

The contract is enforced by `arc-testkit`: a corpus of 120 golden clips (cuts, transitions, keyframes,
color, text, transparency, audio) rendered per PR on lavapipe and nightly on hardware.

## 9. Color pipeline

- **Internal:** RGBA16F, linear light. Working space per sequence: Rec.709 / Rec.2020 / ACEScg.
- **Ingest:** each asset declares its transfer/primaries (auto-detected, user-overridable). Conversion to
  working space happens in the `Source` node — one place, not scattered across effects.
- **Grading:** effects operate in the working space. LUTs declare their expected input space and are
  converted on load.
- **Egress:** `Resolve` applies output transform (matrix + transfer, or a 3D LUT for HDR) per target:
  SDR Rec.709, HDR10 (PQ, Rec.2020), HLG. Tone mapping for SDR-from-HDR is an explicit, selectable policy,
  not a hidden default.
- **Interchange:** an optional OCIO build feature maps working-space operations to an OCIO config for
  studios that require it. Default build uses our native transforms to keep startup deterministic.

## 10. GPU-memory and upload strategy

- Decoded frames arrive from the decode subprocess in shared memory as YUV (typically NV12/P010) — the
  renderer performs **color conversion on the GPU** and uploads the smallest planes, not RGBA.
- A staging ring buffer of 3–4 frames per uploader hides latency; uploads are `map_async` and overlap with
  graph execution.
- Zero-copy is used only where the platform permits (D3D12 shared textures on Windows, IOSurface on macOS
  for VideoToolbox); elsewhere the shared-memory + upload path is the default, because it is uniform and
  crash-isolatable.
- Large intermediates (4K RGBA16F = 66 MB) are allocated from a pool with aliasing: a texture is reused
  for a later node the moment its last reader has submitted. Peak footprint is pruned by the graph planner.

## 11. Text and titles

Text rendering is a first-class render path, not a texture generated by the UI:

- Shaping via `rustybuzz` + `fontdb`, rasterization to a glyph atlas (`cosmic-text`-style pipeline, our
  own atlas manager for GPU instancing).
- A title is a scene: styled runs (font, size, weight, color, tracking, outline, shadow, background box),
  layout box, anchor, and keyframable properties. Titles are editable through the same effect-parameter
  mechanism as everything else, so agents can place and style them with the same tools.
- Subtitle burn-in reuses the same text renderer; SRT cues map to a `Title` scene per cue with a shared
  style. Bitmap subtitle formats (PGS) decode to textures and composite directly.
