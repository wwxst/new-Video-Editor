# 01 — System Architecture

This document is the spine of the proposal. It describes a system sized for one developer with AI
assistance: six crates, two product modes (Standalone and Embedded, [13](13-product-modes-and-live-sessions.md)),
and no machinery that does not serve a v1 user.

## 1. Design principles

1. **One core, one session server, many clients.** The engine is a library. The GUI (`arc-app`) and the
   headless tool (`arc-cli`) host the same session server; humans, the built-in Agent panel, and external
   agents (attached or headless) all drive it through the same command model.
2. **Commands are the only way to change state.** No component mutates the project directly. A GUI drag is
   a `MoveClip` command; an agent call is the same command. This buys undo, audit, crash recovery, and
   dry-run from one mechanism.
3. **Immutable snapshots, single writer.** The document is published as an immutable `Arc<Project>`.
   Readers never lock. One engine thread writes.
4. **Reproducible on the machine that made it.** Same project + commands + renderer version ⇒ same output
   on the same hardware. Cross-device bit-exactness is not promised ([03 §9](03-render-engine.md)).
5. **Native, no web runtime.** `wgpu` for rendering, `egui` for the shell. No Chromium, no WebView, no JS.
6. **Simplicity before robustness theater.** In-process decode, single-process export, one fallback quality
   level. Process isolation and parallel export are documented upgrade paths, not v1 work.
7. **Budgets, not vibes.** Every subsystem has numbers with CI gates ([08](08-performance-reliability.md)).
8. **Windows first.** v1.0 ships on Windows only; macOS ships in v1.1; Linux is post-1.0 with no date
   commitment ([ADR-0011](adr/0011-windows-first.md)).

## 2. Platform matrix

| Concern | Windows (v1.0) | macOS (v1.1) | Linux (post-1.0, unscheduled) |
|---|---|---|---|
| Graphics API | D3D12 (Vulkan fallback) | Metal | Vulkan |
| GPU abstraction | `wgpu` | `wgpu` | `wgpu` |
| Hardware decode | D3D11VA/D3D12VA via FFmpeg | VideoToolbox | VAAPI/NVDEC |
| Hardware encode | NVENC / QSV / AMF | VideoToolbox | NVENC / VAAPI |
| Audio device | WASAPI (shared) | CoreAudio | PipeWire/ALSA |
| Windowing | `winit` (via `egui-winit`; top-level or child window) | `winit` | `winit` |
| Packaging | Portable zip + MSIX, signed | DMG, notarized, universal | Flatpak (community) |

Minimums: Windows 10 21H2 (x64) for v1.0; macOS 13 for v1.1; Ubuntu 22.04 when Linux support starts. Code
stays portable from day one and CI compiles on all three OSes, but only Windows is a shipping target in
v1.0. The release plan and per-platform CI tiers are defined once in [ADR-0011](adr/0011-windows-first.md).

## 3. Deployment topologies

One engine, three shapes. A and B are the two product modes; C is the headless form used by B without a
window and by batch jobs.

```
 A. Standalone Editor                         B. Embedded Editor, level L3 (live attach)
 ┌──────────────────────────────────────┐     ┌──────────────┐  MCP stdio  ┌─────────────────────────┐
 │ arcline  (arc-app)                   │     │ agent client │◄───────────►│ arcline-cli mcp --attach│
 │  ├── UI thread: egui layout + paint  │     │ (Claude      │             │ (bridge, no document)   │
 │  ├── Agent panel worker (optional)   │     │  Desktop,    │             └────────────┬────────────┘
 │  ├── session server + IPC listener ◄─┼─────┼──VS Code, …) │  MCP Apps view            │ named pipe
 │  ├── engine thread (single writer)   │     │  ui://arcline│  (in host iframe)         │ (per-user ACL)
 │  ├── render thread (wgpu compositor) │     └──────────────┘                           │
 │  ├── decode threads (in-process)     │◄──────────────────────────────────────────────┘
 │  ├── audio thread (cpal callback)    │     same arcline process, companion layout
 │  └── journal writer thread           │
 └───────┬──────────────────────────────┘
         │ spawns (argument vectors, no shell)
         ├── ffprobe / ffmpeg      import check, proxies, thumbnails, waveforms
         └── arcline-cli render    exports

 B1. Embedded Editor, levels L1/L2 (full editor inside the host window)
 ┌──────────────────────────── agent client window (ours, or a confirmed L2 host) ──────────────┐
 │  conversation UI   │ ┌────────────────── host-reserved region ─────────────────────────────┐ │
 │                    │ │  arcline.exe child window: full egui + wgpu editor                  │ │
 │  host adapter ─────┼─┼─► embed channel (named pipe): attach/resize/focus/ping             │ │
 │  agent ── MCP ─────┼─┼─► session (named pipe): commands, notifications, control           │ │
 │                    │ └─────────────────────────────────────────────────────────────────────┘ │
 └──────────────────────────────────────────────────────────────────────────────────────────────┘
   separate processes; watchdog falls back to a companion window without losing the session

 C. Headless (batch, CI, agents without a window)
 ┌──────────────┐   JSON-RPC / MCP    ┌──────────────────────────────┐
 │ agent / MCP  │◄── stdio (NDJSON) ─►│ arcline-cli mcp --project …  │
 │ host         │                     │  session, commands, render   │
 └──────────────┘                     └──────────────────────────────┘
```

