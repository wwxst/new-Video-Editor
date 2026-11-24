# 01 — System Architecture

This document is the spine of the proposal. Everything else elaborates one of its sections.

## 1. Design principles

1. **One core, many clients.** The editor engine is a library (`arcline-core` facade over typed crates).
   The GUI, the CLI, the daemon, and the MCP server are all clients of the same command protocol.
2. **Commands are the only way to change state.** No component mutates the project directly. A GUI drag
   is a `MoveClip { .. }` command; an agent call is the same command. This single rule buys undo, audit,
   crash recovery, dry-run, and collaboration.
3. **Immutable snapshots, single writer.** The document is published as an immutable `Arc<Project>`.
   Readers (renderer, UI, agents) never lock. A dedicated engine thread is the only writer.
4. **Determinism is a product feature.** Given (project, commands, renderer version) the output is
   characterized and reproducible; the contract and its exact bounds are in [03 §8](03-render-engine.md).
5. **Native, no web runtime.** No Chromium, no WebView, no JS engine in the process. UI is drawn by our
   own retained widget layer onto `wgpu`.
6. **Failure isolation by construction.** Media decoding runs in a separate process from the UI from day
   one. A malformed file crashes the decoder, not the session.
7. **Performance is budgeted, not hoped for.** Every subsystem has numbers with CI gates ([08](08-performance-reliability.md)).

## 2. Platform matrix

| Concern | Windows | macOS | Linux |
|---|---|---|---|
| Graphics API | D3D12 (Vulkan fallback) | Metal | Vulkan 1.2 (software fallback: lavapipe) |
| GPU abstraction | `wgpu` | `wgpu` | `wgpu` |
| Hardware decode | D3D11VA / D3D12VA via FFmpeg hwaccel | VideoToolbox | VAAPI / NVDEC |
| Hardware encode | NVENC / QSV / AMF | VideoToolbox | NVENC / VAAPI |
| Audio device | WASAPI (shared + exclusive) | CoreAudio | PipeWire → ALSA |
| Windowing | `winit` | `winit` | `winit` (Wayland + X11) |
| File watching | `ReadDirectoryChangesW` | FSEvents | inotify |
| Crash reporting | minidump | minidump | minidump + coredump |
| Primary channel | MSIX + portable zip | DMG, notarized, universal (arm64+x86_64) | Flatpak, AppImage, tarball |

Minimum targets: Windows 10 21H2, macOS 13, Ubuntu 22.04. Apple silicon is first-class (universal binary),
x86_64 macOS remains supported through the transition period.

## 3. Deployment topologies

The same engine binary supports three shapes:

```
 A. Embedded (desktop default)
 ┌──────────────────────────────┐
 │ arcline-app                  │
 │  ├── UI thread (winit+wgpu)  │
 │  ├── engine thread           │
 │  ├── render threads          │
 │  ├── audio thread            │
 │  └── decode subprocess pool  │
 └──────────────────────────────┘

 B. Daemon (agents, automation)
 ┌──────────────┐   JSON-RPC over    ┌─────────────────────────────┐
 │ agent / MCP  │◄── stdio / WS / ──►│ arcline serve --root ~/media │
 │ client       │    named pipe      │  sessions, jobs, watch      │
 └──────────────┘                    └─────────────────────────────┘

 C. Desktop + local control (human editing, agent assisting)
 ┌────────────────┐  named pipe   ┌──────────────────────────────┐
 │ arcline-app    │◄─────────────►│ listens on a per-user socket │
 │ (live session) │  JSON-RPC     │ bound to session capabilities│
 └────────────────┘               └──────────────────────────────┘
```

Topology C is why the protocol must exist even though the GUI is in-process: it is the only supported way
for an agent to act on a session a human currently has open. No screen scraping, ever.

The **decode subprocess** deserves emphasis. `arc-media` runs demuxing/decoding in a child process
(`arcline-decode`) speaking a small binary pipe protocol (shared-memory frame buffers + control messages).
Process-per-source-group, restarted on crash, capped by a memory budget. This isolates FFmpeg (a large C
codebase that has historically had memory-safety incidents) and guarantees the editor survives a hostile
file. Cost: one extra copy on the GPU upload path, mitigated by shared memory (`CreateFileMapping` /
`shm_open` / `mmap`). See [04 §3](04-media-pipeline.md).

## 4. Crate graph and layering

```
                     ┌─────────────┐   ┌─────────────┐   ┌─────────────┐
   clients           │  arc-app    │   │  arc-cli    │   │ arc-server  │
                     └──────┬──────┘   └──────┬──────┘   └──────┬──────┘
                            │                 │                 │
                     ┌──────┴─────────────────┴─────────────────┴──────┐
   interface layer   │       arc-protocol  (rpc types, capabilities)   │
                     └──────┬─────────────────┬─────────────────┬──────┘
                            │                 │                 │
                     ┌──────┴──────┐   ┌──────┴──────┐   ┌──────┴──────┐
   engine layer      │  arc-ui     │   │ arc-render  │   │arc-plugin-  │
                     │  (widgets)  │   │ arc-audio   │   │   host      │
                     └──────┬──────┘   └──────┬──────┘   └──────┬──────┘
                            │                 │                 │
                     ┌──────┴─────────────────┴─────────────────┴──────┐
   core layer        │   arc-command → arc-model → arc-time           │
                     │   arc-media, arc-cache, arc-encode, arc-text,  │
                     │   arc-effects                                  │
                     └───────────────────────┬───────────────────────┘
                                             │
                     ┌───────────────────────┴───────────────────────┐
   platform layer    │  arc-platform (winit/wgpu/audio/fs/hwaccel shims) │
                     └───────────────────────────────────────────────┘
```

