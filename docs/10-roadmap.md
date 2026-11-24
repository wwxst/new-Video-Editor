# 10 — Development Roadmap

Sized for **one developer with AI assistance**. Estimates assume focused full-time work; AI compresses
typing and boilerplate, not integration debugging, codec build matrices, or UI polish. Phases are vertical
slices: each one ends with something runnable.

Windows is the release target through v1.0 ([ADR-0011](adr/0011-windows-first.md)).

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

- `arc-media`: FFmpeg in-process probe/decode/encode, hardware-first, panic isolation at the clip boundary.
- `arc-render`: offscreen device + minimal compositor (single track, transform, color convert).
- `arcline-cli render`: range, resolution, preset; `export_report.json`.
- Windows FFmpeg LGPL build pipeline pinned in the repo; macOS/Linux `cargo check`.

**Exit criteria**

- `arcline-cli render in.mov out.mp4 --range …` is frame-accurate against a reference decode.
- Hardware H.264/HEVC and AV1 (SVT) encode paths work on the reference machine.
- A corrupt file produces a clean per-clip error, not a process crash.

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

- Autosave, journal compaction, crash recovery UI, atomic saves.
- Offline media and relink; proxy generation via `ffmpeg` CLI as a background job.
- Export presets, save modes (link/collect), project settings UI.

**Exit criteria**

- 100 forced kills during editing and saving: zero lost edits.
- A project with moved media opens with placeholders and relinks by path or hash.

## 7. Phase F — Agent surface (weeks 15–19)

**Goal:** agents edit with parity.

Deliverables

- `arc-cli`: JSON-RPC session over stdio, `project.apply`, revisions, dry-run, idempotency, typed errors.
- MCP server (`arcline-cli mcp`) with the 20 tools, resources, and rendered-frame inspection.
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

**Goal:** ship Windows v1.

Deliverables

- Crash reporting, diagnostics, logging; fuzzing and forced-kill hardening.
- Installer (portable zip + MSIX), code signing, auto-update channel.
- User guide, command reference generated from schemas, MCP integration guide.
- FFmpeg LGPL compliance package and codec licensing sign-off.

**Exit criteria**

- 30-day dogfood with real projects; zero lost-edit incidents.
- Clean-VM install → open sample → export in under 5 minutes.
- Legal sign-off on codec posture.

**macOS** follows with +3–4 weeks (FFmpeg build, Metal validation, notarization, packaging). **Linux** is
post-1.0, community-assisted.

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
| **Total** | **~27** | +3–4 for macOS |

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
| Forced-kill recovery | zero lost edits over 100 trials |