In B and B1, every command runs in the GUI's engine, so the human sees each edit as it lands and can take
over at any time. B is integration level L3 (companion window + MCP Apps view); B1 is L1/L2 (native child
window in the host). Integration levels, the embed channel, local IPC, consent, control and takeover, and
state synchronization are defined in [13](13-product-modes-and-live-sessions.md).

**No decode worker process in v1.** Preview decode runs FFmpeg in-process behind a `Decoder` trait. Error
codes and Rust panics are recovered per clip, but a native fault inside FFmpeg (segfault, `abort`) ends the
process — `catch_unwind` cannot prevent that. v1 contains the risk instead: untested media is checked in a
child process before it is decoded in-process, crash breadcrumbs quarantine any file implicated in a crash,
exports run in a child `arcline-cli render` process, and the journal guarantees no acknowledged edit is lost
([04 §1](04-media-pipeline.md), [08 §4](08-performance-reliability.md)). Moving preview decode into a worker
process remains an additive upgrade ([ADR-0003](adr/0003-ffmpeg-for-media-io.md)).

## 4. Crate graph

Six crates, with enforced dependency rules (checked in CI from `cargo metadata`):

```
   arc-app (egui GUI, Agent panel, IPC listener)
      │
      ▼
   arc-cli (library: session server, JSON-RPC, MCP, tool registry, jobs  +  arcline-cli binary)
      │
      ├──────────────┬──────────────┬──────────────┐
      ▼              ▼              ▼              │
   arc-render     arc-audio      arc-media         │
   (wgpu)         (cpal)         (FFmpeg)          │
      │              │              │              │
      └──────────────┴──────────────┴──────────────┴──► arc-core (no GPU, no FFmpeg, no UI)
```

| Crate | Owns | May depend on | Must not depend on |
|---|---|---|---|
| `arc-core` | time, model, commands, validation, undo/journal, project file, schema migrations, interchange writers (pure) | std, serde, `im` | GPU, FFmpeg, UI, audio devices |
| `arc-media` | probe, decode, encode, mux, proxy/thumbnail/waveform jobs | core, FFmpeg | render, UI |
| `arc-render` | `wgpu` device, compositor, effects, text, frame cache | core, `wgpu` | FFmpeg, UI |
| `arc-audio` | device output, mixer, offline render | core, `cpal` | render, UI |
| `arc-app` | `egui` shell, timeline canvas, viewer, Agent panel, IPC listener, consent prompts, app binary | all of the above, including the `arc-cli` library | — |
| `arc-cli` | library: session server, JSON-RPC/MCP, tool registry, MCP Apps resource, attach bridge, jobs; binary: headless render, interchange export, utilities | core, media, render, audio | `egui`, `arc-app` |

Why this shape:

- `arc-core` has **no GPU and no FFmpeg**, so the document model and command semantics are testable
  headlessly with `cargo test`.
- Effect manifests live in `arc-render`, next to the shaders, and are consumed by the UI and MCP alike.
- `arc-cli` renders offscreen with the same `arc-render` code the GUI uses, so export and preview cannot
  diverge.
- Six crates is the smallest split that preserves these properties. Fewer boundaries mean less Cargo
  plumbing and fewer cross-file contexts for AI-assisted edits.

