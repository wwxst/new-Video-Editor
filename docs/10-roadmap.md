# 10 — Development Roadmap

Sized for **one developer with AI assistance**. Estimates assume focused full-time work; AI compresses
typing and boilerplate, not integration debugging, codec build matrices, or UI polish. Phases are vertical
slices: each one ends with something runnable.

This roadmap delivers **v1.0 on Windows**. **v1.1** is the macOS port (no new features); **Linux** is
post-1.0 with no date. The release plan and CI tiers are defined in [ADR-0011](adr/0011-windows-first.md).

## 1. Phase overview

```
Weeks  1    4    8    12   16   20   24   27
       |----|----|----|----|----|----|----|
A Spine     ███
B Pipeline     ███████
C Compositor        ████████████
D Audio                  ██████
E Lifecycle                 ████
F Agent                        █████
G Finish                          ████████
H Harden                              ██████
```

Critical path: A → B → C → G → H. F depends on C's command surface and can run in parallel with late C/D.
D is independent of F. E depends on C and D.

## 2. Phase A — Spine (weeks 1–3)

**Goal:** exact time, the document, and commands, provable without a GPU.

Deliverables

- Cargo workspace with six crates and the dependency CI check.
- `arc-core`: `Time`/`Timebase`, entities, command types, validation, undo/journal, project file read/write,
  schema v7 baseline.
- `arcline-cli` skeleton with `new`, `describe`, and `apply` (JSON commands).
- Property tests: apply → inverse → identical canonical hash.

**Exit criteria**

- 10,000 randomized command sequences pass the invariant suite.
- NTSC/PAL boundary tests pass; no floats in timing types.
- `arcline-cli new demo.arcline && arcline-cli apply` produces a valid container.

## 3. Phase B — Headless pipeline (weeks 3–7)

**Goal:** import → trim → export, end to end, without a UI.

Deliverables

- `arc-media`: FFmpeg in-process decode/encode, hardware-first; error codes and wrapper panics mapped to
  per-clip `MediaError`; subprocess import check (`ffprobe` + test decode, timeout) and quarantine
  ([04 §1.2](04-media-pipeline.md)).
- `arc-render`: offscreen device + minimal compositor (single track, transform, color convert).
- `arcline-cli render`: range, resolution, preset; `export_report.json`.
- Windows FFmpeg LGPL build pipeline pinned in the repo; macOS/Linux CI per ADR-0011.

**Exit criteria**

- `arcline-cli render in.mov out.mp4 --range …` is frame-accurate against a reference decode.
- Hardware H.264/HEVC and AV1 (SVT) encode paths work on the reference machine.
- A corrupt file produces a clean per-clip error; a file that crashes or hangs `ffprobe`/`ffmpeg` in the
  import check is quarantined and the parent process survives (mutated-media corpus).

## 4. Phase C — Compositor and shell (weeks 6–12)

**Goal:** the editor exists: timeline, preview, multi-track compositing.

Deliverables

- Direct compositor: tracks, transforms, effects chain, transitions, overlays.
- Six built-in effects, text/title renderer, frame cache.
- `arc-app`: `egui` shell, docking, media bin, inspector generated from manifests, viewer via `egui_wgpu`.
- Timeline canvas: paint, virtualization, LOD, hit testing, move/trim/split, snapping.
- Preview scheduler with audio-clock placeholder and two-level quality fallback.
- Command palette, keymap, native dialogs, IME.

**Exit criteria**

- 1080p preview ≥ 30 fps with 4 tracks and 8 effects on both reference machines.
- A user can import, arrange, trim, split, and scrub a project entirely in the GUI.
- IME works for CJK title entry on Windows.

## 5. Phase D — Audio (weeks 11–14)

**Goal:** playback and mixing that stay in sync.

Deliverables

- `arc-audio`: `cpal` output, mixer (gain, pan, fades, mute/solo), sample-accurate clip reads.
- Audio-master clock; A/V sync; device-change handling; meters.
- Offline mix path for export with hash parity to real-time.

**Exit criteria**

- 10-minute playback drifts ≤ 1 frame and produces no clicks at edits or seeks.
- Preview mix and offline export mix are hash-identical on the fixture project.

## 6. Phase E — Project lifecycle (weeks 13–16)

**Goal:** trust the app with real work.

Deliverables

- Persistence per [08 §4](08-performance-reliability.md): sidecar directory, journal writer thread,
  commit/ack rule, group `fsync` and durable mode, autosave snapshots, atomic container replace, recovery
  dialog, read-only degraded mode on journal failure.
