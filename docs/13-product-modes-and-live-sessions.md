# 13 — Product Modes and Live Sessions

Arcline ships in two product modes. Both run the same engine, the same `.arcline` format, and the same
command system; they differ only in who drives the session and where the human edits. This document is the
**single normative definition** of the modes, integration levels, native embedding and the embed channel,
local IPC, live attach, control and takeover, state synchronization, the in-app Agent panel, and in-client
views. Decisions: [ADR-0013](adr/0013-product-modes-and-live-sessions.md),
[ADR-0014](adr/0014-integration-levels-and-native-embedding.md).

## 1. The two modes

| | **Standalone Editor** | **Embedded Editor** |
|---|---|---|
| Started by | the user (`arcline.exe`) | an agent client — ours or a third party's |
| Who drives | the human; the built-in **Agent panel** assists | the client's agent; the human edits alongside it and can take over |
| Where the human edits | the Arcline window | per integration level (§2): the **full editor inside the client window** (L1, L2), or a companion window plus in-client view (L3) |
| Agent connection | in-process (Agent panel) or attached external client | live attach over local IPC (§3–4) |
| Engine, format, commands | shared | shared |

Headless operation (`arcline-cli mcp --project …` with no window, [06 §2](06-agent-interface.md)) remains
available for batch and CI use. There is no separate "embedded build": the same `arcline.exe` runs top-level,
as an embedded child window, or as a companion window.

## 2. Integration levels

**Product goal: the full, interactive Arcline editor inside the agent client** — timeline, viewer, inspector,
audio, every editing tool — not a preview panel. Levels 2 and 3 exist because not every host can contain a
native editor; they do not replace Level 1.

| Level | Host | What the user gets | Mechanism | v1.0 status |
|---|---|---|---|---|
| **L1** | **Our own agent client** | Full editor inside the client window, live-attached to the client's agent | Native child-window embedding (§2.1) + embed channel (§2.2) + built-in host adapter | **v1.0 goal**, gated by the PoC (§2.4) |
| **L2** | Third-party agent clients whose extension API exposes a native window handle | Same full editor as L1 | Same mechanism; a thin host adapter written against the host's extension API (§2.3) | Contract in v1.0; adapters built per host once one is confirmed — **none confirmed today** |
| **L3** | All other MCP clients (Claude Desktop, VS Code, Cursor, …) | Full editor in a companion window beside the client + MCP Apps view inside the client + image results | Live attach (§4) and §8 | v1.0 |

Selection is automatic: a host adapter that sends `embed.attach` gets L1/L2; otherwise the bridge offers the
MCP Apps view if the host negotiates it, and opens the companion window. Any embedding failure at runtime
**degrades to L3 without losing the session** (§2.5).

### 2.1 Mechanism: out-of-process native child window

One mechanism serves L1 and L2:

- The host reserves a region of its window and passes a native handle — `HWND` on Windows — to
  `arcline.exe --embed-channel <pipe>`, then sends `embed.attach` (§2.2).
- Arcline creates its window as a **child** of that handle (`winit` `with_parent_window` with a
  raw-window-handle) and renders the full `egui` + `wgpu` editor into it. Engine, decode, audio, and session
  server run in the Arcline process exactly as in Standalone.
- `arc-app` runs its own event loop on **`egui-winit` + `egui-wgpu`** instead of `eframe`, because `eframe`
  does not expose a parent window. Same stack one level down — a few hundred lines of host-loop code — and the
  only architectural change embedding requires ([ADR-0007](adr/0007-native-wgpu-ui.md)).

Out-of-process because it works with any host technology (native, Electron, Qt, JVM), an editor crash cannot
take down the agent client or the reverse, and the editor's threading and GPU usage stay unchanged.

Windows hazards the PoC must measure (cross-process parent/child windows share input state):

| Hazard | Mitigation |
|---|---|
| A hung process can stall the other's input | Arcline's UI thread never blocks (existing budget); a 2 s watchdog detaches an unresponsive peer (§2.5) |
| Keyboard focus and shortcut ownership | explicit focus handoff over the embed channel; host-level shortcuts forwarded; holding `Esc` returns focus to the host |
| DPI and scaling | both processes per-monitor-DPI-aware v2; DPI changes sent over the channel |
| Swapchain on a child window | flip-model presentation on a child `HWND` verified in the PoC; fallback is a DirectComposition visual |
| IME candidate window placement | verified in the PoC with CJK input |

macOS has no cross-process `NSView` parenting. The macOS approach (for example shared-IOSurface rendering
into a host-side layer) is decided during the v1.1 port; v1.0 is Windows-only ([ADR-0011](adr/0011-windows-first.md)).

