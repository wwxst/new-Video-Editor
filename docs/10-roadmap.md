# 10 — Development Roadmap

Sized for **one developer with AI assistance**. Estimates assume focused full-time work; AI compresses
typing and boilerplate, not integration debugging, codec build matrices, or UI polish. Phases are vertical
slices: each one ends with something runnable.

This roadmap delivers **v1.0 on Windows**. **v1.1** is the macOS port (no new features); **Linux** is
post-1.0 with no date. The release plan and CI tiers are defined in [ADR-0011](adr/0011-windows-first.md).

## 1. Phase overview

**Calendar = sum of effort.** One developer works on one phase at a time; AI assistance shortens phases but
does not run them in parallel. (Earlier versions of this plan overlapped phases on the chart, which implied
parallel work and understated the calendar by about 12 weeks; corrected 2026-11-24.)

```
Weeks  1    5    10   15   20   25   30   35   40   45   50
       |----|----|----|----|----|----|----|----|----|----|
A Spine        ███
B Pipeline        █████
C Compositor           ███████
D Audio                       ████
E Lifecycle                       ████
F Agent surface + live sessions       ███████
J Agent workspace                            ████
G Finish                                         ███████
I Interchange                                           █████
H Harden                                                     ████
```

Order rationale: A → B → C build the editor; D and E make it trustworthy; F and J deliver both product modes
([13](13-product-modes-and-live-sessions.md)) as soon as the command surface exists, so agent workflows are
dogfooded through G; I maps the finished feature set; H is last.

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
- Frame-rate and timecode boundary tests of [08 §6](08-performance-reliability.md) pass (DF/NDF, all
  supported rates); no floats in timing types.
- `arcline-cli new demo.arcline && arcline-cli apply` produces a valid container.

## 3. Phase B — Headless pipeline (weeks 4–8)

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

## 4. Phase C — Compositor and shell (weeks 9–15)

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

## 5. Phase D — Audio (weeks 16–19)

**Goal:** playback and mixing that stay in sync.

Deliverables

- `arc-audio`: `cpal` output, mixer (gain, pan, fades, mute/solo), sample-accurate clip reads.
- Audio-master clock; A/V sync; device-change handling; meters.
- Offline mix path for export with hash parity to real-time at the parity tap ([05 §7](05-audio-engine.md));
  transport-speed WSOLA in the monitoring stage only.

**Exit criteria**

- 10-minute playback drifts ≤ 1 frame and produces no clicks at edits or seeks.
- Master-bus output at 1× transport and the offline export mix are hash-identical on the fixture project,
  including clips with non-1× clip speed.

## 6. Phase E — Project lifecycle (weeks 20–23)

**Goal:** trust the app with real work.

Deliverables

- Persistence per [08 §4](08-performance-reliability.md): sidecar directory, journal writer thread,
  commit/ack rule, group `fsync` and durable mode, autosave snapshots, atomic container replace, recovery
  dialog, read-only degraded mode on journal failure.
- Crash breadcrumbs, automatic quarantine after an unclean exit, decode hang watchdog with the 2-thread
  limit and decode-safe mode.
- Offline media and relink; proxy generation via `ffmpeg` CLI as a background job.
- GUI export through a child `arcline-cli render` process; export presets, save modes (link/collect),
  project settings UI.

**Exit criteria**

- 100 forced kills during editing, autosave, and save: zero lost acknowledged batches, zero unopenable
  projects.
- VM hard-reset test: default mode loses at most ~1 s of edits; durable mode loses none.
- Journal fault-injection suite passes: no acknowledgement before a complete write; failed and speculative
  batches rolled back; read-only mode entered.
- A forced native crash during preview decode reopens into the recovered state with the asset quarantined.
- A project with moved media opens with placeholders and relinks by path or hash.

## 7. Phase F — Agent surface and live sessions (weeks 24–30)

**Goal:** agents edit with parity, headless or attached to a live GUI session.

Deliverables

- `arc-cli` library: session server, the 15 JSON-RPC methods and 4 notifications of
  [06 §4.1](06-agent-interface.md), revisions, dry-run, idempotency, typed errors, acknowledgement after a
  complete journal write; `arc-app` depends on it.
- `arcline-cli mcp` with the 23 tools of [06 §6.1](06-agent-interface.md), resources and subscriptions,
  rendered-frame inspection; text timeline (`inspect.timeline`), read-only; render jobs.
- Local IPC in the GUI: per-user named pipe, discovery file, token, consent prompt
  ([13 §3](13-product-modes-and-live-sessions.md)).
- `--attach` / `--launch` bridge; up to 4 clients; `client_id` attribution in history.
- Control and takeover (`shared` / `human` / `agent:<id>`), state-sync notifications, activity strip with
  *Take over*.
- Reference agent script and integration guide for headless and attached use.

**Exit criteria**

- The worked example in [06 §10](06-agent-interface.md) runs end-to-end headless **and** attached to a live
  GUI, with every edit visible in the window ≤ 100 ms after commit.
- Takeover mid-edit: the human takes over, edits, hands back; the agent receives `not_in_control`, then
  `control.changed`, and continues from the new revision. No edit is lost or applied twice.
- Another Windows user's process cannot open the pipe; no edit is possible before consent.
- Path escapes (`..`, symlinks, absolute paths outside root) are rejected by tests.
- Protocol conformance suite (~180 cases) passes identically headless, attached, and in-process.

## 8. Phase J — Agent workspace (weeks 31–34)

**Goal:** both product modes are pleasant to use, not just possible.

