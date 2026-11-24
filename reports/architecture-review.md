# Independent Architecture Review — Arcline (`wwxst/new-Video-Editor`)

**Reviewed:** initial commit `043e841` — 28 documents, no source code.
**Lens:** the stated goal is a professional, agent-native editor built by **one developer with AI doing most
of the engineering**. The proposal was written as if for a 9–10 person team; that mismatch is the root of
most findings below.

---

## Verdict

The conceptual spine is sound and worth keeping: **Rust core, command-driven immutable document, exact
rational time, FFmpeg for codecs, `wgpu` rendering, MCP as a first-class surface.** These are the hard
problems, and the proposal solves them correctly.

The delivery architecture is over-built for a solo effort. Four decisions drive most of the cost:
a custom widget toolkit, an 18-crate workspace, premature robustness (decode subprocess, parallel export
processes, plugin sandboxes), and a 60-week plan sized for ~10 FTE. Fix those and the project is viable in
roughly 6–7 months to a usable Windows v1.

| Area | Verdict |
|---|---|
| Rust | **Keep** |
| `wgpu` | **Keep** |
| Custom UI toolkit | **Replace** with `egui` + custom canvases |
| FFmpeg / codec posture | **Keep**, simplify integration |
| Decode subprocess, export fan-out | **Defer** |
| Render graph | **Simplify** to a direct compositor |
| 18 crates | **Collapse to 6** |
| Command model / MCP | **Keep**, trim the trust machinery |
| 60-week roadmap | **Rewrite** for solo + AI |

---

## 1. Is Rust + `wgpu` + a custom UI the right approach?

### Rust — yes

The requirements (real-time audio, per-frame allocation discipline, FFmpeg FFI, decade-long maintenance)
justify it. AI assistance is unusually effective in Rust because the compiler catches most cross-file
mistakes, and the ecosystem (`wgpu`, `egui`, `cpal`, `ffmpeg-next`, `serde`) covers every need. The
proposal's ADR-0001 arguments hold; nothing here needs revisiting.

### `wgpu` — yes, with one caveat

One API over Vulkan/Metal/D3D12 is the right trade for a solo developer. The caveat is hardware-video
interop: `wgpu` does not expose platform decode surfaces uniformly, so decoded YUV frames will be uploaded
and color-converted on the GPU. That is fine at 1080p/4K preview and removes an entire class of
platform-specific code. The proposal already plans this path; it should be the *only* path in v1.

### Custom UI — no; this is the single biggest mistake

The proposal itself estimates 3–4 engineer-months for "basic parity with a stock toolkit." With AI that is
maybe 6–8 weeks for the basics — but the long tail is the killer: IME, text editing, native dialogs,
OS drag-and-drop, docking, multi-window, accessibility, HiDPI quirks, and the hundred small interactions
users judge an editor by. This is a months-long detour that produces no video-editing capability.

**Recommendation: `egui` for the shell, custom `wgpu` canvases for timeline and viewer.**

- `eframe` (wgpu backend) + `egui_dock` + `egui_extras` gives docking, tables, menus, dialogs, and text
  editing on day one.
- The viewer is a `wgpu` texture shown through `egui_wgpu`'s user-texture path — one device, no copies.
- The timeline is first painted with `egui::Painter` (thousands of shapes are fine on desktop), and only
  moved to a dedicated render pass if profiling proves it necessary. Do not build a second UI stack on
  speculation.

Honest costs of `egui`: it will not look native, text input is less polished than Qt, and multi-window is
limited. For a solo project those are acceptable; the alternative costs months. Keep the UI behind a thin
app-level boundary so a future swap (Slint/Qt) remains possible — but do not build the abstraction now.

**Alternatives considered:** Qt — best chrome, worst integration with a `wgpu` viewport, LGPL/commercial
complexity. Tauri/Electron — violates the native requirement and complicates GPU surfaces. `iced`/`slint` —
better looking than `egui`, but slower iteration and weaker custom-canvas ergonomics for an editor.

---

## 2. Is the stack and cross-platform strategy appropriate?

Mostly, but the strategy is wrong in sequencing.

**Keep:** Rust 2024, `wgpu`, FFmpeg `libav*`, `serde`/JSON project format, `cpal`, `cosmic-text` for canvas
text. The FFmpeg LGPL posture and hardware-encoder-only H.264/HEVC stance (AV1 as the free default) are
correct and should not be softened — they are business constraints, not technical preferences.

