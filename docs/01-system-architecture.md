# 01 — System Architecture

This document is the spine of the proposal. It describes a system sized for one developer with AI
assistance: six crates, two deployment shapes, and no machinery that does not serve a v1 user.

## 1. Design principles

1. **One core, two clients.** The engine is a library. The GUI (`arc-app`) and the headless tool
   (`arc-cli`) are its only clients, and both drive it through the same command model.
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
8. **Windows first.** Windows is the primary target through v1.0; macOS follows; Linux is post-1.0
   ([ADR-0011](adr/0011-windows-first.md)).

## 2. Platform matrix

| Concern | Windows (v1.0) | macOS (follow-up) | Linux (post-1.0) |
|---|---|---|---|
| Graphics API | D3D12 (Vulkan fallback) | Metal | Vulkan |
| GPU abstraction | `wgpu` | `wgpu` | `wgpu` |
| Hardware decode | D3D11VA/D3D12VA via FFmpeg | VideoToolbox | VAAPI/NVDEC |
| Hardware encode | NVENC / QSV / AMF | VideoToolbox | NVENC / VAAPI |
| Audio device | WASAPI (shared) | CoreAudio | PipeWire/ALSA |
| Windowing | `winit` (via `eframe`) | `winit` | `winit` |
| Packaging | Portable zip + MSIX, signed | DMG, notarized, universal | Flatpak (community) |

Minimums: Windows 10 21H2; macOS 13; Ubuntu 22.04 when Linux support starts. Code stays portable from day
one, and CI compiles on all three OSes to prevent bit-rot, but only Windows is a shipping target in v1.

## 3. Deployment topologies

Two shapes, one engine:

```
 A. Desktop (primary)
 ┌──────────────────────────────────────┐
 │ arcline  (arc-app)                   │
 │  ├── UI thread: egui layout + paint  │
 │  ├── engine thread (single writer)   │
 │  ├── render thread (wgpu compositor) │
 │  ├── decode threads (in-process)     │
 │  └── audio thread (cpal callback)    │
 └──────────────────────────────────────┘

 B. Headless (agents, batch)
 ┌──────────────┐   JSON-RPC / MCP    ┌──────────────────────────────┐
 │ agent / MCP  │◄── stdio (NDJSON) ─►│ arcline-cli mcp --project …  │
 │ host         │                     │  session, commands, render   │
 └──────────────┘                     └──────────────────────────────┘
```

Topology B is how agents work in v1: a process that opens a project file, applies commands, and renders.
Attaching an agent to a *live GUI session* (a control socket) is a v1.1+ feature; it is explicitly not in
v1 ([06 §2](06-agent-interface.md)). This removes an entire IPC surface and trust model from the first
release while keeping the protocol that will make it possible later.

**No decode subprocess in v1.** FFmpeg runs in-process behind a `Decoder` trait, with panics caught at the
call boundary and autosave limiting blast radius. The trait shape permits moving decode into a worker
process later without changing callers. This is the largest deliberate robustness trade in v1, and it is
recorded in [ADR-0003](adr/0003-ffmpeg-for-media-io.md).

## 4. Crate graph

Six crates, with enforced dependency rules (checked in CI from `cargo metadata`):

```
              ┌───────────┐        ┌───────────┐
   clients    │  arc-app  │        │  arc-cli  │
              │  (egui)   │        │ (headless)│
              └─────┬─────┘        └─────┬─────┘
                    │                    │
        ┌───────────┼────────┬───────────┼─────────┐
        ▼           ▼        ▼           ▼         ▼
   ┌─────────┐ ┌─────────┐ ┌────────┐ ┌────────┐ ┌─────────┐
   │arc-render│ │arc-audio│ │arc-media│ │arc-cli │ │arc-core │
   │ (wgpu)  │ │ (cpal)  │ │(FFmpeg) │ │ uses   │ │(no GPU, │
   └────┬────┘ └────┬────┘ └───┬────┘ │ all    │ │ no FFmpeg)│
        └───────────┴───────────┴──────┴────────┴─► arc-core │
   └─────────────────────────────────────────────────────────┘
```

