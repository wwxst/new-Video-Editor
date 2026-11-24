# 10 — Development Roadmap

Sequenced so that every phase produces something demonstrable, and so that the riskiest assumptions are
tested earliest. Durations assume the staffing in §7 and calendar weeks from project start.

## 1. Phase overview

```
Weeks  1    6    12   18   24   30   36   42   48   54   60
       |----|----|----|----|----|----|----|----|----|----|
P0 Kernel      ██████
P1 Pipeline        ███████████
P2 Editing                ███████████████
P3 Agent Surface               ████████████
P4 Finishing                         ████████████████
P5 Ecosystem                                   ████████████
P6 1.0                                               ██████████
```

Critical path: P0 → P1 → P2 → P4 (finishing) → P6. P3 (agent surface) depends on P2's command set being
stable and can start as soon as P2's model freezes (week 18), running parallel to late P2.

## 2. Phase 0 — Kernel (weeks 1–6)

**Goal:** prove the pipeline end-to-end with the minimum surface: decode → GPU color convert → encode.

Deliverables

- Cargo workspace, crate skeleton, CI on 3 OSes, dependency layering check.
- `arc-time` complete with exhaustive tests (this is deliberately first; every later bug is caught here).
- `arcline-decode` subprocess + IPC + shared memory; software + hardware decode for H.264/ProRes.
- `arc-render` device layer + `Source → Resolve → Output` graph; preview of one clip at a time.
- `arcline` CLI: `probe`, `render` (trim/scale/transcode), `diag`.
- `arc-encode` with hardware probe; NVENC/VideoToolbox/VA-API paths.

**Exit criteria**

- `arcline render in.mov -o out.mp4 --range 00:00:10:00-00:00:20:00` is frame-accurate (verified by
  frame hash against a reference decode of the same range).
- A 4K HEVC corrupt file crashes the decode worker; the CLI reports a clean error and exits 0/1 correctly.
- CI green on Windows/macOS/Linux; determinism test passes on lavapipe.

## 3. Phase 1 — Media pipeline (weeks 5–14)

**Goal:** real-time multi-source compositing with effects; the render engine earns its architecture.

Deliverables

- Render graph with tracks, transforms, effect chain, transitions, blend modes.
- Frame cache L0/L1; proxy generation; thumbnail and waveform caches.
- 12 built-in effects with manifests and golden tests; text/title renderer.
- Preview scheduler + audio master clock + quality governor (audio in preview = silent mixdown initially).
- `arc-testkit` golden harness; 60 fixtures.

**Exit criteria**

- 1080p30 preview with 8 tracks + 20 effects ≥ 30 fps on Tier A/B reference hardware.
- Seek p95 ≤ 250 ms uncached 4K proxy; ≤ 100 ms cached.
- Golden suite green on hardware nightly; determinism green on lavapipe per PR.
- Decoder fuzz corpus (500 mutated files) without parent-process crash.

## 4. Phase 2 — Editing (weeks 10–24)

**Goal:** a human can edit and export a real project. This is the longest phase; it is where the product
becomes an editor.

Deliverables

- `arc-model` + `arc-command` complete: full command set, validation, undo/redo, journal, autosave,
  crash recovery, migrations infra.
- `.arcline` container format, link and collect modes, missing-media/relink flow.
- UI shell: docking, tabs, theme, keyboard, command palette, file dialogs, IME; timeline renderer with
  virtualized paint, LOD, snapping, trim/move/slip/slide/split; inspector generated from manifests;
  keyframe editor.
- Audio engine: playback, mixing, EQ, fades, mute/solo, meters; offline render parity.
- Export pipeline: parallel segment workers, hw encoders, MP4/MOV/ProRes, WAV stems, `export_report.json`.
- Open/save/autosave/recovery end-to-end.

**Exit criteria**

- A 10-minute, 8-video-track/6-audio-track project with transitions, titles, and 20 effects is edited,
  saved, reopened, and exported on all three OSes by a tester with no developer help.
- Forced-kill test: 1,000 random kills during editing/saving → zero unrecoverable sessions.
- Keyboard-only path completes "import → cut → export".
- IME: CJK title entry works on Win/mac/Linux.

## 5. Phase 3 — Agent surface (weeks 18–30)

**Goal:** agents edit with parity. This phase gates every "agent-native" claim in the pitch.

Deliverables

- `arc-protocol` frozen at 1.0: sessions, revisions, capability tokens, typed errors, idempotency, dry-run.
- `arc-server`: daemon with stdio/pipe/WebSocket transports; job queue; job persistence across disconnects.
- MCP server: the tool catalog of [06 §6](06-agent-interface.md), resources, image-returning inspection tools.
- Text timeline projection + round-trip apply + ABNF grammar + fuzz tests.
- GUI control endpoint (topology C): agents act on a live human session.
- Security: root allow-lists, token storage, audit log, rate limits; threat-model review + pen test.
- Reference agent integration (a sample MCP client script + docs) that builds a captioned cut from a folder.

**Exit criteria**

- The worked example of [06 §9](06-agent-interface.md) runs end-to-end against the daemon and against a live
  GUI session, producing a reviewable export.
- An agent with `project:read` cannot write; with no `fs:*` capability cannot read outside root (verified by
  tests attempting escapes, symlinks, and `..` traversal).
- Protocol conformance suite: 200 request/response cases, including stale revisions, idempotent retries, and
  malformed batches.

## 6. Phase 4 — Finishing (weeks 25–40)

**Goal:** broadcast/streaming deliverables and the color/HDR story.