**Change the cross-platform plan: Windows-first.**

- The proposal targets three signed installers by week 60. For a solo developer, notarization, MSIX, Flatpak,
  and three FFmpeg build matrices are weeks of unpaid work per OS, repeated every release.
- Build Windows-only in v1; keep code portable; run **compile checks** on macOS/Linux in CI from week 2 to
  prevent bit-rot, but ship installers only for Windows. Add macOS when there is a user asking for it;
  Linux via Flatpak/community later.
- This alone removes roughly 15–20% of the roadmap without touching the product.

**FFmpeg integration:** `libav` FFI in-process behind a `Decoder`/`Encoder` trait. The decode subprocess in
the proposal is a robustness feature priced for a team; it adds IPC, shared memory, a second binary, and a
whole failure-recovery surface. Defer it. Catch panics at the decoder boundary, autosave aggressively, and
keep the trait shaped so process isolation can be added later without touching callers.

**Use the `ffmpeg` CLI for the boring jobs.** Proxy generation, thumbnails, waveform peaks, and media
probing at import are one-shot transcodes, not interactive paths. Shelling out to the bundled `ffmpeg`
binary for these saves a lot of code (and can run as a background job). Reserve the FFI path for
frame-accurate decode and encode.

**Drop from v1:** OFX hosting, WASM effects, VST3, EDL/FCPXML/OTIO interchange, OCIO/ACES/HDR. Each is a
multi-week subsystem with no v1 user. The proposal schedules them late already; remove them from the plan
entirely and revisit after 1.0.

---

## 3. Is the video engine and GPU architecture unnecessarily complex?

Yes, in five places, all of which can be simplified without losing visible capability.

| Proposed | Problem for solo | Recommendation |
|---|---|---|
| Generic pull-based render graph with planner, texture aliasing, node cache | A graph is justified when you have many execution strategies; v1 has one | **Direct compositor function**: loop active tracks bottom→top, render each clip's effect chain to a layer texture, blend, resolve. Build the graph later only if a second consumer appears |
| Decode subprocess + shared-memory ring | Days of plumbing, a second binary, custom IPC | **In-process decoder** behind a trait; panic isolation at the call site |
| Parallel export via N render processes with segment checkpoints | Substantial orchestration for a speedup hardware encoders already provide | **Single-process export**: render → feed encoder → mux. 1080p H.264 exceeds realtime on any modern GPU encoder |
| Frame cache L0 (GPU) + L1 (disk) | Disk cache adds invalidation and serialization complexity | **In-memory LRU only**, keyed by `(clip, source_frame, param_hash)`. Add disk cache only if measurements demand it |
| Capability tiers, 5-level quality governor, reference render mode | Solves problems a single-developer product does not yet have | **One quality fallback** (full → half-res on deadline misses). Probe adapter features ad hoc |

**Keep** the parts of the render design that are cheap and high-value: WGSL effects with JSON manifests
(one source of truth for UI and MCP), a texture pool, half-resolution intermediates for heavy effects,
audio-master-clock playback, and frame-granular seeking.

**Color:** internal linear RGBA16F is cheap and correct — keep. But v1 should be **Rec.709 SDR only**, with
3D LUT support. HDR10/HLG, ACEScg, OCIO, and tone-mapping policies are a finishing-suite problem; defer.

**Determinism:** keep same-machine reproducibility and golden-frame tests; drop the cross-device contract
and the `--render-mode=reference` machinery. Renderer-version attribution in the export report is cheap —
keep that.

**Audio:** the design is right but oversized for v1. Keep the sample-accurate mix graph and audio-master
clock; defer EQ/compressor/limiter/loudness normalization to v1.1. Fades, gain, pan, mute/solo and meters
are enough to ship.

Net effect: roughly half the engine code disappears, and none of the user-visible v1 features do.

---

## 4. Should the crate structure be simplified?

Yes. Eighteen crates is a team-scale decomposition. Every crate boundary is a Cargo.toml, a public API, a
dependency rule, and a place where AI-assisted edits must hold two contexts at once. Collapse to **six**:

| Crate | Contents | Rule |
|---|---|---|
| `arc-core` | time, model, commands, validation, project file, schema migrations | no GPU, no FFmpeg, no UI |
| `arc-media` | probe, decode, encode, mux, proxy jobs (FFmpeg FFI + CLI) | depends on core |
| `arc-render` | wgpu device, compositor, effects, text/title rendering, frame cache | depends on core |
| `arc-audio` | cpal output, mixer, offline render | depends on core |
| `arc-app` | egui shell, timeline canvas, viewer, app binary | depends on all |
| `arc-cli` | headless render, MCP/JSON-RPC server, project utilities | depends on core/media/render |

Absorbed: `arc-time`, `arc-command` → `core`; `arc-effects`, `arc-text`, `arc-cache` → `render`;
`arc-encode` → `media`; `arc-protocol`, `arc-server` → `cli`; `arc-ui` → deleted (egui);
`arc-plugin-host` → deleted (deferred); `arc-testkit` → a `fixtures/` directory and per-crate test helpers.

Boundary discipline lives in modules and a lint rule, not in crates. This keeps the important property —
**core is headless and testable without a GPU or FFmpeg** — while cutting build ergonomics and cross-crate
churn.

---

## 5. Is the Agent interface well-designed?

Yes — it is the strongest part of the proposal. The command-driven immutable document is what makes agents
safe, undo free, and verification possible. Retain: typed commands, atomic batches, revisions with
optimistic concurrency, `dry_run`, stable error `kind`s, generated MCP schemas, image-returning inspection
tools, and the text timeline as a read view.

Trim the parts that exist for a multi-tenant product:

- **Drop capability tokens and the live-GUI attach topology** in v1. The daemon is local, single-user, and
  runs with the user's privileges; a root-path setting plus explicit render output paths is enough. Revisit
  when remote or multi-user access exists.
- **Drop WebSocket transport** initially; stdio and a local named pipe cover MCP and CLI.
- **Defer `timeline.apply_text`.** Keep `inspect.timeline` (text output) from day one — it is cheap and
  genuinely helps LLM reasoning — but the round-trip parser/diff is a subsystem to add in v1.1.
- **Keep idempotency keys** (trivial to implement, valuable for retries).
- **Reduce the tool surface to ~18 tools** across project, media, edit, effect, inspect, render. Every extra
  tool is schema maintenance and a chance for the model to pick wrong.

One addition worth making early: `inspect_frame` should return the *rendered* frame (post-effects), not just
the source frame. Verified visual feedback is what separates agent editing from blind automation.

---

## 6. Is the roadmap realistic?

No. 60 weeks at the stated staffing is ~570 person-weeks. AI compresses typing and boilerplate, not
integration debugging, codec build matrices, or UI polish. Realistically that plan is 2–3 years solo.

The plan also front-loads infrastructure (P0 kernel, P1 pipeline) before anything a user can try. A solo
project should be **vertical-slice first**: import a clip, cut it, preview it, export it — then widen.

### Recommended roadmap (~27 weeks to usable Windows v1)

| Phase | Weeks | Deliverable | Exit criterion |
|---|---|---|---|
| **A — Spine** | 1–3 | `arc-core`: rational time, model, commands, undo, project file; CLI | Property test: apply→inverse→identical hash; 10k random command sequences pass |
| **B — Headless pipeline** | 3–7 | `arc-media`: probe, decode, encode; `arc-cli render` | `arcline render in.mov out.mp4 --trim …` is frame-accurate; hardware encode works |
| **C — Compositor** | 6–12 | `arc-render`: direct compositor, 6 effects, viewer; `egui` shell + timeline canvas | 1080p preview ≥ 30 fps with 4 tracks + 8 effects; move/trim/split works |
| **D — Audio** | 11–14 | `arc-audio`: playback, mix, fades, A/V sync | 10-minute playback drifts ≤ 1 frame; preview/export mix hashes match |
| **E — Project lifecycle** | 13–16 | save/load, autosave, recovery, relink, export presets | 100 forced kills during edit/save → zero lost sessions |
| **F — Agent surface** | 15–19 | MCP stdio server, 18 tools, text view, headless render jobs | Reference agent builds a captioned 3-shot cut end-to-end |
| **G — Finish** | 18–24 | transitions, keyframes, subtitles (import + burn-in), LUT color, proxies | A 10-minute project edited and exported by a non-developer |
| **H — Harden** | 23–27 | perf pass, crash handling, Windows installer, docs | Budgets met on reference hardware; beta with 5 external users |

