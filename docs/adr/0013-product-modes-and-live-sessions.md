# ADR-0013 — Two product modes and live GUI sessions in v1

**Status:** Accepted, 2026-11-24. Supersedes in part [ADR-0006](0006-embedded-core-and-daemon.md) (live GUI
attach and named-pipe transport were deferred there).

## Context

Arcline must work both as a standalone editor with an integrated agent and as an editor driven from external
agent clients, with real-time preview, state synchronization, and human takeover. The v2 architecture
deferred live GUI attach; agents could only work headless on project files, so a human could not watch or
take over an agent's edit in progress.

Feasibility research (2026-11) found:

- Target agent clients (Claude Desktop, VS Code, Cursor) are Electron/web apps with no API for hosting a
  foreign native window; cross-process reparenting is not a supportable path.
- `winit` can create a child window against a host-supplied parent handle, but no target host supplies one,
  and `eframe` does not expose it.
- MCP Apps (official MCP extension, stable 2026-01-26) lets a server render a sandboxed HTML view inside
  supporting hosts (Claude Desktop, VS Code GitHub Copilot, and others).

## Decision

1. Ship two product modes in v1.0, **Standalone** and **Embedded**, on one engine, one format, one command
   system ([13](../13-product-modes-and-live-sessions.md)).
2. Bring **live GUI attach** into v1: the GUI hosts the same session server as `arcline-cli`, reachable over
   a per-user **named pipe** (Unix socket on later platforms); `arcline-cli mcp --attach` bridges MCP to it.
3. Add **control and takeover** (`shared` / `human` / `agent:<id>`), checked by the engine thread; the human
   can always take over in one action.
4. Add **state synchronization** notifications (`session.changed`, `control.changed`, `selection.changed`,
   `job.progress`).
5. "Embedded" in v1 = **companion window** (primary) + **MCP Apps view** where supported + image results as
   fallback. No native in-window embedding.
6. Add an **Agent panel** to the standalone GUI: a local agent loop on the Anthropic Messages API using the
   same tool registry as the MCP server.
7. No new crates: the `arc-cli` library holds session/protocol/tools; `arc-app` depends on it.
8. No plugins for third-party video editors.

## Consequences

- Agents and humans can co-edit one live project; every edit is attributed and undoable.
- The trust model grows from "a process the user launched" to "a local process that connects to the user's
  open session"; mitigated by user-only pipe ACLs, a discovery token, and a consent prompt per client.
- The Agent panel is Arcline's first network feature; it is off until configured, keys live in the OS
  credential store, and frames are sent only when the agent requests them.
- The MCP Apps view is HTML executed by the host; Arcline itself still has no web runtime.
- Roughly 6 weeks of additional work (Phase F grows; new Phase J). See [10](../10-roadmap.md).

## Alternatives considered

- **Native window embedding into agent clients**: not supported by the hosts; rejected for v1. Revisit if a
  host offers native-handle embedding (would require moving from `eframe` to `egui-winit` + `egui-wgpu`).
- **WebView-based Arcline UI to embed in hosts**: contradicts the native architecture (ADR-0007) and would
  duplicate the UI.
- **Agent panel via an external agent only (no built-in loop)**: simpler, but leaves standalone users without
  an integrated assistant; external clients remain supported through attach.
- **Hosted agent runtime for the panel**: the tools act on local process state and files; a remote sandbox
  adds cost and latency without benefit.
- **Exclusive locking instead of `shared` control**: safer but makes the human wait on the agent; the
  default favors co-editing with instant takeover.
