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
| Forced-kill recovery | zero lost *acknowledged* edits, 100 trials | journal ack rule, §4 |

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

This section is the single normative definition of persistence. Other documents link here.

### 4.1 On-disk layout

```
Demo.arcline                     # container (ZIP): last explicit save, replaced atomically
Demo.arcline.autosave/           # sidecar directory, same parent folder
├── snapshot-<rev>.json          # latest autosave snapshot (canonical project.json at revision <rev>)
├── journal-<rev>.bin            # batches committed after snapshot <rev>, append-only
├── session.lock                 # PID + start time; stale-lock detection
└── crash-breadcrumb             # assets being decoded in-process (see 04 §1.2)
```

### 4.2 Journal records

Each record: `magic | length (u32) | revision (u64) | payload (resolved forward commands, CBOR) | CRC32C`.
Records are appended with a single `write` call each. A torn or corrupt tail is detected by length/CRC and
truncated during recovery; nothing after the first invalid record is trusted.

### 4.3 Commit point and acknowledgement

1. The engine validates and applies a batch, then publishes the new snapshot immediately (UI stays
   responsive; the engine thread never waits on disk).
2. The batch's record is handed to the **journal writer thread**, which `write`s it to the file.
3. A batch is **committed** when that `write` returns: the bytes are in the OS page cache and survive any
   *process* crash or kill.
4. **Protocol clients** (`project.apply` responses) are acknowledged only after step 3. An acknowledged
   batch is never lost to a process crash.
5. **The GUI** does not wait. Batches queued to the writer but not yet written when the process dies (in
   practice the last few milliseconds of interaction) can be lost. The UI never shows a "saved" state for
   them.
6. If a journal `write` fails (disk full, permission, device removed), the session enters **read-only
   degraded mode**: further commands are rejected with `journal_unavailable`, the user is warned, and the
   last committed state can still be exported or saved elsewhere. Edits are never accumulated without a
   journal.

### 4.4 `fsync` policy

`write` protects against process crashes only. Protection against OS crash or power loss requires `fsync`
(`FlushFileBuffers` on Windows):

| Mode | Journal flush | Use |
|---|---|---|
| **Default** | group `fsync` at most 1 s after the oldest unflushed record, and on save, on window focus loss, and on clean exit | GUI and most agent sessions |
| **Durable** (`--durable`, or `durable: true` at `project.open`) | `fsync` before acknowledging each batch | pipelines that cannot tolerate any loss; adds a few ms per batch |

### 4.5 Snapshots and saves

- **Autosave** (every 30 committed batches or 60 s, whichever comes first, if anything changed): write
  `snapshot-<rev>.json.tmp` → `fsync` file → rename to `snapshot-<rev>.json` → start `journal-<rev>.bin` →
  delete the previous snapshot and journal. On Windows the rename uses `MoveFileExW` with
  `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH`; on macOS/Linux the parent directory is `fsync`ed
  after rename.
- **Explicit save**: write `Demo.arcline.tmp` (ZIP with checksums) → `fsync` → atomic replace of
  `Demo.arcline` (same rename rules) → write an autosave snapshot at the saved revision and rotate the
  journal. An interrupted save leaves the previous container intact.
- A clean exit after an explicit save removes the sidecar directory.

### 4.6 Recovery algorithm

On open, if the sidecar directory exists and is newer than the container:

1. Load the newest valid base: `snapshot-<rev>.json` if its checksum verifies, else the container.
2. Replay `journal-<rev>.bin` records in order, verifying CRC and that each `revision` is exactly one more
   than the last. Each record is re-validated through the normal command path.
3. Stop at the first invalid record or failed validation. Report the recovered revision, the number of
   records discarded, and offer: open recovered state (default), open last explicit save, or keep both.
4. Never silently discard: the unrecovered tail is preserved as `journal-<rev>.bin.rejected` for support.

### 4.7 Data-loss guarantees (summary)

| Event | Default mode | Durable mode |
|---|---|---|
| App crash, panic, `TerminateProcess`/`SIGKILL` | no acknowledged batch lost; GUI may lose writer-queued batches (milliseconds) | same |
| OS crash, power loss | at most ~1 s of committed batches | no acknowledged batch lost |
| Disk full / journal write failure | no silent loss; session becomes read-only | same |
| Corrupt container on disk | recover from sidecar snapshot + journal if present | same |
| Storage device that ignores flush requests | not guaranteed (hardware limitation) | not guaranteed |

Forced-kill testing (`TerminateProcess`/`SIGKILL` at random points during edits, autosaves, and saves) runs
in CI: 100 trials per run; required result is zero lost acknowledged batches and zero unopenable projects.
Power-loss behavior is verified manually with a VM hard-reset test before each release.

### 4.8 Media crash isolation

In-process FFmpeg decode can terminate the whole process; `catch_unwind` does not prevent that. The
isolation strategy (subprocess import check, crash breadcrumbs, quarantine, out-of-process export) is
defined in [04 §1](04-media-pipeline.md). Persistence guarantees above are what bound the cost of such a
crash: the user loses no acknowledged edits and reopens into the recovered state.

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
- **Fuzzing**: project JSON and text-timeline parsers in-process. A mutated-media corpus is fed through the
  **subprocess import check** ([04 §1.2](04-media-pipeline.md)), asserting the parent survives and every
  crashing/hanging file ends up quarantined. The same corpus runs through the in-process decoder in a
  separate harness process, where crashes are expected to be possible and are triaged as FFmpeg or wrapper
  defects.
- **CI matrix**: per the release plan in [ADR-0011](adr/0011-windows-first.md) — Windows: build, full test
  suite, packaging. macOS: `cargo check` on every PR until v1.1 work starts, then full tests. Linux:
  `cargo check` plus `arc-core` tests on every PR (core is platform-independent).

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
