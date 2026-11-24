# 04 — Media Pipeline

Ingest must be tolerant (whatever the camera or phone produced) and egress must be exact (whatever the
deliverable spec says). Those are different problems, solved by different code paths that share FFmpeg's
demuxers and codecs.

## 1. Architecture

```
              ┌──────────────────────────── App process ─────────────────────────────┐
              │  arc-media (client)                                                  │
              │   ├── ProbeCache        (in-project metadata, fast path)             │
              │   ├── DecodeClient      (IPC to decode workers, request/response)    │
              │   ├── FrameSource       (seek+decode strategy, preroll, EOS handling)│
              │   └── ProxyManager      (generate/validate/select proxies)           │
              └───────────────┬──────────────────────────────────────────────────────┘
                              │ control pipe (bincode/CBOR) + shared-memory frames
              ┌───────────────▼──────────────────────────────────────────────────────┐
              │  arcline-decode (separate process, one per source group)             │
              │   ├── FFmpeg demux + decode (hwaccel, fallback to sw)                │
              │   ├── scopes: decode to shared mem, never to the app address space   │
              │   ├── hardened: rlimits, watchdog, no network, no file writes        │
              │   └── reports capability matrix at spawn                             │
              └──────────────────────────────────────────────────────────────────────┘
```

Process isolation is the central decision ([01 §3](01-system-architecture.md)). FFmpeg is a superb but
large C codebase; hostile or merely broken files are a daily reality for editors. A decode crash must
degrade one clip to "damaged media" state, never take the session down.

**Shared-memory frame transport:** worker writes planar YUV (NV12, P010, YUV422P10, YUV444P) into a ring
of N slots (default 8, ~64 MB at 4K), sends `{slot, pts, colorspace, dtype}` to the client; client hands the
slot to the uploader and returns it via a free-list. Frames are never copied through the control pipe.
Fuzzed by feeding truncated/corrupt files and asserting the parent survives.

## 2. Probe

`avformat_find_stream_info` is a known slow path (up to seconds on some network-mounted files). Strategy:

- On import: shallow probe (container + stream headers only, no per-frame decode) → enough to size the
  clip. Full probe (first/last keyframe, duration confidence, rotation matrix, HDR metadata, chapter list,
  color tags) runs in the background and updates the asset's cached metadata.
- Probe results are cached in the project (`asset.probe`) and in a per-user database keyed by
  `(size, mtime, first 4 MB hash)` — reopening a project never re-probes.
- **Rotation metadata** is normalized at import into the asset's `rotation` field and baked into the
  render node, never left to per-decoder quirks. Phone footage "just works."

## 3. Decode strategy

The hard problem is **seeking**, not playback throughput.

1. **Seek to keyframe ≤ target**, then decode forward to the exact frame. For long-GOP 4K HEVC on rotating
   media, this can be 30–80 frames — too slow for scrubbing.
2. **Optimized media (proxies).** On import of long-GOP footage above a configurable threshold, generate a
   proxy: half-to-quarter resolution, intra-frame (ProRes Proxy / DNxHR LB / H.264 all-intra), same
   timebase, audio copied (as AAC for size). Proxy generation is a background job with progress, never a
   modal block.
3. **Proxy/preview/original selection** is automatic per context: preview at Q0–Q1 uses original when
   cost permits; scrubbing and Q2+ use proxies; export always uses originals.
4. **Frame-exact seek cache** for scrubbing: the decoder keeps a small GOP cache (last decoded GOP,
   ~1 s of frames) so back-and-forth scrubbing in a region is O(1).
5. **Hardware decode first.** Probe order per platform: VideoToolbox (mac), D3D11VA/D3D12VA (Win),
   VAAPI/NVDEC (Linux) → software. Hardware path yields NV12/P010 directly; the GPU color-converts.
   Fallback is per-file: some files decode incorrectly on some drivers, so probe results include a
   verdict cache (`hw_ok`, `hw_broken`, `unknown`) keyed by `(codec, profile, driver version)`.

Audio decoding is straightforward by comparison; it runs in the decode worker too, and produces
48 kHz f32 planar blocks consumed by the audio engine's source readers.

## 4. Formats