Dependency rule enforcement: a CI script parses `cargo metadata` and fails on forbidden edges. Module-level
boundaries inside `arc-core` (e.g. `model` vs `command`) carry the discipline that separate crates used to.

## 5. Runtime: threads and ownership

```
 ┌────────────────── UI THREAD (egui-winit/egui) ───────────────────┐
 │ event loop · egui layout · paint · timeline canvas · presenter   │
 │ never blocks > 4 ms; missing data renders as stale, never stall  │
 └───────┬───────────────────────────────────────────────┬─────────┘
         │ command batch (mpsc)                          │ frame request
         ▼                                               ▼
 ┌──────────── ENGINE THREAD ────────────┐     ┌──────── RENDER THREAD ─────────┐
 │ single writer · validate · apply ·    │     │ direct compositor · GPU submit │
 │ publish Arc<Project> · journal        │────►│ frame cache · readback/export  │
 └───────────────┬───────────────────────┘     └───────────────┬────────────────┘
                 │ snapshots                                   │ frame upload
                 ▼                                             ▼
 ┌─────────── SESSION (arc-cli) ─────────┐        ┌──── DECODE THREADS ─────────┐
 │ JSON-RPC dispatch · MCP tools · jobs  │        │ per-clip decoders · seek ·  │
 └───────────────────────────────────────┘        │ YUV → GPU upload            │
                                                  └─────────────────────────────┘

 ┌──────── AUDIO THREAD (cpal callback, no alloc, no locks) ────────┐
 │ ring buffer ← mixed blocks from audio worker · device clock      │
 └──────────────────────────────────────────────────────────────────┘
```

Rules:

- The **engine thread** owns the document and never blocks on I/O. It publishes each applied batch
  speculatively and hands its record to the **journal writer thread**; every client — GUI and protocol —
  receives the `ApplyResult` only after the record is completely written ([08 §4.3](08-performance-reliability.md)).
  On a journal write failure, speculative batches are rolled back. Commands that reference files (`asset.import`,
  `subtitle.import`, LUTs) are read/probed by the session layer *before* they reach the engine.
- The **UI thread** never waits on decode. A missing frame shows the nearest cached frame with a stale
  indicator.
- The **audio callback** allocates nothing and takes no locks; it drains a pre-filled SPSC ring.
- The **render thread** uses the same `wgpu::Device`/`Queue` created by `egui-wgpu`; `wgpu` queues are
  thread-safe, and the viewer presents the resulting texture through `egui_wgpu` user textures.
- Snapshots are cheap: `Project` uses persistent collections (`im::Vector`, `im::HashMap`), so a 1,000-clip
  edit produces an O(log n) new root. Target ≤ 200 µs per applied batch.

No async runtime is used in v1. Threads and channels cover the workload; `tokio` is added only if a real
network requirement appears.

## 6. The protocol

JSON-RPC 2.0, newline-delimited, over **stdio** and **local IPC** (per-user named pipe on Windows,
[13 §3](13-product-modes-and-live-sessions.md)) in v1, plus an in-process transport for the Agent panel.
WebSocket is a later transport, not a new implementation.

### 6.1 Methods

Fifteen methods in five groups, plus four notifications. Full signatures are defined once, in [06 §4.1](06-agent-interface.md):

| Group | Methods |
|---|---|
| Project | `project.open`, `project.describe`, `project.apply`, `project.save` |
| History | `history.undo`, `history.redo`, `history.list` |
| Inspect | `effect.list`, `inspect.timeline`, `inspect.frame` |
| Jobs | `job.submit`, `job.status`, `job.cancel` |
| Control | `control.request`, `control.release` |

Every edit goes through `project.apply`; `history.undo`/`history.redo` produce new revisions from stored
inverses; the remaining methods read state, save, or run jobs ([02 §4](02-domain-model-and-project-format.md)).

### 6.2 Semantics

- **Revisions.** Every committed batch increments `revision` by one. A stale `base_revision` fails with
  `stale_revision` and the current revision. Optimistic concurrency, no locks.
- **Batches are atomic.** The whole batch is validated against a simulated snapshot before the first
  mutation; it applies fully or not at all.
- **Idempotency.** A repeated key returns the original result; retries after timeouts are safe.
- **Dry run.** Validation plus a structured diff, no commit.
- **Typed errors.** `{code, kind, message, path?, details?}` with stable `kind` strings
  (`unknown_clip`, `overlap`, `unsupported_codec`, `path_denied`, …).