### 2.2 Embed channel

A second connection role on the existing named pipe (§3), used only by host adapters, never by agents. NDJSON
messages:

| Direction | Message | Purpose |
|---|---|---|
| host → Arcline | `embed.attach {parent, bounds, dpi, theme?}` | create the child window in the host region |
| host → Arcline | `embed.resize {bounds, dpi}`, `embed.visibility {visible}` | follow the host layout |
| host → Arcline | `embed.focus {grant}`, `embed.shortcut {action}` | hand keyboard focus in; forward host-level commands |
| Arcline → host | `embed.ready`, `embed.focus_release` | ready to show; return focus to the host |
| both | `embed.ping` / `embed.pong` | liveness for the watchdog (1 s interval) |
| either | `embed.detach {reason}` | end embedding; Arcline falls back per §2.5 |

The host's agent uses the same session over MCP/JSON-RPC (§4) as in any other mode; consent, control, and
takeover (§3, §5) are unchanged. While embedded, the built-in Agent panel is hidden by default because the
host provides the conversation. The agent API of [06](06-agent-interface.md) does not change.

### 2.3 Host adapters

A host adapter is the host-side shim: it launches or finds `arcline.exe`, reserves the region, passes the
handle, and speaks the embed channel. The editor contains no host-specific code.

- **L1 (our client):** the adapter is part of our agent client. This repository ships the embed-channel
  specification and a minimal **reference host** (an `arc-app` example binary, Win32) used by the PoC and CI
  and as the reference implementation for the client. Building the agent client itself is outside this
  repository's roadmap.
- **L2 (third parties):** an adapter is written only after a host is confirmed to give extensions a native
  window handle and a stable region. Candidates surveyed in the PoC: JVM-based hosts (heavyweight AWT canvas
  via JAWT) and Electron-based hosts whose plugins can reach the main process
  (`BrowserWindow.getNativeWindowHandle()`). Hosts whose extensions run in an isolated extension host with only
  webviews (VS Code-style) cannot support L2 and stay at L3.
- **Never:** injecting into hosts without an extension path, or reparenting into a host window without the
  host's cooperation.

### 2.4 Embedding PoC (early)

A one-week spike directly after Phase A, before media work, so its result can change the UI plan cheaply
([10](10-roadmap.md)):

- a minimal `egui-winit` + `egui-wgpu` app (dock layout, scrolling timeline canvas, 60 fps animated texture,
  text field with IME) embedded as a child window in (a) the Win32 reference host and (b) a minimal Electron
  host via its native window handle;
- measured: frame pacing and input latency versus top-level, resize and move without artifacts, focus and
  shortcut handoff, IME, DPI change across monitors, hang and crash of either process, flip-model presentation
  on the child window;
- a one-day survey of candidate L2 hosts' extension APIs.

**Go criteria:** frame pacing indistinguishable from top-level; resize without artifacts; correct focus
round-trips and IME; a hang or crash in either process leaves the other usable after the watchdog fires;
presentation works on the child window.

**No-go path:** a second, time-boxed spike for the alternative L1 mechanism — Arcline renders into a shared GPU
texture that the host composites (DXGI shared handle / DirectComposition), with input forwarded over the embed
channel. Full embedding stays the goal either way; only the mechanism changes. The outcome is recorded in
[ADR-0014](adr/0014-integration-levels-and-native-embedding.md).

### 2.5 Runtime fallback

| Trigger | Behavior |
|---|---|
| No host adapter, or `embed.attach` fails | companion window (L3), plus the MCP Apps view if the host supports it |
| Watchdog: no `embed.pong` for 2 s, or the parent window disappears | Arcline destroys the child window and opens a top-level companion window; session, undo history, and agent connection continue (same process) |
| Arcline crashes | the host shows "Editor stopped — Reopen"; recovery per [08 §4.6](08-performance-reliability.md) |
| User chooses *Pop out* | same as the watchdog path, deliberately |

## 3. Local IPC

Transport for JSON-RPC between processes on the same machine. Same NDJSON framing as stdio
([01 §6](01-system-architecture.md)).

| Platform | Endpoint | Access control |
|---|---|---|
| Windows (v1.0) | named pipe `\\.\pipe\arcline-<user-SID>-<session-id>` | DACL: current user only; remote clients rejected (`PIPE_REJECT_REMOTE_CLIENTS`) |
| macOS (v1.1) / Linux (post-1.0) | Unix socket in the per-user runtime directory | mode `0600`, owner check |

