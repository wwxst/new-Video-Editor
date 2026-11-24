# 08 — Performance & Reliability

Performance claims are budgets with owners and CI gates. Reliability claims are failure modes with tests.

## 1. Method

- **Budgets** are defined per operation (§2), measured by `arc-bench` on fixed reference hardware and by
  telemetry percentiles in the field.
- **Regression gates**: > 10% regression on any budget fails CI; 5–10% posts a warning and requires a
  written justification in the PR.
- **Reference hardware** (CI runners and manual bench):
  - *Desktop*: Windows 11, RTX 3060-class, 32 GB, NVMe.
  - *Laptop/integrated*: macOS 14, Apple M2 Air, 16 GB.
  - *Linux*: Ubuntu 24.04, AMD RX 6600, 32 GB.
  - *Software*: lavapipe on a 16-core CI VM for determinism tests.

## 2. Budgets (v1.0 targets)

| Operation | Budget | Notes |
|---|---|---|
| Cold start → interactive window | ≤ 1.5 s | shader cache warm |
| Open 1,000-clip project | ≤ 2.0 s | shallow probe + cached metadata |
| Apply command batch (1,000 clips) | ≤ 200 µs | structural sharing, no full copy |
| UI frame (idle/interactive) | ≤ 8 ms CPU, present on demand | 120 Hz capable |
| Timeline scroll/drag paint | ≤ 8 ms | 8 tracks, 2,000 visible clip-seconds |
| Seek, cached frame | ≤ 100 ms to first present | p95 |
| Seek, uncached (proxy) | ≤ 250 ms | p95, 4K source |
| Playback 1080p30, 8 tracks, 20 effects | ≥ 30 fps sustained | Tier A/B GPU |
| Playback 4K30 with proxy | ≥ 30 fps | Tier A |
| Export 1080p30 H.264 | ≥ 4× real time | NVENC/VideoToolbox; 8 workers |
| Export 4K30 ProRes | ≥ 1× real time | CPU-bound |
| A/V sync drift over 2 h | ≤ 1 frame | audio-master clock |
| Memory, 4K project + 2 GB cache | ≤ 8 GB RSS | memory manager enforces |
| Audio callback | ≤ 2 ms worst case at 128 frames | measured p99.9 |
| Export determinism | bit-exact, same device+driver | reference mode otherwise |

## 3. Memory policy

A central `MemoryBudget` owns all large caches and GPU pools:

| Consumer | Default share | Eviction |
|---|---|---|
| GPU texture pool | 40% of VRAM (probed) | LRU by node key, live frame pinned |
| Frame cache L0 (GPU) | part of above | LRU |
| Frame cache L1 (disk) | 20 GB cap | size+age |
| Decode worker frames | 512 MB total | ring slots per worker |
| Thumbnails/waveforms | 256 MB | LRU |
| Document/journal | unbounded by design (small) | — |

Rules: allocations above 8 MB go through the budget; exceeding a consumer's share triggers its eviction
*before* the allocator sees pressure; GPU allocation failure is handled by evicting and retrying once, then
degrading quality. OOM is a bug with a crash bundle, not a user-facing event.

## 4. Persistence and crash recovery

Layers:

1. **Journal** (every applied batch, append-only, CRC per record, fsync every 1 s or 100 batches). Cost is
   a few hundred bytes per edit.
2. **Autosave** (default: every 30 commands and every 60 s): full snapshot written to
   `.arcline.autosave/project.json.tmp` → fsync → rename. Atomic.
3. **Explicit save**: snapshot + journal truncation into the container, checksums, then atomic rename of the
   container file.
4. **Recovery**: on open, if a journal is newer than the snapshot, replay batches onto the snapshot; if
   replay fails at record N, offer "recover to revision N−1" with the diff of what is lost. Never silently
   discard.

Failure injection is part of CI: kill the process at random points during save (`SIGKILL`/`TerminateProcess`
in a fuzz harness), then assert recovery restores a consistent document. Target: zero lost-edit incidents
across a 30-day soak.

