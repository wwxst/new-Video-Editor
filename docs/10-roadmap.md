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
Weeks  1    5    10   15   20   25   30   35   40   45   50   55
       |----|----|----|----|----|----|----|----|----|----|----|
A Spine        ███
P Embed PoC       █
B Pipeline         █████
C Compositor            ███████
D Audio                        ████
E Lifecycle                        ████
F Agent surface + live sessions        ███████
J Agent workspace + embedding                 ██████
G Finish                                            ███████
I Interchange                                              █████
H Harden                                                        ████
```

Order rationale: A builds the core; P tests native embedding before any UI is built on it; B → C build the
editor; D and E make it trustworthy; F and J deliver both product modes and all three integration levels
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

## 3. Phase P — Embedding PoC (week 4)

**Goal:** prove or disprove full-editor embedding as a native child window before the UI is built on it
([13 §2.4](13-product-modes-and-live-sessions.md)).

Deliverables

- Minimal `egui-winit` + `egui-wgpu` app (dock layout, timeline-like canvas, 60 fps animated texture, IME text
  field) running top-level and as a child window in a Win32 reference host and a minimal Electron host.
- Embed channel prototype (attach, resize, focus, ping) on a named pipe.
- Measurements against the go criteria; one-day survey of candidate L2 host extension APIs.
- Decision recorded in [ADR-0014](adr/0014-integration-levels-and-native-embedding.md).

**Exit criteria**

- Go: frame pacing and input indistinguishable from top-level, artifact-free resize, correct focus and IME,
  survivable hang/crash on either side, presentation on the child window — or
- No-go: a second time-boxed spike (≤ 1 week, added to the schedule) on shared-texture composition, and the
  chosen L1 mechanism recorded. Either outcome keeps full embedding as the goal.

## 4. Phase B — Headless pipeline (weeks 5–9)

**Goal:** import → trim → export, end to end, without a UI.

Deliverables

- `arc-media`: FFmpeg in-process decode/encode, hardware-first; error codes and wrapper panics mapped to
  per-clip `MediaError`; subprocess import check (`ffprobe` + test decode, timeout) and quarantine
  ([04 §1.2](04-media-pipeline.md)).
- `arc-render`: offscreen device + minimal compositor (single track, transform, color convert).
- `arcline-cli render`: range, resolution, preset; `export_report.json`.
- Windows FFmpeg LGPL build pipeline pinned in the repo; macOS/Linux CI per ADR-0011.

**Exit criteria** (status as of 2026-11-25, batch 2; evidence in
[HANDOVER §2](../HANDOVER.md), the [audit follow-up](../reports/audit-follow-up-2026-11-25.md) §4 and
the [Phase B closure report](../reports/phase-b-exit-2026-11-25.md))

- **Met** — `arcline-cli render in.mov out.mp4 --range …` is frame-accurate against a reference decode.
  `render_e2e` verifies every output frame (FFV1, H.264, ranges); long-GOP, B-frame and MPEG-TS
  sources are frame-exact in `seek_accuracy` (audit F-06/F-08 fixed).
- **Met on the reference machine** — hardware H.264/HEVC and AV1 (SVT) encode paths work.
  H.264 (`h264_qsv`), HEVC (`hevc_qsv`) and AV1 (`libsvtav1`) are tested end to end; HEVC is
  validated with the pinned ffprobe (codec/resolution/frame rate/pixel format) in
  `render_e2e::hevc_exports_are_decodable_and_match_the_spec`. The shipped package has **no software
  HEVC encoder** (libx265 is GPL and excluded by D1), so a machine without a hardware HEVC encoder
  gets a typed `NoEncoder` error naming the candidates; the assertions skip on such machines (CI
  runners have no GPU).
- **Met** — a corrupt file produces a clean per-clip error; a file that crashes or hangs
  `ffprobe`/`ffmpeg` in the import check is quarantined and the parent process survives. The
  generated corpus (`arc-media/tests/malformed.rs`, nine fixtures under 2 MB) is refused by the
  import check (truncation is detected through a frame-count check) and converges to typed
  `MediaError`s; a mixed good/broken export completes with the broken asset named as unavailable
  (`render_e2e::mixed_media_export_survives_an_unreadable_asset`). The quarantine branch itself is
  covered by `arc-media/tests/quarantine.rs`: a generated stub tool crashes (NTSTATUS 0xC0000005),
  hangs (killed at the 20 s probe timeout) and prints garbage, and the check returns
  `Quarantined`/`Offline` in bounded time while real media still imports in the same process. A
  *real* crashing-media fixture (fuzzed decoder output) remains future work.
- **Met** — Windows FFmpeg LGPL build pipeline pinned in the repo (`third_party/ffmpeg/`, manifest
  with hashes, SBOM, `verify.sh`; FFmpeg 9.1, D1 LGPL v2.1+).
- **Met** — `.github/workflows/ci.yml`: on GitHub, **all four jobs are green** (runs 7 and 9):
  `core` on ubuntu/windows/macos (Rust 1.95 + clippy, the FFmpeg-free crates) and the Windows
  `media` job (MSYS2 + the in-repo FFmpeg 9.1 build with caching, then the full workspace
  build/test/clippy under Rust 1.95 + the WARP software adapter). The two media causes found in
  batch 3 are fixed: the FFmpeg package now installs where the cache/`ARCLINE_FFMPEG_DIR` expect it
  (`HOME="$(cygpath "$USERPROFILE")"`), and the GPU tests are serialized per test binary. Failures
  emit check-run annotations readable without a token — see the closure report §3.2/§4.

## 5. Phase C — Compositor and shell (weeks 10–16)

**Goal:** the editor exists: timeline, preview, multi-track compositing.

**Status (2026-11-26):** plan written ([docs/specs/phase-c-plan.md](specs/phase-c-plan.md)); first slice **C1** (headless preview frame pipeline, `arc_cli::preview`) implemented with pixel parity to the export path - see [reports/phase-c-c1-2026-11-26.md](../reports/phase-c-c1-2026-11-26.md). Next: C2 scheduler, then C3 shell/viewer.

Deliverables

- Direct compositor: tracks, transforms, effects chain, transitions, overlays.
- Six built-in effects, text/title renderer, frame cache.
- `arc-app`: `egui` shell on the `egui-winit` + `egui-wgpu` host loop from Phase P (top-level and child-window
  modes), docking, media bin, inspector generated from manifests, viewer via `egui_wgpu`.
- Timeline canvas: paint, virtualization, LOD, hit testing, move/trim/split, snapping.
- Preview scheduler with audio-clock placeholder and two-level quality fallback.
- Command palette, keymap, native dialogs, IME.

**Exit criteria**

- 1080p preview ≥ 30 fps with 4 tracks and 8 effects on both reference machines.
- A user can import, arrange, trim, split, and scrub a project entirely in the GUI.
- IME works for CJK title entry on Windows.

## 6. Phase D — Audio (weeks 17–20)

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

## 7. Phase E — Project lifecycle (weeks 21–24)

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

## 8. Phase F — Agent surface and live sessions (weeks 25–31)

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

## 9. Phase J — Agent workspace and embedding (weeks 32–37)

**Goal:** both product modes are pleasant to use, and the full editor runs embedded (L1).

Deliverables

- **L1 embedding, production:** embed channel complete ([13 §2.2](13-product-modes-and-live-sessions.md)),
  child-window mode in the `arc-app` host loop, focus/shortcut handoff, DPI, watchdog and *Pop out* fallback,
  Agent panel hidden when embedded, reference host (`examples/embed_host`) in CI, embed-channel spec for our
  agent client. (If the PoC chose shared-texture composition, this item implements that mechanism instead.)

- **Agent panel** (Standalone): Messages API loop on a worker thread, streaming, tool-call display with
  undo links, confirmations, OS credential storage, first-use disclosure ([13 §7](13-product-modes-and-live-sessions.md)).
- **Companion window** (L3): full editor with a compact starting layout; `--launch` opens it.
- **MCP Apps view** `ui://arcline/workspace`: preview monitor, timeline overview, control buttons, job
  progress ([13 §8](13-product-modes-and-live-sessions.md)); tested in Claude Desktop and VS Code.