- **Versioning.** Handshake reports `{protocol: "1.0", schema: 7, renderer: "…"}`; unknown fields are
  ignored; unknown methods return `unsupported_method`.

### 6.3 Trust model (v1)

Local, same user: every process runs with the user's privileges. `arcline-cli` takes `--root` (media
allow-list) and `--output-root` (render destinations) and rejects paths outside them before any syscall.
Live sessions add a user-only pipe ACL, a discovery token, a per-client consent prompt, and human takeover
([13 §3–5](13-product-modes-and-live-sessions.md)). Capability tokens, remote access, and multi-tenant
isolation are deferred; see [06 §8](06-agent-interface.md).

## 7. Data flow: one edit, end to end

GUI drag of a clip:

```
1. UI thread      pointer drag ends → MoveClip{clip, to_track, to_start}
2. UI thread      sends CommandBatch{base_revision, cmds} to the engine (non-blocking)
3. Engine thread  validates on a simulated snapshot:
                    track kind matches, no overlap after the explicit ripple policy,
                    times are exact rationals on frame boundaries
4. Engine thread  applies → new immutable snapshot (structural sharing)
                  bumps revision, publishes speculatively via arc-swap, queues the journal record
5. Render thread  sees the new snapshot; invalidates cache keys for the affected range
6. Journal writer completes the write of the record → batch committed (survives process crash)
7. UI thread      receives ApplyResult (only now); the pending marker and optimistic ghost are reconciled
8. Autosave       snapshot every 30 batches / 60 s; group fsync per 08 §4.4
```

The UI moves a **ghost** optimistically during drag; authoritative state always comes from the engine. If
validation fails, the ghost animates back and the typed error is shown. An agent experiences the same flow
without the ghost: its `project.apply` response is the same `ApplyResult`, sent at step 7. If step 6 fails,
the batch is rolled back and the client receives `journal_unavailable`.

## 8. Startup and project open

Budget: cold start to usable window ≤ 2 s; open a 1,000-clip project ≤ 2 s.

```
main → config → logging/crash handler → wgpu adapter (via egui-wgpu)
     → window shown with last layout skeleton → spawn engine
     → open project if given:
          read manifest (JSON) → validate schema → migrate if older
          → resolve asset refs (missing files become offline assets)
          → publish first snapshot → shader cache warm-up
```

Project open never blocks on media probing: probe results are cached in the container and refreshed in the
background. Shader pipelines are cached per driver to avoid first-frame hitches.

## 9. Error model

| Layer | Example | Handling |
|---|---|---|
| **Domain error** | overlap, unknown clip, unsupported codec | Typed `CommandError`, returned to the client, no log noise |
| **Operational error** | FFmpeg error code, Rust panic in decoder wrapper, GPU device lost, journal write failure | Recovered: per-clip `MediaError`, recreate device; on journal failure roll back unacknowledged batches and enter read-only degraded mode |
| **Defect** | invariant violation in the engine | Engine thread catches panics per batch, marks the session degraded, keeps the last good snapshot exportable, writes a crash bundle |
| **Fatal** | native fault in FFmpeg or a driver (segfault, `abort`) | Process ends. Next launch recovers from journal and quarantines the implicated asset ([04 §1.2](04-media-pipeline.md)) |

No Rust panic crosses into the UI thread. `catch_unwind` only handles Rust panics; it never handles native
faults and is never relied on for them.

## 10. Extension points

| Point | Mechanism | Status |
|---|---|---|
| New built-in effect | WGSL pass + JSON manifest registered in `arc-render` | v1 |
| Automation | JSON-RPC / MCP / CLI, headless or live-attached | v1 |
| In-client view | MCP Apps resource `ui://arcline/workspace` | v1 ([13 §8](13-product-modes-and-live-sessions.md)) |
| Full editor embedding in agent clients | out-of-process native child window + embed channel; host adapters | v1.0 for our client (L1, PoC-gated); per confirmed host (L2) ([13 §2](13-product-modes-and-live-sessions.md)) |
| Third-party effect plugins | OFX / WASM | Post-1.0 ([09](09-effects-and-extensions.md)) |
| Interchange export | FCPXML / FCP7 XML / OTIO writers in `arc-core::interchange`, export job in `arc-cli` | v1.0 ([12](12-project-interchange.md)) |
| Interchange import | OTIO first | Post-1.0 |
