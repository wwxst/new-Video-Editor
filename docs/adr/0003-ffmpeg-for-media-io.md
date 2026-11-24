# ADR-0003 — FFmpeg for media I/O, in-process preview decode in v1

**Status:** Accepted, revised 2026-11-24 after architecture review. Clarified 2026-11-24: `catch_unwind`
limits and the practical isolation strategy.

## Context

The product must ingest a wide range of containers and codecs and encode professional deliverables. Writing
demuxers and codecs is a multi-year quality trap, so FFmpeg is the media engine. The open question is
isolation. FFmpeg is C: malformed or hostile media can cause native faults (segfault, `abort`, heap
corruption) or hangs. The original proposal ran all decoding in a dedicated subprocess with shared-memory
frame transport — strong isolation, but days of IPC plumbing before any editing exists.

An earlier v2 draft claimed that panics would be "caught per clip". That is only partly true: Rust's
`catch_unwind` catches **Rust panics** in our wrapper code. It cannot catch native faults or hangs inside
libav, and unwinding must never cross an `extern "C"` boundary.

## Decision

Use **FFmpeg `libav*`** behind an owned `arc-media` abstraction with an LGPL-only, dynamically linked build.

- **Preview decode runs in-process** behind a `Decoder` trait. FFmpeg error codes map to per-clip
  `MediaError`; Rust panics in the wrapper are caught with `catch_unwind` (release profile keeps
  `panic = "unwind"`). Native faults are **not** recoverable in-process and are treated as fatal.
- **Everything else that touches untrusted media runs in child processes:**
  - import check and probe (`ffprobe` + a short test decode, with a timeout) before an asset is admitted
    for in-process decode;
  - proxies, thumbnails, waveform PCM (`ffmpeg` CLI);
  - exports (`arcline-cli render`, spawned by the GUI and by `job.submit`).
- **Containment of the remaining risk:** crash breadcrumbs quarantine any asset being decoded when the
  process died; a hang watchdog moves stuck assets to the proxy path, with at most 2 abandoned decode threads per process
  before the session switches to proxy-only "decode-safe mode"; the journal guarantees no
  acknowledged edit is lost ([08 §4](../08-performance-reliability.md)).

Full strategy: [04 §1](../04-media-pipeline.md).

## Consequences

- The interactive preview path stays simple and fast (no IPC, no shared-memory transport).
- A native fault during preview decode ends the session. The user loses no acknowledged edits, reopens into
  the recovered state, and the implicated file is quarantined so the crash does not repeat.
- Import is slower by the duration of the check (seconds per file, cached by content hash).
- Exports are isolated: an encoder or decoder crash fails the job, not the session.
- **Upgrade trigger:** if crash reports show in-process faults on admitted (non-quarantined) media above an
  agreed threshold, preview decode moves into a worker process behind the same `Decoder` trait. This change
  is additive.
- Legal obligations remain: LGPL dynamic linking, published build scripts and source offer, no GPL
  components, separate patent posture for H.264/HEVC ([11 §2](../11-risks-and-legal.md)).

## Alternatives considered

- **Subprocess decode with shared-memory frames for all decoding** (original decision): strongest
  isolation; deferred as the upgrade path, not rejected.
- **In-process everything, relying on `catch_unwind`**: rejected — it does not catch native faults, so it
  would give a false sense of safety.
- **GStreamer**: pipeline model fights frame-accurate random access; large plugin surface.
- **Platform decoders only**: three divergent format matrices; poor MXF/ProRes coverage on Windows.