Dependency rules, enforced by a CI check on `cargo metadata` (no exceptions, no "just this once"):

- `arc-time` depends on nothing internal.
- `arc-model` may depend only on `arc-time`.
- `arc-command` may depend on `arc-model`, `arc-time`. It must **not** depend on render, audio, media, or
  UI. Commands are pure data + pure validation.
- `arc-render`, `arc-audio`, `arc-media`, `arc-encode`, `arc-text`, `arc-effects` depend on model/time,
  never on each other (effects depend on render's shader ABI only).
- `arc-ui` never calls decoders or encoders directly; it talks to `arc-render`/`arc-audio` handles.
- Only `arc-app`, `arc-cli`, `arc-server` may construct a "session" (the thing that owns threads).

Rationale: this graph makes the core testable headlessly (no GPU required for model/command tests) and
makes the agent server provably free of UI entanglement.

## 5. Runtime: threads and ownership

```
 ┌──────────────────────────── UI THREAD ─────────────────────────────┐
 │ winit event loop · input dispatch · widget layout · submit to GPU  │
 │ owns: UiState, WidgetTree, RenderTarget. Never blocks > 2 ms.      │
 └───┬───────────────────────────────────────────────────────────┬────┘
     │ Command batch (mpsc, non-blocking)                        │ Frame request
     ▼                                                           ▼
 ┌────────────── ENGINE THREAD ──────────────┐      ┌──────── RENDER THREAD ────────┐
 │ single writer · validate · apply · publish│      │ builds render graph per frame │
 │ Arc<Project> via arc-swap · journal · undo│─────►│ GPU submit · present · cache  │
 └───┬───────────────────────────▲───────────┘      └───────────────┬───────────────┘
     │ commands                  │ snapshots                        │ frame requests
     ▼                           │                                  ▼
 ┌──────────── SESSION/AGENT ────┴───────┐          ┌──────── RENDER WORKERS (N) ───┐
 │ JSON-RPC dispatch, MCP adapter,       │          │ offscreen render, effect pre- │
 │ auth, rate limits, job queue          │          │ pass, thumbnail generation    │
 └───────────────────────────────────────┘          └───────────────────────────────┘

 ┌──────────── AUDIO THREAD (real-time, no alloc, no lock) ───────────┐
 │ pull model: ring buffer ← offline mixed blocks from audio workers  │
 └────────────────────────────────────────────────────────────────────┘

 ┌──────────── TOKIO RUNTIME (I/O) ───────────────────────────────────┐
 │ file watching, proxying, encoder subprocesses, network transports  │
 └────────────────────────────────────────────────────────────────────┘
```

Rules that are architectural, not stylistic:

- **The audio thread allocates nothing and takes no locks.** It reads from a lock-free SPSC ring
  pre-filled by a mixing worker. Any violation fails a debug assertion in CI's soak test.
- **The engine thread never blocks on I/O.** Persistence is a command journal appended by a writer task;
  the engine hands off bytes and moves on.
- **The UI thread never waits on decode.** Missing frames resolve to the nearest cached frame plus a
  "stale" indicator; never a stall.
- **Snapshots are cheap.** `Project` uses persistent data structures (`im::Vector`, `im::HashMap`) so a
  1,000-clip edit produces an O(log n) new root, not a full copy. Measured target: ≤ 200 µs per applied
  command batch at 1,000 clips ([08 §2](08-performance-reliability.md)).

## 6. The protocol

One protocol everywhere. JSON-RPC 2.0 over newline-delimited framing, with three transports:

| Transport | Use | Notes |
|---|---|---|
| stdio | `arcline mcp` spawned by an agent host | Simplest; no ports, no firewall |
| named pipe (Win) / unix socket (mac/Linux) | GUI ⇄ local agents | Per-user ACL; path in a discovery file |
| WebSocket (localhost by default) | Debugging, remote LAN use, browser-based control surfaces | Off unless `--listen` given; TLS required for non-loopback |

### 6.1 Envelope

```jsonc
// request
{
  "jsonrpc": "2.0",
  "id": "7f3a",
  "method": "project.apply",
  "params": {
    "session": "s_01J8…",
    "base_revision": 412,
    "dry_run": false,
    "idempotency_key": "agent-a-step-0042",
    "commands": [
      { "op": "timeline.trim_clip", "clip": "cl_01J8…", "edge": "out",
        "to": { "num": 185, "den": 30 } }
    ]
  }
}
// response
{
  "jsonrpc": "2.0",
  "id": "7f3a",
  "result": {
    "revision": 413,
    "applied": 1,
    "resulting": [{ "op": "timeline.trim_clip", "clip": "cl_01J8…", "duration": "6.00s" }],
    "warnings": []
  }
}
```

### 6.2 Semantics

- **Revisions.** Every committed command batch increments `revision` by exactly one. Clients send
  `base_revision`; mismatch → error `409 stale_revision` with the current revision and a structured diff
  of what changed. This gives agents optimistic concurrency without locks.
- **Batching is atomic.** A batch either fully applies or applies nothing. Validation runs on the whole
  batch against a simulated snapshot before the first mutation.
- **Idempotency.** A repeated `idempotency_key` within a session returns the original result. Agents that
  retry after a timeout do not double-apply.
- **Dry run.** `dry_run: true` validates and returns the diff without committing — how agents "look before
  they leap."
- **Typed errors.** `{code, kind, message, path?, hint?, details?}` where `kind` is a stable machine string
  (`unknown_clip`, `overlap`, `unsupported_codec`, `capability_denied`, …). Agents branch on `kind`, not on
  prose.
- **Protocol version** is in the handshake (`session.open` → `{protocol: "1.0", schema: 7, renderer: "…"}`)
  and every message is forward-compatible by ignoring unknown fields.

### 6.3 Capabilities

A session is opened with a capability token granting a set of scopes:

```
project:read  project:write  media:import  render:submit  plugin:load  fs:root=/home/x/media
```

The daemon takes `--root` allow-lists; any path outside them is rejected before syscalls. `plugin:load` is
denied by default for agent-opened sessions. Tokens are random 256-bit values, stored hashed, and expire
(default 24 h). Full model in [06 §8](06-agent-interface.md).

## 7. Data flow: one edit, end to end

A drag of a clip in the GUI:

```
1. UI thread      pointer drag ends → builds MoveClip{clip, to_track, to_start}
2. UI thread      sends CommandBatch{base_revision, cmds} to engine thread (non-blocking)
3. Engine thread  validate on simulated snapshot:
                    - destination track type matches clip kind
                    - no overlap after resolved ripple/snap policy
                    - time values are exact rationals on frame boundaries
4. Engine thread  apply → new immutable Project root (structural sharing)
                  append to journal (durable, fsync policy §08)
                  bump revision → publish snapshot via arc-swap
5. Render thread  sees new snapshot; invalidates cache keys for affected time range
6. UI thread      receives ApplyResult; refreshes the clip's visual state
                  (UI had already moved the ghost during the drag — no round-trip latency)
7. Autosave task  enqueues periodic snapshot write
```

The UI mutates its own *ghost* state optimistically during interaction; the authoritative state always
arrives from the engine. If validation fails, the UI animates the clip back and shows the typed error. The
same flow, minus step 6's ghost, is what an agent experiences.

## 8. Startup and project open

Startup budget: **cold start to usable window ≤ 1.5 s**, open a 1,000-clip project ≤ 2 s.

```
main → load user config → init logging/crash handler → create wgpu instance/adapter
     → create window (shown with last layout skeleton immediately) → spawn engine
     → restore workspace layout → if palette argument, open project:
          read container manifest (JSON, ≤ 2 MB) → validate schema version
          → migrate if older (offline, no UI block) → resolve asset refs (may be missing → offline refs)
          → first snapshot published → renderer warm-up (compile shaders from cache)
```

Shader compilation is cached to disk keyed by `(driver id, pipeline hash)`; first run pays it, later runs
do not. Project open never blocks on media probing: probe results are cached in the container and refreshed
asynchronously.

## 9. Error model

Three layers, each with a different owner:

| Layer | Example | Handling |
|---|---|---|
| **Domain error** (expected) | overlap, unknown clip, unsupported codec | Typed `CommandError`, surfaced to client, no log noise |
| **Operational error** (unexpected but recoverable) | decoder died, GPU device lost, disk full | Recover: restart decoder / recreate device / pause and warn |
| **Defect** (bug) | invariant violation, panic in engine | Catch at thread boundary, quarantine session state, offer journal recovery, emit minidump |

The engine thread catches panics via `catch_unwind` around each batch and, on catch, marks the session
`degraded`: further commands rejected, project exportable from last good snapshot, crash bundle written.
No panic may cross into the UI thread.

## 10. Extension points (summary)

| Point | Mechanism | Doc |
|---|---|---|
| New effect | WGSL pass + JSON param schema, registered in a manifest | [03 §5](03-render-engine.md) |
| Third-party effect (native) | OFX bridge (P5) | [09](09-plugin-system.md) |
| Third-party effect (sandboxed) | WASM component with declared buffer access | [09](09-plugin-system.md) |
| Automation | JSON-RPC / MCP tools | [06](06-agent-interface.md) |
| Interchange | EDL, FCPXML, OTIO import/export | [04 §8](04-media-pipeline.md) |
```