## 5. Telemetry and crash reporting

- **Opt-in** at first run (clearly explained, default *on* for crash reports, *off* for usage analytics;
  documented in [11](11-risks-and-legal.md)).
- Crash: minidump + last 20 journal records (command *types* and hashes, no media content) + adapter/driver
  info + thread states. Uploaded to a self-hosted endpoint; a CLI flag disables entirely.
- Performance: p50/p95 of the §2 budgets, quality-governor level distribution, decoder crash rate by codec.
  No file paths, no frame content, no project names.

## 6. Testing strategy

```
                    ┌──────────────────────────────┐
                    │  Soak / forced-kill / fuzz   │  nightly, 8 h
                    ├──────────────────────────────┤
                    │  Golden frames + determinism │  per PR (lavapipe) + nightly (hw)
                    ├──────────────────────────────┤
                    │  Property tests (commands)   │  per PR
                    ├──────────────────────────────┤
                    │  Integration (pipeline)      │  per PR
                    ├──────────────────────────────┤
                    │  Unit tests                  │  per commit
                    └──────────────────────────────┘
```

- **Unit**: time math, keyframe evaluation, command validation, migration functions. Time math gets
  exhaustive tests over NTSC/23.976/25/29.97/30/50/59.94/60 and boundary values.
- **Property tests** (`proptest`): random command sequences against random documents, asserting all
  invariants of [02 §9](02-domain-model-and-project-format.md), especially apply→inverse→hash equality.
- **Golden frames**: 120-clip corpus; PSNR/SSIM thresholds against reference renders; per-effect tier
  coverage; transparency, HDR, and text cases; run on lavapipe in PRs for determinism, on hardware nightly
  for correctness.
- **Pipeline integration**: fixture media (generated: color bars, motion patterns, tone sweeps, 10-bit HDR)
  → import → edit → export → decode → compare. No external media in CI (licensing + determinism).
- **Fuzzing**: project/JSON/text-timeline parsers; FFmpeg file corpus (truncated, mutated headers) through
  the decode worker asserting parent survival; MCP tool input fuzzing.
- **Soak**: 8 h playback + random edits, 30-day internal dogfood on real footage, forced-kill loop 1,000×.
- **Accessibility**: `accesskit` tree snapshots + a keyboard-only end-to-end script that edits and exports.

## 7. Profiling and observability (developer)

- `puffin`/Tracy integration behind a feature flag: CPU scopes for UI, engine, render, audio; GPU timestamp
  queries per graph node.
- `arcline diag` prints device capabilities, cache sizes, budget usage, governor state, decoder health.
- Every log line carries `session`, `revision`, and subsystem; logs rotate at 10 MB × 5.

## 8. Build and distribution performance

- Release builds: `lto = "fat"`, `codegen-units = 1` for the app; workspace default `lto = "thin"` for fast
  iteration.
- CI matrix: Windows (MSVC), macOS (arm64 + x86_64 universal), Linux (glibc + Flatpak). Target full matrix
  ≤ 25 min with `sccache`.
- Binary size target: ≤ 80 MB installer (FFmpeg shared libs excluded from the count, installed alongside).

## 9. Known hot spots and design answers

| Hot spot | Answer |
|---|---|
| Timeline paint with thousands of clips | instanced quads + virtualization + LOD |
| Command apply on large documents | persistent structures, O(log n) edit, cached canonical hash |
| Decoder seek latency | proxies, GOP cache, hw decode, frame-exact seek cache |
| GPU upload stalls | staging ring + async map + YUV upload + color convert on GPU |
| Export wall time | parallel segment processes + hw encoders + audio rendered once |
| Audio callback jitter | lock-free ring, pre-mixed blocks, no work on callback |
| Shader compile hitches | persistent pipeline cache keyed by driver; warm-up on project open |
| Large project open | shallow probe, paged document for > 5 MB, lazy asset resolution |