**Discovery.** A running GUI writes `%LOCALAPPDATA%\Arcline\sessions\<session-id>.json` (`{pid, project,
endpoint, token, started}`; user-only ACL) and removes it on exit; stale files are detected by PID.

**Authentication and consent.** Clients present the discovery `token` (protects against other users and
accidental connections; same-user processes can read it, which matches the local trust model of
[06 §8](06-agent-interface.md)). The first connection from a client name shows a consent prompt in the GUI —
*Allow "Claude Desktop" to control this project?* — with *Allow once*, *Always allow*, *Deny*. Nothing is
editable by an external client before the human allows it.

## 4. Live attach

```
agent client ──MCP stdio──► arcline-cli mcp --attach [--project P] [--launch]
                                     │  bridge: MCP ⇄ JSON-RPC
                                     ▼
                          named pipe (local IPC)
                                     ▼
                     arcline.exe (GUI) — session server thread
                                     ▼
                     engine thread (single writer, unchanged)
```

- `--attach` connects to the running GUI that has project `P` open (or the only running session). With
  `--launch`, it starts `arcline.exe` on `P` in **companion layout** if none is running.
- Without `--attach`, `arcline-cli mcp` keeps the headless behavior.
- `arcline-cli` in attach mode is a thin bridge: it holds no document. All commands execute in the GUI's
  engine, so the human sees each edit as it lands.
- One GUI session accepts **up to 4 attached clients** (configurable). Each client is identified by
  `client_id` and display name in history, audit, and the activity strip.
- The project lock (`session.lock`, [08 §4.1](08-performance-reliability.md)) is held by the GUI; attached
  clients do not take it. A headless `arcline-cli` on a project that a GUI has open fails with
  `project_locked` and the hint to use `--attach`.

## 5. Control and human takeover

The engine stays the single writer. Control decides **which clients may submit edits**; it is checked by the
engine thread at validation time, so there are no races with in-flight batches.

| Control state | Human (GUI) edits | Agent edits |
|---|---|---|
| `shared` (default when an agent attaches) | allowed | allowed; conflicts resolved by `base_revision` ([01 §6.2](01-system-architecture.md)) |
| `human` (after takeover) | allowed | rejected with `not_in_control` |
| `agent:<client_id>` (granted on request) | allowed — any human edit **automatically takes over** | allowed for that client only |

Rules:

1. **The human can always take control** in one action: the *Take over* button, `Esc` held for 1 s in the
   viewer, or any manual edit while an agent holds control. The human is never blocked.
2. Takeover applies to the next batch the engine validates. A batch already committed stays committed; it
   can be undone like any other.
3. Agents call `control.request` to ask for exclusive control (the GUI shows *Grant* / *Deny*) and
   `control.release` to hand back.
4. Every control change is broadcast as `control.changed` and recorded in `history.list`.
5. Running jobs (render, interchange export) are not affected by control changes; they work from snapshots.
6. The built-in Agent panel (§7) is a client like any other and follows the same rules.

## 6. State synchronization

Notifications pushed by the session server to every attached client (JSON-RPC notifications; no new
request methods):

| Notification | Payload | When |
|---|---|---|
| `session.changed` | `revision`, `source` (`human` / `client_id` / `undo`), compact diff (changed clip/track IDs and kinds) | after every committed batch |
| `control.changed` | new state, `by` | on takeover, grant, release |
| `selection.changed` | selected clip IDs, playhead time | debounced to ≤ 5/s |
| `job.progress` | `job_id`, progress | ≤ 2/s per job |

- Clients re-read details with `project.describe` or `inspect.timeline` (range-limited) when they need them.
- MCP bridging: `session.changed` becomes `notifications/resources/updated` for
  `arcline://project/timeline.txt`; other notifications are surfaced through the MCP Apps view (§8) and the
  next tool result's `state` field (`revision`, `control`, `playhead`).
- Selection and playhead let a human point at something ("this clip") and the agent see it.

## 7. Standalone: the Agent panel

A docked panel in the Arcline window where the user chats with an agent that edits the open project.

**Architecture.** The panel runs an agent loop on a worker thread in `arc-app`. Its tools are the same tool
registry the MCP server uses (`arc-cli` library, §9), called through the in-process transport, so the panel
has exactly the agent's capabilities — no more.

**Model access.** Anthropic Messages API, called over HTTPS directly (there is no official Rust SDK), from a
blocking client on the worker thread (no async runtime). Defaults:

- model `claude-opus-5-5`, user-configurable; adaptive thinking; streaming with eager tool-input streaming,
  and every tool input validated against its schema before execution;
- server-side refusal fallback enabled (`fallbacks: "default"`, beta `server-side-fallback-2026-07-01`);
  `stop_reason` checked before tool execution (`refusal`, `max_tokens`);
- prompt caching with a stable tool list and system prompt first;
- `tool_choice: auto` (forced tool choice is not supported by the default model).

The loop runs locally because every tool acts on local process state and local files; a hosted agent runtime
would add a remote sandbox the editor does not use.

**Credentials and privacy.** The API key is stored in the OS credential store (Windows Credential Manager),
never in project files, journals, logs, or crash bundles. The panel is **off until the user configures it**;
it is Arcline's only feature that sends data to a network service. What is sent: the conversation, tool
results (project summaries, timeline text), and — only when the agent calls `inspect_frame` — still frames.
Media files are never uploaded. This is disclosed on first use ([11 §4](11-risks-and-legal.md)).

**Approvals.** Each tool call is shown in the panel. Edits apply immediately (all are undoable); renders,
interchange exports, and saves to new paths ask for confirmation unless the user enables auto-approve.
Timeline text, file names, and subtitle text in tool results are treated as data, never as instructions.

**External agents instead of the panel.** Users who prefer their own agent client attach it (§4); the panel
and attached clients can coexist under the control rules of §5.

## 8. Level 3: companion window and in-client view

**Companion window** (workspace preset `Companion`, opened by `--launch`, by fallback, or *View → Companion*):
the **full editor** in its own window beside the agent client, with a compact starting layout (viewer,
timeline, and an **activity strip** showing the attached client, control state, the last commands applied,
and *Take over*). Only the starting layout differs from Standalone; every editing tool is available.

**In-client view (MCP Apps)** — a monitor and remote control for hosts that cannot embed the editor, not a
substitute for it. `arcline-cli mcp` declares the `io.modelcontextprotocol/ui` extension when the host
negotiates it and serves one resource, `ui://arcline/workspace`, linked from `project_describe`,
`inspect_frame`, and `render_status` via `_meta.ui.resourceUri`. MCP Apps is the first official MCP extension
(stable since 2026-01-26); hosts listed by the MCP project include Claude / Claude Desktop, VS Code GitHub
Copilot, Microsoft 365 Copilot, Goose, and Postman; support varies and is tested per host. The view shows:

- current preview frame (JPEG ≤ 960 px, refreshed on `session.changed` and on scrub, ≤ 4 fps) — a monitor,
  not a playback engine;
- timeline overview (tracks and clips from `inspect.timeline`), playhead, selection;
- control state with *Take over* / *Hand back* (calls `control_request` / `control_release`), and *Open
  editor* (brings the companion window forward);
- job progress and links to reports.

It fetches data only through the tools above, served as `text/html;profile=mcp-app`, with no external network
requests. Hosts without MCP Apps receive normal text and image tool results. The view is HTML executed by the
host; Arcline itself contains no web runtime.

## 9. Code placement (no new crates)

| Concern | Location |
|---|---|
| Session server, JSON-RPC dispatch, tool registry, MCP server, MCP Apps resource, attach bridge | `arc-cli` (library + `arcline-cli` binary) |
| IPC listener thread, embed channel, top-level/child-window host loop (`egui-winit` + `egui-wgpu`), consent prompts, companion layout, activity strip, Agent panel and its Messages API loop | `arc-app` (depends on the `arc-cli` library) |
| Reference host for the PoC, CI, and client teams | `arc-app` example binary (`examples/embed_host`, Win32) |
| Control state and its validation check | `arc-core::command` (pure; the engine checks the batch's `source`) |

`arc-cli`'s library must not depend on `egui` (unchanged rule). The GUI process now hosts the same session
server as the CLI, so behavior is identical whether an agent attaches or runs headless.

## 10. Failure behavior

| Event | Behavior |
|---|---|
| Attached client disconnects | its control (if any) reverts to `shared`; nothing else changes |
| GUI exits or crashes | bridge returns `session_ended` to the agent; recovery on next launch per [08 §4.6](08-performance-reliability.md) |
| Embedding fails or the embed peer stops responding | fall back to the companion window per §2.5; the session continues |
| Client floods requests | per-client rate limit and batch-size caps ([06 §8](06-agent-interface.md)); excess rejected with `rate_limited` |
| Agent panel network/API failure | panel shows the error; the editor is unaffected; no automatic retries of tool calls |
| Consent denied | client receives `consent_denied`; no further prompts from that client for the session |