**Ingest (v1 target):** MP4/MOV/M4V, MKV, WebM, AVI, MXF (basic, OP-1a/OP-Atom), transport streams, WAV,
AIFF, FLAC, MP3, AAC/M4A, Opus, Ogg; PNG/JPEG/TIFF/WebP stills; PNG/TIFF/DPX sequences; ProRes, DNxHR,
H.264/HEVC/AV1/VP9 video. Fragmented MP4 and phone HEVC-with-alpha are explicitly in scope. AVCHD and
older formats are "best effort" via FFmpeg demuxers.

**Egress:** see [03 §7](03-render-engine.md).

**Interchange:** EDL (CMX3600), FCPXML 1.10, OTIO (JSON) — import and export. These are adapters over the
document model, with lossy mapping documented per adapter; they are *not* the native format. FCPXML/OTIO
support makes Arcline a citizen of existing pipelines, which is a pragmatic adoption lever.

## 5. Hardware encode

`arc-encode` probes and ranks encoders per codec and platform, exposing a stable abstract preset
(`fast | balanced | quality | master`) that resolves to concrete parameters:

| Codec | Preferred | Fallback |
|---|---|---|
| H.264 | NVENC / QSV / AMF / VideoToolbox | libopenh264 (licensed build) |
| HEVC | NVENC / QSV / AMF / VideoToolbox | libkvazaar (if licensed) |
| AV1 | NVENC / QSV (Arc) / SVT-AV1 | SVT-AV1 (CPU, BSD) |
| ProRes | FFmpeg prores_ks (CPU) | — (no HW exists; threaded well) |
| DNxHR | FFmpeg dnxhd (CPU) | — |

Rate control: constant-quality (CQ/CRF-like) by default with bitrate caps; two-pass available for
hardware encoders that support it; loudness normalization and frame-rate conversion occur *before* encode.

**Frame-rate conversion policy (explicit):** conform (drop/dup) by default; optical-flow interpolation is
an optional effect, never a silent default.

## 6. Subtitles

- **Import:** SRT, WebVTT, ASS/SSA (subset: text + basic styling), TTML (basic). Cue timing is converted to
  the project timebase with documented rounding (floor to frame).
- **Model:** cues in a `SubtitleDoc`, edited as subtitle-track clips ([02 §3.7](02-domain-model-and-project-format.md)).
- **Burn-in:** render pass using the title renderer; style presets ship with the app; per-language tracks
  can be exported as sidecars (SRT/VTT) matching the burn-in timing exactly.
- **ASR hook (P3):** an agent can supply a transcript with word timings; `subtitle.from_transcript` builds
  cues. Arcline does not ship an ASR model — it accepts results from any tool. This keeps the binary small
  and the responsibility clear.

## 7. Encode orchestration and muxing

- Segments rendered in parallel workers ([03 §7](03-render-engine.md)); each worker writes an elementary
  stream plus a timing manifest; the muxer concatenates, adjusts timestamp offsets, and writes container
  metadata (rotation, color tags, HDR static metadata, chapters).
- Audio is rendered once, in the same process as the muxer, from the offline DSP path; the muxer interleaves
  with correct A/V offset (including any intentional delay from resampling).
- **Report:** `export_report.json` next to the output — codec parameters, encoder, segment hashes, loudness
  measurements, dropped/duplicated frame counts, renderer identity. This is the artifact agents and
  pipelines use to *verify* an export rather than trust it.

## 8. Caching layout

```
<project>.arcline/cache/
├── proxies/<asset>/<profile>.mp4
├── thumbs/<asset>/<frame>.jpg          # filmstrip, 2 Hz
├── wave/<asset>.peak                   # min/max pyramid, 256 samples/level
├── frames/<key>.jxl                    # L1 frame cache
└── shaders/<driver-hash>.bin           # pipeline cache (platform dir, not project)
```

All caches are regenerable and excluded from `collect` saves. Cache eviction obeys the global memory policy
([08 §3](08-performance-reliability.md)); proxies are kept while the asset is in the project and purged
when it is removed (with a grace period).

## 9. Licensing posture (summary; details in [11](11-risks-and-legal.md))

Ship a **custom FFmpeg build pinned to LGPLv2.1-compatible components only** (no `--enable-gpl`), dynamically
linked, with the LGPL compliance package (source offer, build scripts, relink instructions). H.264/HEVC
software encoding relies on hardware encoders plus a patent-licensed software encoder where the business
requires it. AV1 (SVT-AV1, BSD) is the royalty-free path and the recommended default for web delivery of
new projects. This is a business decision surfaced early because it constrains Phase 1 deliverables.
