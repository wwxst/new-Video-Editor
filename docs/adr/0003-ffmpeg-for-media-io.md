# ADR-0003 — FFmpeg for media I/O, out of process

**Status:** Accepted · 2026-11-24

## Context

The product must ingest a very wide range of containers and codecs (camera, phone, screen recording,
broadcast MXF, stills, image sequences) and must encode professional deliverables (ProRes, DNxHR, H.264/HEVC,
AV1, PCM audio). Writing demuxers and codecs from scratch is a multi-year effort and a quality trap.

## Decision

Use **FFmpeg `libav*`** as the media I/O engine, behind an owned abstraction (`arc-media`), running inside a
**dedicated decode subprocess** (`arcline-decode`), linked against an **LGPL-only build**. Encode uses the
same libraries for muxing plus hardware encoders where available.

## Consequences

- Broadest format support available; hardware acceleration for decode/encode on all three platforms.
- Process isolation converts FFmpeg's security and stability risks into a recoverable failure mode: a
  decoder crash degrades one clip, never the session ([04 §1](04-media-pipeline.md)).
- IPC cost is real but bounded: frames move through shared memory, not the control pipe.
- Legal obligations are real: LGPL dynamic linking, source/build-script publication, no GPL components, and
  a separate patent posture for H.264/HEVC ([11 §2](11-risks-and-legal.md)). This is scheduled work, not a
  footnote.
- Version pinning is deliberate: we upgrade FFmpeg on a fixed cadence with a regression suite over the
  fixture corpus, not continuously.
- The abstraction lets us swap individual decoders (e.g., platform VideoToolbox directly for specific
  codecs, or a future AV2 implementation) without touching callers.

## Alternatives considered

- **GStreamer**: excellent for streaming pipelines, but its push/pipeline model fights frame-accurate
  random access, and the dependency/plugin surface is large for a desktop app.
- **Platform decoders only** (Media Foundation / AVFoundation / GStreamer-on-Linux): best platform
  integration but three divergent format matrices and poor support for MXF/ProRes/DNxHR on Windows.
- **Commercial SDKs** (MainConcept, NDI, etc.): licensing cost and vendor lock-in; not justified for v1.
- **Custom demuxers**: rejected — decades of real-world edge cases live in FFmpeg's demuxers.
