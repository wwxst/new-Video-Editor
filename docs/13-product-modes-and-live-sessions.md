# 13 — Product Modes and Live Sessions

Arcline ships in two product modes. Both run the same engine, the same `.arcline` format, and the same
command system; they differ only in who drives the session and where the human watches it. This document is
the **single normative definition** of the modes, local IPC, live attach, control and takeover, state
synchronization, the in-app agent panel, and in-client views. Decision: [ADR-0013](adr/0013-product-modes-and-live-sessions.md).

## 1. The two modes

| | **Standalone Editor** | **Embedded Editor** |
|---|---|---|
| Started by | the user (`arcline.exe`) | an external agent client (Claude Desktop, VS Code, Cursor, …) via MCP |
| Who drives | the human; the built-in **Agent panel** assists | the external agent; the human watches and can take over |
| Where the human sees the edit | the Arcline window | the Arcline window in **companion layout** next to the client, and/or an **in-client view** (MCP Apps) |
| Agent connection | in-process (Agent panel) or attached external client | `arcline-cli mcp --attach` → local IPC → running Arcline |
| Engine, format, commands | shared | shared |

Headless operation (`arcline-cli mcp --project …` with no window, [06 §2](06-agent-interface.md)) remains
available for batch and CI use. It is the same session code without a GUI.

There is no separate "embedded build". Embedded mode is the standalone app with an attached client and a
compact layout.

## 2. Native UI embedding: feasibility (verified 2026-11)

The question: can the Arcline UI run *inside* an external agent client's window?

| Approach | v1 verdict | Evidence and reasons |
|---|---|---|
| Native Arcline window reparented into an agent client (Win32 `SetParent` across processes) | **Not feasible** | The target clients are Electron/web apps that offer no API to host a foreign native window. Cross-process reparenting is undocumented behavior for these hosts: input focus, DPI, z-order, and crash coupling break unpredictably. |
| Native child window created against a parent handle supplied by the host (`winit` `with_parent_window`, raw-window-handle) | **Feasible in principle, no target host offers it** | `winit` documents child-window creation against a raw parent handle (platform list to be confirmed against the current `winit` docs; Wayland is not expected to support it). `eframe` does not expose it, so this path would replace `eframe` with `egui-winit` + `egui-wgpu`. Deferred until a host offers native-handle embedding. |
| **In-client view via MCP Apps** (`io.modelcontextprotocol/ui`, `ui://` resources rendered in a sandboxed iframe) | **Feasible — v1** | MCP Apps is the first official MCP extension, spec stable since 2026-01-26. Hosts listed by the MCP project include Claude / Claude Desktop, VS Code GitHub Copilot, Microsoft 365 Copilot, Goose, and Postman. Support varies by host and must be tested per host. |
| **Companion window** (Arcline window beside the client, live-attached) | **Feasible — v1, primary** | Works with every MCP client; full-fidelity real-time preview and editing. |
| Image-only fallback (`inspect_frame` results in chat) | **Feasible — v1** | Works with every MCP client, including hosts without MCP Apps. |

Conclusion: in v1, "embedded" means a **live-attached companion window** plus an **MCP Apps view** where the
host supports it, with image results as the universal fallback. True native in-window embedding is not
available from any target host and is not built.

The MCP Apps view is a small static HTML/JS page **served by Arcline and executed by the host**. Arcline
itself still contains no web runtime ([01 §1](01-system-architecture.md)).

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

## 8. Embedded: companion layout and in-client view

**Companion layout** (GUI workspace preset, activated by `--launch` or *View → Companion*): compact window
with viewer, timeline, and an **activity strip** showing the attached client, control state, the last
commands applied, and the *Take over* button. Docked beside the agent client by the user (or snapped to the
screen edge on launch).

**In-client view (MCP Apps).** `arcline-cli mcp` declares the `io.modelcontextprotocol/ui` extension when
the host negotiates it and serves one resource, `ui://arcline/workspace`, linked from `project_describe`,
`inspect_frame`, and `render_status` via `_meta.ui.resourceUri`. The view shows:

- current preview frame (JPEG ≤ 960 px, refreshed on `session.changed` and on scrub, ≤ 4 fps) — a monitor,
  not a playback engine;
- timeline overview (tracks and clips from `inspect.timeline`), playhead, selection;
- control state with *Take over* / *Hand back* (calls `control_request` / `control_release`);
- job progress and links to reports.

It fetches data only through the tools above, served as `text/html;profile=mcp-app`, with no external network
requests. Hosts without MCP Apps receive normal text and image tool results.

## 9. Code placement (no new crates)

| Concern | Location |
|---|---|
| Session server, JSON-RPC dispatch, tool registry, MCP server, MCP Apps resource, attach bridge | `arc-cli` (library + `arcline-cli` binary) |
| IPC listener thread, consent prompts, companion layout, activity strip, Agent panel and its Messages API loop | `arc-app` (depends on the `arc-cli` library) |
| Control state and its validation check | `arc-core::command` (pure; the engine checks the batch's `source`) |

`arc-cli`'s library must not depend on `egui` (unchanged rule). The GUI process now hosts the same session
server as the CLI, so behavior is identical whether an agent attaches or runs headless.

## 10. Failure behavior

| Event | Behavior |
|---|---|
| Attached client disconnects | its control (if any) reverts to `shared`; nothing else changes |
| GUI exits or crashes | bridge returns `session_ended` to the agent; recovery on next launch per [08 §4.6](08-performance-reliability.md) |
| Client floods requests | per-client rate limit and batch-size caps ([06 §8](06-agent-interface.md)); excess rejected with `rate_limited` |
| Agent panel network/API failure | panel shows the error; the editor is unaffected; no automatic retries of tool calls |
| Consent denied | client receives `consent_denied`; no further prompts from that client for the session |