**Exit criteria**

- Standalone: a user asks the Agent panel for a captioned rough cut of imported footage, reviews it, undoes
  one agent step, and exports — without leaving Arcline.
- L1: the full editor runs inside the reference host; the [00 §9](00-product-scope.md) L1 scenario passes,
  including takeover in the embedded window and watchdog fallback to the companion window without session
  loss.
- L3: the [00 §9](00-product-scope.md) L3 scenario passes in Claude Desktop with the MCP Apps view, and in one
  host without MCP Apps using the companion window and image results.
- Agent-panel replay tests pass offline; no key material appears in logs or crash bundles.

## 10. Phase G — Finish (weeks 38–44)

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

## 11. Phase I — Interchange export (weeks 45–49)

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

## 12. Phase H — Harden (weeks 50–53)

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

## 13. Explicit v1 cut list

No plugins (OFX/WASM/VST3); no plugins for third-party video editors; no L2 host adapters until a host is
confirmed; no embedding into hosts without their cooperation; no remote or multi-user agent access; no HDR/ACES/OCIO; no collaboration; no render
farm/multi-GPU; no project import from other editors, no EDL/AAF, no native Jianying/CapCut drafts; no
multicam; no screen-reader certification; no image sequences; no
audio EQ/compressor/loudness; no three-OS installer matrix.

These are not failures. They are the difference between shipping and not.

## 14. Effort assumptions

| Phase | Weeks | Notes |
|---|---|---|
| A | 3 | pure logic, very AI-friendly |
| P | 1 | embedding PoC (+1 if the no-go spike is needed) |
| B | 5 | FFmpeg build matrix is the schedule risk |
| C | 7 | UI work; the largest phase |
| D | 4 | cpal + sync; mostly known |
| E | 4 | persistence; test-heavy |
| F | 7 | protocol + MCP (5) + local IPC, attach, control, sync (2) |
| J | 6 | L1 embedding (2), Agent panel (2), MCP Apps view (1.5), companion window (0.5) |
| G | 7 | broad surface; cut aggressively if late |
| I | 5 | 1 week lab + 4 weeks writers, packaging, reports |
| H | 4 | packaging + hardening |
| **Total** | **~53** | sequential; v1.0 (Windows) ≈ week 53; v1.1 macOS port +3–4 (+ macOS embedding mechanism) |

If a phase slips beyond its estimate by more than 30%, cut from Phase G's list before extending the
schedule — the v1 promise is a working editor with an agent surface, not a feature checklist. Within
Phase I, cut in this order: OTIO for Premiere, consolidate/transcode modes, stems; never the report.

If the release date matters more than scope, these cuts bring v1.0 toward ~week 45, in this order:
1. MCP Apps view → companion window + image results only (−1.5 weeks).
2. Agent panel → external agent clients only (−2 weeks).
3. Phase I → FCPXML + FCP7 XML only, no OTIO; Jianying/CapCut lab deferred (−1.5 weeks).
4. Phase G → keyframe UI and chroma key deferred (−3 weeks).

Never cut: full-editor embedding for our own client (L1), live attach, takeover, the journal guarantees, or
the compatibility report.

## 15. KPIs

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
