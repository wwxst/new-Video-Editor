# ADR-0003 — FFmpeg for media I/O, in-process in v1

**Status:** Accepted, revised 2026-11-24 after architecture review.

## Context

The product must ingest a wide range of containers and codecs (camera, phone, screen recording, broadcast
MXF, stills) and encode professional deliverables (ProRes, DNxHR, H.264/HEVC, AV1, PCM). Writing demuxers
and codecs is a multi-year quality trap. The original proposal isolated FFmpeg in a dedicated subprocess
with shared-memory frame transport — a robustness feature whose complexity is priced for a team.

## Decision

Use **FFmpeg `libav*`** behind an owned `arc-media` abstraction, **in-process in v1**, with:

- hardware decode/encode first, software fallback;
- panic isolation at the `Decoder` call boundary (a decoder panic becomes a per-clip `MediaError`);
- the `ffmpeg` CLI for batch jobs (proxies, thumbnails, waveform decode, unusual audio conversion);
- an LGPL-only build, dynamically linked, with the compliance package.

The `Decoder`/`Encoder` traits are shaped so decode can move to a worker process later without changing
callers.

## Consequences

- Broad format support with no IPC plumbing in v1; the media path stays simple.
- A hostile or broken file can crash the decoder. Mitigations: catch-unwind at the boundary, journal +
  autosave limiting loss, fuzzed media in CI. The upgrade path to process isolation is documented and
  additive.
- Batch transcodes via the CLI remove a large amount of encoder-wiring code from `arc-media`.
- Legal obligations remain: LGPL dynamic linking, published build scripts and source offer, no GPL
  components, and a separate patent posture for H.264/HEVC ([11 §2](../11-risks-and-legal.md)).
- Version pinning is deliberate; upgrades happen on a cadence against the fixture corpus.

## Alternatives considered

- **Subprocess decode with shared-memory frames** (original decision): strongest isolation, but days of
  IPC, a second binary, and a custom failure-recovery surface before any editing exists. Deferred, not
  rejected.
- **GStreamer**: pipeline model fights frame-accurate random access; large plugin surface.
- **Platform decoders only** (Media Foundation/AVFoundation): three divergent format matrices; poor
  MXF/ProRes coverage on Windows.
- **Commercial SDKs**: cost and lock-in; not justified for v1.