- Crash breadcrumbs, automatic quarantine after an unclean exit, decode hang watchdog.
- Offline media and relink; proxy generation via `ffmpeg` CLI as a background job.
- GUI export through a child `arcline-cli render` process; export presets, save modes (link/collect),
  project settings UI.

**Exit criteria**

- 100 forced kills during editing, autosave, and save: zero lost acknowledged batches, zero unopenable
  projects.
- VM hard-reset test: default mode loses at most ~1 s of edits; durable mode loses none.
- A forced native crash during preview decode reopens into the recovered state with the asset quarantined.
- A project with moved media opens with placeholders and relinks by path or hash.

## 7. Phase F — Agent surface (weeks 15–19)

**Goal:** agents edit with parity.

Deliverables

- `arc-cli`: `arcline-cli rpc` with the 13 JSON-RPC methods of [06 §4.1](06-agent-interface.md),
  revisions, dry-run, idempotency, typed errors, acknowledgement after journal write.
- `arcline-cli mcp` with the 20 tools of [06 §6.1](06-agent-interface.md), resources, and rendered-frame
  inspection.
- Text timeline (`inspect.timeline`), read-only.
- Render jobs with progress and reports.
- Reference agent script and integration guide.

**Exit criteria**

- The worked example in [06 §10](06-agent-interface.md) runs end-to-end and produces a reviewable export.
- Path escapes (`..`, symlinks, absolute paths outside root) are rejected by tests.
- Protocol conformance suite passes ~150 request/response cases.

## 8. Phase G — Finish (weeks 18–24)

**Goal:** a project a non-developer can complete.

Deliverables

- Keyframe editing UI and evaluation parity.
- Transitions polish; titles styling; subtitle import and burn-in.
- LUT color workflow; chroma key; sharpen/vignette.
- Timeline UX: slip/slide, selection box, markers, track controls.
- Performance pass against [08 §2](08-performance-reliability.md).

**Exit criteria**

- A 10-minute, 4-track project with titles, music, and effects is edited and exported by a non-developer.
- Budgets met on both reference machines.

## 9. Phase H — Harden (weeks 23–27)

**Goal:** ship v1.0 on Windows.

Deliverables

- Crash reporting, diagnostics, logging; fuzzing and forced-kill hardening.
- Installer (portable zip + MSIX), code signing, auto-update channel.
- User guide, command reference generated from schemas, MCP integration guide.
- FFmpeg LGPL compliance package and codec licensing sign-off.

**Exit criteria**

- 30-day dogfood with real projects; zero lost acknowledged edits.
- Clean-VM install → open sample → export in under 5 minutes.
- Legal sign-off on codec posture.

**After v1.0** ([ADR-0011](adr/0011-windows-first.md)):

- **v1.1 — macOS port**, +3–4 weeks: FFmpeg build, VideoToolbox paths, CoreAudio, Metal validation, full CI,
  notarized universal DMG. Same features as v1.0.
- **Linux** — post-1.0, no date commitment; Flatpak, started on demonstrated demand.

## 10. Explicit v1 cut list

No plugins (OFX/WASM/VST3); no HDR/ACES/OCIO; no collaboration; no live GUI agent attach; no render
farm/multi-GPU; no EDL/FCPXML/OTIO; no multicam; no screen-reader certification; no image sequences; no
audio EQ/compressor/loudness; no three-OS installer matrix.

These are not failures. They are the difference between shipping and not.

## 11. Effort assumptions

| Phase | Weeks | Notes |
|---|---|---|
| A | 3 | pure logic, very AI-friendly |
| B | 5 | FFmpeg build matrix is the schedule risk |
| C | 7 | UI work; the largest phase |
| D | 4 | cpal + sync; mostly known |
| E | 4 | persistence; test-heavy |
| F | 5 | protocol + MCP; well-scoped |
| G | 7 | broad surface; cut aggressively if late |
| H | 4 | packaging + hardening |
| **Total** | **~27** | v1.0 (Windows); v1.1 macOS port +3–4 |

If a phase slips beyond its estimate by more than 30%, cut from Phase G's list before extending the
schedule — the v1 promise is a working editor with an agent surface, not a feature checklist.

## 12. KPIs

| KPI | Target at 1.0 |
|---|---|
| Reference agent produces a 60 s captioned cut | ≤ 10 min wall clock |
| Human time-to-first-export (new user) | ≤ 10 min |
| Crash-free sessions | ≥ 99% |
| p95 seek latency (proxy) | ≤ 300 ms |
| Export throughput (1080p H.264) | ≥ 2× real time |
| Golden-test pass rate on CI | 100% (tolerance-bounded) |
| Forced-kill recovery | zero lost acknowledged batches over 100 trials |