Deliverables

- Color management: input transforms, working spaces (709/2020/ACEScg), output transforms, HDR10/HLG,
  tone-mapping policies; 3D LUT support with space conversion.
- Loudness: R128 measurement, two-pass normalization, true-peak limiting, strict targets in export.
- Keyframe curves: bezier/ease editing UI + evaluation parity in preview/export.
- Masks (shapes + tracked-lite), chroma key, glow, film grain, perspective/corner-pin.
- Subtitles: SRT/VTT/ASS import, styling, burn-in + sidecar export; transcript→cues tool.
- Interchange: EDL, FCPXML, OTIO import/export with a documented loss matrix.
- Proxy workflow polish, optimized media management, media bin search/filter.

**Exit criteria**

- HDR10 export passes a PQ round-trip test (decode → compare within tolerance) on all hardware runners.
- A 60-second spot exported to -23 LUFS and -14 LUFS targets within ±0.1 LU measured, true peak ≤ -1 dBTP.
- Round-trip Premiere ⇄ Arcline ⇄ Resolve via FCPXML/OTIO on a 20-clip project with transitions and audio,
  with a documented list of lossy items (and no silent losses).

## 7. Phase 5 — Ecosystem (weeks 36–52)

**Goal:** extensibility and the professional long tail.

Deliverables

- OFX host (subprocess) with a curated compatibility suite; WASM effect tier + `arcline effect` SDK.
- VST3 audio plugin hosting in subprocess (Win/mac first).
- Batch/pipeline mode: `arcline render --jobs file.json`, watch folders, template projects.
- Multi-GPU export (one process per GPU), render farm submit hooks (generic command adapter).
- Scripting convenience: a thin typed client library (Rust, Python) over the protocol.
- Collaboration *groundwork*: command log export/import, deterministic merge tool for divergent logs
  (research milestone; no live multi-user in 1.0).

**Exit criteria**

- Three third-party OFX plugins from two vendors render frame-accurate output in a reference project.
- A WASM effect authored by an agent installs, renders, and is disabled cleanly when it exceeds fuel.
- A batch job of 50 renders completes unattended with per-job reports and a machine-readable summary.

## 8. Phase 6 — 1.0 (weeks 50–60)

**Goal:** ship it, signed, documented, stable.

Deliverables

- Installers: MSIX + portable (Win), notarized universal DMG (mac), Flatpak + AppImage (Linux); auto-update
  channel with signature verification and staged rollout.
- Accessibility audit and fixes (screen reader, high contrast, reduced motion); localization infrastructure.
- Docs: user guide, command reference (generated from schemas), MCP integration guide, plugin SDK guide.
- Performance hardening pass against [08 §2](08-performance-reliability.md) on all reference hardware.
- Security: dependency audit, threat-model sign-off, FFmpeg CVE watch process, LGPL compliance package.
- 30-day dogfood soak with real projects; crash-free-session SLO met.

**Exit criteria**

- All §2 budgets met on all four reference configurations.
- Crash-free sessions ≥ 99.5% over the soak; zero lost-edit incidents.
- Installer on a clean VM: install → open sample project → export, ≤ 5 minutes, no admin rights on Windows
  portable build.
- Legal sign-off on codec and FFmpeg distribution posture ([11](11-risks-and-legal.md)).

## 9. Dependencies and sequencing rationale

| Decision | Why this order |
|---|---|
| Time model first | every subsystem depends on exact time; retrofitting it is a rewrite |
| Decode isolation in P0 | retrofitting process isolation later touches every media call site |
| Command model before UI | the UI must be written against commands from day one, or it will grow private state |
| Agent surface after P2 | freezing the protocol before the command set is exercised by real editing creates a bad API |
| OFX late | it is a compatibility feature; it must not constrain the core effect model |
| HDR/color in P4 | color management is cross-cutting; doing it before the graph is exercised invites rework |

## 10. KPIs (measured continuously after P3)

| KPI | Target at 1.0 |
|---|---|
| Time for reference agent to produce a 60 s captioned cut | ≤ 10 min wall clock |
| Human time-to-first-export for a new user | ≤ 10 min (guided sample project) |
| Crash-free sessions | ≥ 99.5% |
| p95 seek latency (proxy) | ≤ 250 ms |
| Export throughput vs real time (1080p H.264) | ≥ 4× |
| Protocol error rate in agent sessions | ≤ 2% of calls (proxy for API clarity) |
| Golden-test pass rate across hardware | 100% (tolerance-bounded) |

## 11. Staffing

| Role | P0–P1 | P2 | P3–P4 | P5–P6 |
|---|---|---|---|---|
| Architect / tech lead | 1 | 1 | 1 | 1 |
| Render / GPU engineer | 1 | 2 | 2 | 2 |
| Media / codec engineer | 1 | 1 | 1 | 1 |
| Systems / backend (protocol, server) | 0.5 | 1 | 1.5 | 1 |
| UI engineer | 0.5 | 2 | 2 | 2 |
| Audio engineer | 0 | 1 | 1 | 1 |
| QA / release engineer | 0 | 0.5 | 1 | 1.5 |
| **Total (FTE)** | **4** | **8.5** | **9.5** | **9.5** |

Schedule risk is carried in §11 of [11-risks-and-legal.md]; the two largest are codec licensing (affects
P1 deliverables) and the widget layer ramp-up (affects P2). Both have explicit fallbacks:
hardware-only encoders + open codecs (AV1) for the former; a license-friendly toolkit adoption as a
contingency for the latter (ADR-0007 documents the reversal cost).