Deliverables

- **Agent panel** (Standalone): Messages API loop on a worker thread, streaming, tool-call display with
  undo links, confirmations, OS credential storage, first-use disclosure ([13 §7](13-product-modes-and-live-sessions.md)).
- **Companion layout** (Embedded): compact workspace preset; `--launch` opens it.
- **MCP Apps view** `ui://arcline/workspace`: preview monitor, timeline overview, control buttons, job
  progress ([13 §8](13-product-modes-and-live-sessions.md)); tested in Claude Desktop and VS Code.

**Exit criteria**

- Standalone: a user asks the Agent panel for a captioned rough cut of imported footage, reviews it, undoes
  one agent step, and exports — without leaving Arcline.
- Embedded: the success scenario in [00 §9](00-product-scope.md) passes in Claude Desktop with the MCP Apps
  view, and in one host without MCP Apps using the companion window and image results.
- Agent-panel replay tests pass offline; no key material appears in logs or crash bundles.

## 9. Phase G — Finish (weeks 35–41)

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

## 10. Phase I — Interchange export (weeks 42–46)

**Goal:** an Arcline edit opens as an editable timeline in Premiere Pro, DaVinci Resolve, and Final Cut Pro,
and in Jianying Pro if an import path is confirmed ([12](12-project-interchange.md)).

Deliverables

- **Week 1 — compatibility lab:** hand-built FCPXML, FCP7 XML, and OTIO samples imported into Premiere,
  Resolve, Final Cut Pro (borrowed or rented Mac), Jianying Pro, and CapCut; results recorded; default
  formats chosen for Jianying Pro and Resolve; decision on offering Premiere OTIO.
- `arc-core::interchange`: IR, target profiles with evidence levels, FCPXML / FCP7 XML / OTIO writers,
  loss analysis.
- `arc-cli` export job: packaging (reference / copy / consolidate, optional transcode), baking of effect and
  title clips, SRT sidecars, stems, reference render, CapCut handoff package.
- `interchange_report.json` / `.md`; `job.submit {kind: "interchange"}`; MCP `interchange_export`;
  `arcline-cli export`; *Export Project for…* dialog with report view.
- CI validation: FCPXML DTD, OTIO reference loader, frame-count checks, 12 golden reference projects.

**Exit criteria**

- The 12 reference projects import into Premiere (FCP7 XML), Resolve (FCPXML and OTIO), and Final Cut Pro
  (FCPXML) with cuts, tracks, source ranges, transitions, audio levels/fades, and markers intact.
- Every approximation, bake, and drop appears in the report; no feature is silently lost.
- Jianying Pro and CapCut each have a recorded lab verdict, and their profiles reflect it (editable import,
  or handoff package).

## 11. Phase H — Harden (weeks 47–50)

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

## 12. Explicit v1 cut list

No plugins (OFX/WASM/VST3); no plugins for third-party video editors; no native in-window embedding inside
agent clients; no remote or multi-user agent access; no HDR/ACES/OCIO; no collaboration; no render
farm/multi-GPU; no project import from other editors, no EDL/AAF, no native Jianying/CapCut drafts; no
multicam; no screen-reader certification; no image sequences; no
audio EQ/compressor/loudness; no three-OS installer matrix.

These are not failures. They are the difference between shipping and not.

## 13. Effort assumptions

| Phase | Weeks | Notes |
|---|---|---|
| A | 3 | pure logic, very AI-friendly |
| B | 5 | FFmpeg build matrix is the schedule risk |
| C | 7 | UI work; the largest phase |
| D | 4 | cpal + sync; mostly known |
| E | 4 | persistence; test-heavy |
| F | 7 | protocol + MCP (5) + local IPC, attach, control, sync (2) |
| J | 4 | Agent panel (2), MCP Apps view (1.5), companion layout (0.5) |
| G | 7 | broad surface; cut aggressively if late |
| I | 5 | 1 week lab + 4 weeks writers, packaging, reports |
| H | 4 | packaging + hardening |
| **Total** | **~50** | sequential; v1.0 (Windows) ≈ week 50; v1.1 macOS port +3–4 |

If a phase slips beyond its estimate by more than 30%, cut from Phase G's list before extending the
schedule — the v1 promise is a working editor with an agent surface, not a feature checklist. Within
Phase I, cut in this order: OTIO for Premiere, consolidate/transcode modes, stems; never the report.

If the release date matters more than scope, these cuts bring v1.0 toward ~week 42, in this order:
1. MCP Apps view → companion window + image results only (−1.5 weeks).
2. Agent panel → external agent clients only (−2 weeks).
3. Phase I → FCPXML + FCP7 XML only, no OTIO; Jianying/CapCut lab deferred (−1.5 weeks).
4. Phase G → keyframe UI and chroma key deferred (−3 weeks).

Never cut: live attach, takeover, the journal guarantees, or the compatibility report.

## 14. KPIs

| KPI | Target at 1.0 |
|---|---|
| Reference agent produces a 60 s captioned cut | ≤ 10 min wall clock |
| Human time-to-first-export (new user) | ≤ 10 min |
| Crash-free sessions | ≥ 99% |
| p95 seek latency (proxy) | ≤ 300 ms |
| Export throughput (1080p H.264) | ≥ 2× real time |
| Golden-test pass rate on CI | 100% (tolerance-bounded) |
| Human takeover of a live agent session | effective on the next batch; zero lost or duplicated edits |
| Forced-kill recovery | zero lost acknowledged batches over 100 trials |
