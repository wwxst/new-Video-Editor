# 08 — Performance & Reliability

Performance claims are budgets with owners and CI gates. Reliability claims are failure modes with tests.
v2 scales both to a single developer: fewer gates, one reference machine, and tests that run in minutes.

## 1. Method

- **Budgets** (§2) are measured by a small benchmark binary on one reference machine and on CI.
- **Regression gate:** > 15% regression on any budget fails CI; smaller regressions warn.
- **Reference hardware:** the developer's primary machine (Windows 11, mid-tier discrete GPU, 32 GB) plus
  one integrated-GPU laptop for the lower bound. A software adapter (lavapipe/WARP) is used only for
  determinism tests.

## 2. Budgets (v1.0 targets)

| Operation | Budget | Notes |
|---|---|---|
| Cold start → interactive window | ≤ 2.0 s | shader cache warm |
| Open 1,000-clip project | ≤ 2.0 s | shallow probe + cached metadata |
| Apply command batch (1,000 clips) | ≤ 200 µs | persistent structures, no full copy |
| UI frame | ≤ 16 ms | 60 fps shell; present on demand |
| Timeline scroll/drag paint | ≤ 8 ms | 8 tracks, 2,000 visible clip-seconds |
| Seek, cached frame | ≤ 150 ms from cache | p95 |
| Seek, uncached (proxy) | ≤ 300 ms | p95, 4K source |
| Playback 1080p30, 4 tracks, 8 effects | ≥ 30 fps sustained | discrete and integrated GPU |
| Export 1080p30 H.264 | ≥ 2× real time | hardware encoder |
| A/V sync drift | ≤ 1 frame over 10 min | audio-master clock |
| Memory, 4K project + caches | ≤ 6 GB RSS | memory manager enforces |
| Audio callback | ≤ 3 ms worst case at 128 frames | p99.9 |
| Determinism | bit-exact, same machine | asserted in CI |
| Forced-kill recovery | zero lost edits, 100 trials | autosave + journal |

## 3. Memory policy

One `MemoryBudget` owns the large allocations:

| Consumer | Default | Eviction |
|---|---|---|
| GPU texture pool | 40% of VRAM (probed) | LRU, live frame pinned |
| Frame cache | 2 GB | LRU by content key |
| Decode frame buffers | 512 MB total | ring slots per decoder |
| Thumbnails/waveforms | 256 MB | LRU |
| Document/journal | effectively unbounded (small) | — |

Rules: allocations above 8 MB go through the budget; a consumer over its share evicts before the allocator
sees pressure; GPU allocation failure evicts once, then drops to half resolution. OOM is a defect with a
crash bundle, not a user-facing event.

## 4. Persistence and crash recovery

1. **Journal**: every applied batch, append-only with a CRC per record, fsync every second or 100 batches.
2. **Autosave**: every 30 commands and every 60 s; write temp → fsync → rename.
3. **Explicit save**: snapshot + journal truncation into the container, checksums, atomic rename.
4. **Recovery**: if the journal is newer than the snapshot, replay it; if replay fails at record N, offer
   recovery to N−1 with a diff of what is lost. Never silently discard.

Forced-kill testing (`TerminateProcess`/`SIGKILL` at random points) is part of CI: 100 trials per run, zero
unrecoverable sessions required.

In-process decode raises the stakes on malicious media. Mitigations: decoder panics are caught per clip,
the journal limits loss to the last second of edits, and the media path never executes shell interpolation.
A decode worker process is the documented next step if crash rates justify it ([ADR-0003](adr/0003-ffmpeg-for-media-io.md)).

## 5. Telemetry and crash reporting

- Crash reports are opt-in at first run, off by default; a CLI flag disables them entirely.
- Bundles contain a minidump, the last 20 journal **command types and hashes**, adapter/driver info, and
  thread states — no media content, file names, or project names.
- Performance telemetry is not collected in v1. Local diagnostics (`arcline-cli diag`) print device
  capabilities, cache usage, governor state, and decoder health.

## 6. Testing strategy

```
┌───────────────────────────────┐
│ Property tests (time, commands)│  every PR
├───────────────────────────────┤
│ Golden frames (tolerance)      │  every PR, software adapter
├───────────────────────────────┤
│ Forced-kill + parser fuzz      │  nightly
├───────────────────────────────┤
│ Unit tests                     │  every commit
└───────────────────────────────┘
```

- **Unit**: time math (NTSC/PAL boundary cases), keyframe evaluation, command validation, migrations.
- **Property tests** (`proptest`): random command sequences over random documents; all invariants of
  [02 §9](02-domain-model-and-project-format.md), especially apply→inverse→identical hash.
- **Golden frames**: a 40-clip fixture corpus (cuts, transitions, keyframes, color, text, chroma key,
  transparency) rendered on CI with a software adapter; comparison by PSNR ≥ 45 dB, not exact equality.
  Goldens are regenerated deliberately, with the change reviewed.
- **Pipeline integration**: generated fixture media (color bars, motion patterns, tone sweeps) → import →
  edit → export → decode → compare. No third-party media in CI.
- **Fuzzing**: project JSON and text-timeline parsers; a mutated-media corpus fed through the decoder path,
  asserting the app survives.
- **Compile matrix**: Windows is the release target; macOS and Linux `cargo check` run on every PR to
  prevent bit-rot.

## 7. Profiling

- `puffin` scopes behind a feature flag for UI, engine, render, and audio.
- GPU timestamps per compositor stage when the adapter supports them.
- `arcline-cli diag` for field diagnostics.
- Every log line carries `session`, `revision`, and subsystem; logs rotate at 10 MB × 5.

## 8. Known hot spots and design answers

| Hot spot | Answer |
|---|---|
| Timeline paint with thousands of clips | virtualized painting + LOD; measure `egui` before adding a custom pass |
| Command apply on large documents | persistent structures, O(log n) edits, cached canonical hash |
| Decoder seek latency | proxies, GOP cache, hardware decode |
| GPU upload stalls | YUV upload + GPU color conversion + staging ring |
| Export wall time | hardware encoders + readback ring; parallel segments are post-1.0 |
| Audio callback jitter | lock-free ring, pre-mixed blocks, no work on the callback |
| Shader compile hitches | persistent pipeline cache keyed by driver |
| Large project open | shallow probe, cached metadata, lazy asset resolution |
