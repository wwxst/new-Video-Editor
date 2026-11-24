# 03 — Render Engine

The render engine turns an immutable `Project` snapshot plus a time `t` into a frame, and a snapshot plus
an export spec into a file. Preview and export share one compositor.

v2 simplification: there is **no generic render graph**. The compositor is a direct, readable function.
A graph, planner, and texture aliasing were designed for many execution strategies; v1 has one. Add them
back only when a second consumer proves the need.

## 1. Device layer

`wgpu` provides Vulkan/Metal/D3D12 behind one API. The app uses the `wgpu::Device`/`Queue` created by
`egui-wgpu`, so the UI and the viewer share one device, one queue, and one adapter — no interop copies
between UI and renderer.

`arc-render::Device` is a thin wrapper that owns:

- **Adapter selection** with a preference list (discrete → integrated → software).
- **Feature probe**: fp16, timestamp queries, storage textures. Used ad hoc per pass; there are no
  pre-negotiated capability tiers.
- **Device-loss recovery**: on `DeviceLost`, drop GPU resources, recreate the device, rebuild pipelines,
  re-render the current frame. The document lives on the CPU and is untouched. Target recovery ≤ 2 s.
- **Pipeline cache**, keyed by `(driver id, pipeline hash)`, persisted to disk to avoid first-run hitches.

The device layer knows nothing about timelines. It offers texture creation, pipeline creation, submit, and
readback.

## 2. The compositor

```
fn render_frame(project, t) -> FrameTexture:
    target = pool.acquire(resolution)          # RGBA16F
    clear(target, sequence.background)
    for track in video_tracks(bottom → top):   # active at t
        layer = pool.acquire(resolution)
        for clip in active_clips(track, t):
            src  = frame_source(clip, t)       # cache or decode, source time resolved
            tex  = pool.acquire(resolution)
            draw_transform(src, clip.transform_at(t), tex)   # affine + opacity
            for effect in clip.effects:        # ordered chain
                tex = effect.apply(tex, effect.params_at(t))
            blend(tex, layer, clip.blend, clip.opacity_at(t))
        if transition active on track:
            blend the two neighbors over the transition curve
        blend(layer, target, track.blend, track.opacity)
    draw_overlays(target, subtitles, titles)   # text is a separate pass
    resolve_to_rec709(target)                  # linear → display
    return target
```

Notes:

- **Texture pool** with ping-pong buffers; a texture returns to the pool when its last reader has been
  submitted. Peak footprint at 1080p is modest (≈ 40 buffers × 8 MB); 4K is bounded by the memory policy
  ([08 §3](08-performance-reliability.md)).
- **Half-resolution intermediates** are used for heavy effects (blur chains) where precision allows.
- **Transitions** evaluate two clips over the overlap window using source handles; the underlying clip
  timing is not rewritten ([02 §3.5](02-domain-model-and-project-format.md)).
- **One pass per clip effect**, fragment shader, parameters in a uniform buffer. Compute is used only where
  it clearly wins (separable blur), not as a default.
- The compositor is **stateless between frames**; rebuilding the per-frame work list is O(active clips)
  and costs microseconds.

## 3. Frame cache

Single-level, in-memory, GPU-side:

```
key = H(clip_id, source_frame_index, effect_chain_hash, resolved_param_hash)
```

- LRU texture cache sized by the memory budget (default 2 GB).
- Automatic invalidation: a trim or parameter change alters the key for affected nodes; untouched entries
  survive.
- No disk-backed L1 cache in v1. Thumbnails and waveforms cover the scrub filmstrip ([04 §6](04-media-pipeline.md)).
- Scrubbing renders at half resolution; a full-quality render follows when the pointer settles.

## 4. Built-in effects (v1)

Six effects, each a WGSL pass with a manifest ([02 §3.4](02-domain-model-and-project-format.md)):

| Effect | Parameters |
|---|---|
| `core.color.correct` | exposure, contrast, saturation, temperature |
| `core.color.lut` | LUT asset, input/output space |
| `core.blur.gaussian` | radius, quality tier |
| `core.key.chroma` | key color, tolerance, softness, spill suppression |
| `core.stylize.sharpen` | amount, radius |
| `core.stylize.vignette` | amount, midpoint, roundness |

Transform, crop, and opacity are clip properties, not effects. Titles and subtitles use the text renderer.
More effects (glow, grain, corner-pin, curves UI) arrive in Phase G or later; the manifest format is stable.

