# ADR-0006 — Embedded core + headless CLI, one protocol

**Status:** Accepted, revised 2026-11-24 after architecture review.

## Context

The desktop app wants the engine in-process for latency and simplicity. Agents want a headless, scriptable
surface. Supporting both with different code paths guarantees divergence. The original proposal also
included a live-attach daemon and multiple transports for v1.

## Decision

Build the engine as a library. `arc-app` embeds it (`arcline`); `arc-cli` embeds the same library for
headless rendering and the MCP server (`arcline-cli`). Both speak the **same JSON-RPC protocol**. In v1 the
transport is **stdio only**; there is no daemon and no live GUI attach.

## Consequences

- One behavioral definition of "edit"; the GUI and agent paths share the command engine and the offscreen
  renderer, so preview and export cannot diverge.
- Agents work on project files in a dedicated process — a clean trust boundary for a local tool.
- The protocol must be complete enough for the CLI; the GUI is not exempt from it. This is enforced by the
  crate rule that only `arc-app`/`arc-cli` construct sessions.
- Deferred but additive: named-pipe transport, live GUI attach, remote WebSocket, capability tokens,
  multi-session daemon. None changes command semantics ([06 §9](../06-agent-interface.md)).
- A GUI-only direct call path is avoided; the cost (serialization on in-process boundaries) is negligible
  at command granularity.

## Alternatives considered

- **Always out-of-process core**: clean isolation, but adds IPC to every UI interaction and complicates
  GPU-surface sharing.
- **Live-attach daemon in v1**: an IPC surface and trust model with no demonstrated user; deferred.
- **Separate server implementation**: two protocol implementations drift; rejected.
