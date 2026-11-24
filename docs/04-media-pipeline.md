# 04 — Media Pipeline

Ingest must be tolerant of whatever a camera or phone produced; egress must be exact. Both are built on
FFmpeg, used in two ways: `libav*` in-process for frame-accurate decode and encode, and the `ffmpeg` CLI for
one-shot batch jobs.

## 1. Architecture

```
arcline / arcline-cli process
├── arc-media
│   ├── ProbeCache        (project-cached metadata, fast path)
│   ├── Decoder           (per-clip libav decode, hardware-first, panic-isolated)
│   ├── Encoder           (libav mux + hardware encoders)
│   └── BatchJobs         (proxies, thumbnails, waveform peaks via ffmpeg CLI)
└── decode threads        (pool; frames → GPU upload via arc-render)
```

**In-process decode in v1.** FFmpeg runs inside the application process behind a `Decoder` trait. A decoder
panic is caught at the call boundary and converted to a `MediaError` for that clip; other clips continue.
Autosave and the journal bound the damage. This is a deliberate trade: a subprocess with shared-memory
frame transport is the right long-term hardening ([ADR-0003](adr/0003-ffmpeg-for-media-io.md)), but it is
days of plumbing that produces no v1 capability. The trait is shaped so isolation can be added without
changing callers.

## 2. Probe

- On import: shallow probe (container + stream headers) → enough to place the clip.
- Background: full probe (duration confidence, first/last keyframe, rotation matrix, color tags, chapter
  list) updates the asset metadata.
- Results are cached in the project (`asset.probe`) and keyed by `(size, mtime, first 4 MB hash)`; reopening
  never re-probes.
- **Rotation metadata** is normalized into the asset at import and applied in the compositor; phone footage
  just works.

## 3. Decode strategy

Seeking, not throughput, is the hard problem.

1. **Seek to keyframe ≤ target**, then decode forward to the exact frame. Long-GOP 4K HEVC can require
   30–80 frames of decoding.
2. **Proxies** (below) make scrubbing cheap when the source is expensive.
3. **GOP cache**: each decoder keeps the last decoded GOP (~1 s) so back-and-forth scrubbing in a region is
   O(1).
4. **Hardware decode first**: D3D11VA/D3D12VA (Windows), VideoToolbox (macOS), VAAPI/NVDEC (Linux) →
   software fallback. A per-file verdict cache `(codec, profile, driver version)` remembers files that
   decode incorrectly on a given driver.
5. **Exactness**: the decoder is driven by frame indices derived from the project timebase; no rounding at
   the source boundary.

Audio decoding runs on the same threads and produces 48 kHz f32 blocks for the audio engine.

## 4. Formats

**Ingest (v1):** MP4/MOV/M4V, MKV, WebM, AVI, MXF (basic), transport streams; WAV, AIFF, FLAC, MP3, AAC,
Opus; PNG/JPEG/TIFF/WebP stills; ProRes, DNxHR, H.264/HEVC/AV1/VP9. Fragmented MP4 and phone HEVC are in
scope; older formats are best-effort via FFmpeg demuxers.

**Egress (v1):** MP4/MOV (H.264, HEVC, AV1), MOV ProRes, WAV/AIFF.

**Interchange** (EDL/FCPXML/OTIO) is post-1.0.

## 5. Hardware encode

`arc-media::encode` probes encoders at startup and ranks them per codec:

| Codec | Preferred | Fallback |
|---|---|---|
| H.264 | NVENC / QSV / AMF / VideoToolbox | none shipped except hardware |
| HEVC | NVENC / QSV / AMF / VideoToolbox | none shipped except hardware |
| AV1 | NVENC / QSV / SVT-AV1 | SVT-AV1 (CPU, BSD) |
| ProRes | FFmpeg prores_ks (CPU) | — |
| DNxHR | FFmpeg dnxhd (CPU) | — |

Presets are semantic (`fast | balanced | quality | master`) and resolve to concrete parameters per
encoder. Constant-quality rate control by default, bitrate caps optional. Software H.264/HEVC encoders are
not shipped for licensing reasons ([11 §2](11-risks-and-legal.md)); every machine with a GPU encoder is
fully served, and AV1 covers the royalty-free case.

Frame-rate conversion is conform (drop/duplicate) by default; interpolation is never silent.

## 6. Batch jobs via the `ffmpeg` CLI

One-shot work does not belong in the interactive code path. The app shells out to its bundled `ffmpeg`
binary (or a system binary on Linux) for:

- **Proxies**: half-resolution, intra-frame (ProRes Proxy / DNxHR LB / H.264 all-intra) with audio.
- **Thumbnails**: 2 Hz filmstrip JPEGs for the timeline.
- **Waveform peaks**: decode to PCM, compute a min/max pyramid in Rust (`ffmpeg -f s16le -`).
- **Audio conversion** for unusual codecs at import.

Jobs run in the background with progress, are cancelable, and are never on a modal path. This removes a
large amount of encoder-wiring code from `arc-media`.

## 7. Export orchestration

Single process:

```
for each output frame:
    texture = renderer.render(t)        # arc-render, offscreen target
    pixels  = readback(texture)         # 2–3 slot ring
    encoder.write_video(pixels, t)
audio_track = audio.mix_offline(range)  # arc-audio, same DSP as preview
encoder.write_audio(audio_track)
mux.finalize() → export_report.json
```

- Hardware encoders sustain several times realtime at 1080p; no segment workers are needed in v1.
- Audio is rendered once and interleaved at mux time with correct offsets.
- The report includes encoder identity, codec parameters, frame count, duration, loudness if measured, and
  renderer version — the artifact automated pipelines verify against.

## 8. Cache layout

```
<project>.arcline/cache/
├── proxies/<asset>/<profile>.mp4
├── thumbs/<asset>/<frame>.jpg
├── wave/<asset>.peak
└── shaders/<driver-hash>.bin      # platform dir, not project
```

All caches are regenerable and excluded from `collect` saves. Proxy and thumbnail caches are purged when an
asset is removed, after a grace period. Global cache size is bounded by the memory policy
([08 §3](08-performance-reliability.md)).

## 9. Licensing posture (summary; details in [11](11-risks-and-legal.md))

Ship a **custom FFmpeg build pinned to LGPLv2.1-compatible components only** (no `--enable-gpl`), dynamically
linked, with the LGPL compliance package (source, build scripts, relink instructions). H.264/HEVC encoding
uses hardware encoders only; AV1 (SVT-AV1, BSD) is the royalty-free default for software encoding. This
constrains Phase B deliverables and is a business decision, not a technical one.