| Crate | Owns | May depend on | Must not depend on |
|---|---|---|---|
| `arc-core` | time, model, commands, validation, undo/journal, project file, schema migrations | std, serde, `im` | GPU, FFmpeg, UI, audio devices |
| `arc-media` | probe, decode, encode, mux, proxy/thumbnail/waveform jobs | core, FFmpeg | render, UI |
| `arc-render` | `wgpu` device, compositor, effects, text, frame cache | core, `wgpu` | FFmpeg, UI |
| `arc-audio` | device output, mixer, offline render | core, `cpal` | render, UI |
| `arc-app` | `egui` shell, timeline canvas, viewer, app binary | all of the above | — |
| `arc-cli` | headless render, JSON-RPC/MCP server, utilities | core, media, render, audio | `egui` |

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
 ┌──────────────────── UI THREAD (eframe/egui) ────────────────────┐
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

- The **engine thread** owns the document and never blocks on I/O. Persistence is an append-only journal
  written by a background task.
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

JSON-RPC 2.0, newline-delimited, over **stdio** in v1. Named pipes and WebSocket are later transports, not
new implementations.

### 6.1 Methods

```
project.open      {path, create?}        → session, schema, renderer, revision, project summary
project.describe  {session, exact?}      → compact summary (tracks, clips, issues)
project.apply     {session, base_revision, commands[], dry_run?, idempotency_key?}
                  → {revision, applied, resulting[], warnings[]}
project.save      {session}              → {path}
job.submit        {session, kind: "render", spec} → {job_id}
job.status        {job_id}               → {state, progress, report?, error?}
job.cancel        {job_id}
history.undo/redo {session}              → {revision}
```

### 6.2 Semantics

- **Revisions.** Every committed batch increments `revision` by one. A stale `base_revision` fails with
  `409 stale_revision` and the current revision. Optimistic concurrency, no locks.
- **Batches are atomic.** The whole batch is validated against a simulated snapshot before the first
  mutation; it applies fully or not at all.
- **Idempotency.** A repeated key returns the original result; retries after timeouts are safe.
- **Dry run.** Validation plus a structured diff, no commit.
- **Typed errors.** `{code, kind, message, path?, details?}` with stable `kind` strings
  (`unknown_clip`, `overlap`, `unsupported_codec`, `path_denied`, …).
- **Versioning.** Handshake reports `{protocol: "1.0", schema: 7, renderer: "…"}`; unknown fields are
  ignored; unknown methods return `unsupported_method`.

### 6.3 Trust model (v1)

Local, single-user, no authentication: the process runs with the user's privileges. `arcline-cli` takes
`--root` (media allow-list) and `--output-root` (render destinations) and rejects paths outside them before
any syscall. Capability tokens, remote access, and multi-tenant isolation are deliberately deferred; see
[06 §8](06-agent-interface.md).

## 7. Data flow: one edit, end to end

GUI drag of a clip:

```
1. UI thread      pointer drag ends → MoveClip{clip, to_track, to_start}
2. UI thread      sends CommandBatch{base_revision, cmds} to the engine (non-blocking)
3. Engine thread  validates on a simulated snapshot:
                    track kind matches, no overlap after the explicit ripple policy,
                    times are exact rationals on frame boundaries
4. Engine thread  applies → new immutable snapshot (structural sharing)
                  appends to the journal, bumps revision, publishes via arc-swap
5. Render thread  sees the new snapshot; invalidates cache keys for the affected range
6. UI thread      receives ApplyResult; the optimistic ghost is reconciled
7. Autosave task  enqueues a periodic snapshot write
```

The UI moves a **ghost** optimistically during drag; authoritative state always comes from the engine. If
validation fails, the ghost animates back and the typed error is shown. An agent experiences the same flow
without the ghost.

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
| **Operational error** | decoder panic, GPU device lost, disk full | Caught and recovered: restart decoder, recreate device, pause and warn |
| **Defect** | invariant violation | Engine thread catches panics per batch, marks the session degraded, keeps the last good snapshot exportable, writes a crash bundle |

No panic crosses into the UI thread. A decoder panic is converted into a `MediaError` for that clip; other
clips continue to decode.

## 10. Extension points

| Point | Mechanism | Status |
|---|---|---|
| New built-in effect | WGSL pass + JSON manifest registered in `arc-render` | v1 |
| Automation | JSON-RPC / MCP | v1 |
| Third-party effect plugins | OFX / WASM | Post-1.0 ([09](09-effects-and-extensions.md)) |
| Interchange | EDL/FCPXML/OTIO adapters | Post-1.0 |