macOS support: +3–4 weeks after H (mostly FFmpeg build, Metal validation, packaging). Linux: community or
Flatpak later.

### Explicit v1 cut list

No plugins (OFX/WASM/VST3), no HDR/ACES/OCIO, no collaboration, no render farm/multi-GPU, no EDL/FCPXML/OTIO,
no live-GUI agent attach, no screen-reader certification (keyboard completeness stays), no multicam.
These are not failures — they are the difference between shipping and not.

---

## What to retain

1. Rust + `wgpu` + FFmpeg core stack.
2. Command-driven document with immutable snapshots and a single writer. This is the architectural
   differentiator and the reason agents can be first-class.
3. Exact rational time (`arc-time` semantics, merged into core).
4. JSON project container with schema versioning and migrations.
5. MCP as a first-class surface, with generated schemas and image-returning inspection.
6. Audio-master-clock playback design.
7. Honest codec/licensing posture (LGPL FFmpeg build, hardware encoders, AV1 default).
8. Golden-frame and property-test discipline (scaled down to match solo maintenance).

## What to remove or simplify

| Remove | Replace with |
|---|---|
| Custom widget toolkit (`arc-ui`) | `egui` + custom timeline/viewer canvases |
| 18 crates | 6 crates |
| Decode subprocess + shared-memory transport | In-process decoder trait, panic-isolated |
| Parallel export processes / segment checkpoints | Single-process render → encode → mux |
| Generic render graph, planner, aliasing | Direct compositor function |
| Disk frame cache, 5-level governor, adapter tiers | In-memory LRU, one fallback level, ad-hoc feature probe |
| Capability tokens, live-attach daemon, WebSocket | Local stdio/pipe, root-path setting |
| `timeline.apply_text` round-trip | Read-only text view; JSON commands for writes |
| Plugin system (three tiers), HDR/ACES/OCIO, VST3, interchange formats | Deferred past 1.0 |
| 60-week, 9.5-FTE plan | 27-week, solo-calibrated plan above |

## What must be redesigned

1. **UI layer** — from custom toolkit to `egui` shell with custom canvases (biggest change).
2. **Render execution** — from generic graph to a concrete compositor; keep only the optimizations that
   measurements justify.
3. **Crate decomposition** — six crates, module-level boundaries.
4. **Cross-platform sequencing** — Windows-first, compile checks elsewhere, installers later.
5. **Roadmap and scope** — vertical slices, explicit cut list, solo-calibrated estimates.

## Final recommended stack

| Layer | Choice |
|---|---|
| Language | Rust 2024 |
| GPU | `wgpu` (via `egui-wgpu`; one device, one queue) |
| UI | `egui` + `egui_dock` + `egui_extras`; timeline/viewer as custom painting |
| Window/input | `winit` (through `eframe`) |
| Text (canvas) | `cosmic-text` + `glyphon` |
| Media | FFmpeg `libav*` (FFI, LGPL build) for decode/encode; `ffmpeg` CLI for proxies/thumbs/waveforms |
| Audio | `cpal` + custom mixer |
| Data | `serde` + `serde_json`; zip container |
| Async | std threads + channels; no async runtime in v1 |
| Protocol | newline-delimited JSON-RPC over stdio/pipe; MCP facade |
| Testing | `proptest` (time/commands), fixture media, image-diff goldens on CI |

## Residual risks

- **`egui` polish ceiling** — mitigate with theming and custom canvases; accept "tool-like" aesthetics for v1.
- **FFmpeg FFI setup on Windows** — pin a prebuilt `libav` distribution early in Phase A; do not let this
  block Phase B.
- **Scope creep** — the cut list is the contract; no plugin or HDR work before H is complete.
- **AI code quality** — small PRs, property tests on the command layer, and a strict "core has no GPU/media
  dependencies" rule keep it reviewable.

---

**Bottom line:** keep the brain, replace the body. The command model, time model, codec strategy, and agent
surface are worth building exactly as designed. The toolkit, crate count, engine machinery, and schedule are
team-scale decisions that a solo developer should not pay for. With the changes above, this is a credible
6–7 month path to a real, agent-native editor.
