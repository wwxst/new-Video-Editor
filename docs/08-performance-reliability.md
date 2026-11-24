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
| Agent edit → visible in GUI (attached) | ≤ 100 ms after commit | IPC + publish + repaint |
| Notification delivery (`session.changed`) | ≤ 50 ms after commit | per attached client |
| Takeover effective | next validated batch | engine-thread check |
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

Each record: `magic (u32) | length (u32) | revision (u64) | payload (resolved forward commands, CBOR) |
CRC32C (over all preceding fields)`. `length` covers the whole record, so a record whose declared length runs
past end-of-file is a torn record by definition.

### 4.3 Write states, commit point, and acknowledgement

The journal writer thread appends each record at the current end offset `E` and classifies the outcome:

| State | Definition | Handling |
|---|---|---|
| **Complete write** | every byte of the record has been accepted by the OS (`write` calls returned a total equal to `length`), and in durable mode the following `fsync` succeeded | the batch is **committed**; its acknowledgement is released |
| **Partial write** | a `write` call returned fewer bytes than requested, without an error | not a failure: the writer continues writing the remaining bytes. The batch stays uncommitted and unacknowledged until the total reaches `length` |
| **Write failure** | a `write` returns an error (disk full, permission, device removed, I/O error), or `fsync` fails in durable mode | the batch is **not** committed: see failure handling below |
| **Torn record on disk** | the process died during a partial write | detected at recovery by length/CRC and truncated (§4.6); it was never acknowledged |

**The acknowledgement rule (all clients).** A batch's `ApplyResult` — the `project.apply`/`history.*` response
for protocol clients, and the result delivered to the UI thread for the GUI — is sent **only after a complete
write** of its journal record. Nothing is acknowledged earlier.

Sequence for one batch:

1. The engine validates and applies the batch and publishes the new snapshot **speculatively**, so the
   viewer and timeline update without waiting for disk. Speculative revisions are marked pending in the UI.
2. The record is queued to the journal writer thread. Records are written strictly in revision order.
3. On complete write the revision becomes committed and its `ApplyResult` is released. Committed bytes are in
   the OS page cache and survive any *process* crash or kill; power-loss durability follows §4.4.

**Write-failure handling:**

1. The writer truncates the journal back to `E` (removing any bytes of the failed record) and verifies the
   file length. If truncation also fails, the torn tail is left for recovery to discard; this is safe because
   recovery trusts nothing after the last valid record.
2. The engine **rolls back** every speculative revision after the last committed revision `C`, applying the
   stored inverses, and publishes the snapshot for `C`. Each rolled-back batch receives the error
   `journal_unavailable` instead of an acknowledgement.
3. The session enters **read-only degraded mode**: new batches are rejected with `journal_unavailable`; the
   user is warned. *Save As* to another location writes a full container and starts a new sidecar journal
   there, which returns the session to normal mode. Export remains available.
4. A **group `fsync` failure in default mode** cannot un-acknowledge batches that were already completely
   written: the session enters read-only degraded mode immediately and warns that batches since the last
   successful `fsync` may not survive power loss, recommending *Save As*.

Edits are never accumulated without a journal, and no acknowledged batch is ever rolled back.

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
| App crash, panic, `TerminateProcess`/`SIGKILL` | no acknowledged batch lost; speculative (not yet acknowledged) batches — typically the last few milliseconds — are lost | same |
| OS crash, power loss | at most ~1 s of committed batches | no acknowledged batch lost |
| Disk full / journal write failure | failed and later speculative batches rolled back and reported; session becomes read-only (§4.3) | same |
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

- **Unit**: time math, keyframe evaluation, command validation, migrations.
- **Frame-rate and timecode boundary tests** (rules in [02 §2.3](02-domain-model-and-project-format.md)),
  required before Phase A exits:
  - exhaustive round trip frame → NDF → frame and frame → DF → frame for every frame of 24 h at 29.97 and
    59.94, and frame → NDF → frame at every other supported rate;
  - fixed vectors at 29.97 DF: frame 1799 = `00:00:59;29`, 1800 = `00:01:00;02`, 17982 = `00:10:00;00`,
    107892 = `01:00:00;00`; at 59.94 DF: 3599 = `00:00:59;59`, 3600 = `00:01:00;04`; at 29.97 NDF:
    `01:00:00:00` = frame 108000 = 3603.6 s exactly (`108000 × 1001/30000`);
  - rejections: skipped DF labels (`00:01:00;00`, `00:01:00;01` at 29.97; `;00`–`;03` at 59.94),
    `;` at non-drop rates, `FF ≥ nominal`, `MM`/`SS ≥ 60`, labels past the 24 h cap, decimal rates other
    than the three aliases, rationals off a frame boundary (e.g. 1 s at 29.97);
  - `frame_at` for each rounding mode at exact boundaries and one tick either side;
  - frame-boundary → sample-index mapping at 44.1/48/96 kHz for all supported rates (e.g. one 29.97 frame
    = 1601.6 samples at 48 kHz), checked against [05 §2](05-audio-engine.md);
  - text-timeline and `project.describe` output is always NDF regardless of `timecode_display`.
- **Journal fault injection** ([§4.3](#43-write-states-commit-point-and-acknowledgement)): a test file layer
  that returns short writes, fails mid-record (ENOSPC/EIO), and fails `fsync`; asserts no `ApplyResult`
  before a complete write, rollback of speculative batches, truncation to the record start, read-only mode,
  and clean recovery of torn tails.
- **Decode-safe mode**: a test decoder that blocks forever; asserts abandonment after the deadline, pool
  replacement, decode-safe mode on the second abandonment ([04 §1.2](04-media-pipeline.md)), and exit
  without joining.
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
- **Interchange**: FCPXML DTD validation, OTIO reference-library load, FCP7 XML structural checks,
  exact frame-count checks, and byte-stable golden exports of 12 reference projects
  ([12 §7](12-project-interchange.md)). Import into target editors is a manual lab step.
- **Live sessions**: IPC endpoint rejects other users and remote clients; consent required before any edit;
  4-client limit; takeover races (agent batch in flight while the human takes over) resolve per
  [13 §5](13-product-modes-and-live-sessions.md); notifications arrive in revision order; client disconnect
  and GUI crash yield `session_ended` and a clean recovery; a protocol conformance run executes the same
  suite headless, attached, and through the Agent panel transport with identical results.
- **Agent panel**: recorded API responses replayed offline (no network in CI), including refusal,
  `max_tokens`, malformed tool input, and API errors; no key material in logs or crash bundles.
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