**Shader ABI** (versioned, small on purpose):

```wgsl
struct ParamBlock { params: array<vec4<f32>, 8>, lut: texture_2d<f32> };
@fragment fn shade(in: VsOut, u: &ParamBlock) -> @location(0) vec4<f32>
```

The manifest is the single source of truth for UI controls, MCP parameter schemas, and validation.

## 5. Text and titles

Text is a first-class render path, not a UI-generated texture:

- Shaping via `cosmic-text`/`rustybuzz`; rasterization to a glyph atlas; drawing via `glyphon`.
- A title is a scene: styled runs (font, size, weight, color, tracking, outline, shadow), layout box,
  anchor, and keyframable properties.
- Subtitle burn-in reuses the renderer: each cue maps to a text scene with a shared style.
- Text parameters travel through the same animation mechanism as effects, so agents can place and style
  titles with the same tools.

## 6. Preview scheduler and A/V sync

Playback is audio-driven; the audio device clock is the master ([05 §4](05-audio-engine.md)).

```
audio thread:    [48k samples …] → device → clock t_audio
video preview:   target = t_audio + output_latency_compensation
                 if frame cache hit → present
                 else → request render; if deadline missed → repeat last frame
quality:         if p95 frame time exceeds budget for 500 ms → drop to half resolution
                 if comfortably under budget for 3 s → return to full resolution
```

- **Two quality levels only** (full and half resolution), with asymmetric hysteresis: fast to degrade, slow
  to recover.
- **Frame dropping, never stalling.** A late frame is dropped; the decoder gets priority for the next
  request. Present-on-demand when paused.
- The governor state is visible in the status bar and via `preview` telemetry, so performance problems are
  diagnosable.

## 7. Export pipeline

One export process (`arcline-cli render`, spawned as a child by the GUI or by `job.submit` —
[04 §1.2](04-media-pipeline.md)), sequential frames, hardware encoder — simpler and fast enough:

```
render frames in range → GPU readback ring (2–3 frames) → encoder → mux
audio: offline mix of the same DSP graph → encoder
mux: interleave, write container, metadata, loudness/format report
```

- A readback ring overlaps GPU rendering with encode, so 1080p export exceeds realtime on hardware encoders.
- Parallel segment rendering (multiple processes, checkpoints) is a documented post-1.0 optimization; it
  is not needed for v1 workloads and costs substantial complexity.
- **Export targets (v1):** MP4/MOV with H.264/HEVC (hardware), AV1 (SVT-AV1 software), MOV with ProRes
  422/422HQ (CPU), WAV/AIFF audio. Image sequences and additional mezzanines later.
- Each export writes `export_report.json`: encoder, codec parameters, frame count, loudness if measured,
  renderer version, dropped frames. Agents and pipelines verify exports with this artifact.

## 8. Color

- **Internal:** RGBA16F linear light. One sequence working space: Rec.709.
- **Ingest:** each asset declares transfer/primaries (auto-detected, user-overridable); conversion to
  working space happens in the source step — one place, not scattered across effects.
- **Grading:** effects operate in linear working space. LUTs declare their expected input space and are
  converted on load.
- **Egress:** linear → Rec.709 display transform. HDR10/HLG, ACES, and OCIO are post-1.0
  ([09](09-effects-and-extensions.md)).
- **10-bit sources** decode and composite at 16F precision; the 8-bit output is dithered. 10-bit encode
  (HEVC) is available where the hardware supports it.

## 9. Determinism

The honest, testable contract:

1. **Same machine, same driver, same renderer version ⇒ bit-exact output.** Rendering twice yields identical
   frames; CI asserts this on every PR.
2. **Across machines ⇒ tolerance-bounded.** Golden tests compare with PSNR ≥ 45 dB. Effects with
   non-associative math are documented as excluded.
3. No reference-render mode, no cross-device bit-exactness claim. Every export report records renderer
   version and shader-cache hash so a result can be attributed.

Golden frames are generated from fixture media ([08 §6](08-performance-reliability.md)); the fixtures and
expected images live with the render crate.

## 10. GPU upload strategy

- Decoded frames arrive as planar YUV (NV12/P010); the renderer uploads the smallest planes and performs
  color conversion on the GPU.
- A staging ring of 3–4 frames per decoder hides upload latency; uploads overlap graph execution.
- Zero-copy platform interop (D3D12 shared textures, IOSurface) is a later optimization. v1 uses the
  uniform shared-memory-free path: decode → upload → convert.
