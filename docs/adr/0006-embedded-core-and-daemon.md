# ADR-0006 — Embedded core + optional daemon, one protocol

**Status:** Accepted · 2026-11-24

## Context

A desktop editor wants the engine in-process for latency and simplicity. Agents want a headless, scriptable
service. A human editing in the GUI may want an agent to assist in the *same live session*. Supporting each
with a different code path guarantees divergence.

## Decision

Build the engine as a **library**. The desktop app embeds it; a separate `arcline serve` binary embeds the
same library. Both speak the **same JSON-RPC protocol**; the desktop app exposes a local control endpoint
(named pipe / unix socket) so agents can attach to a live session. The MCP server is a facade over the
protocol, not a separate implementation.

## Consequences

- One behavioral definition of "edit"; tests run against the protocol, so GUI and agent paths are covered by
  the same conformance suite.
- A live GUI session becomes agent-automatable without screen scraping — the central product promise.
- Cost: the protocol must be complete enough for the GUI, which means avoiding tempting shortcuts where the
  UI calls internal APIs directly. This is enforced by lint/architecture rules (UI crate cannot depend on
  engine internals beyond session handles).
- In-process GUI calls still take the in-memory transport path; the overhead is a serialization-free direct
  dispatch when both ends are in the same process (the wire format stays the same, the transport is a
  function call).
- The daemon gains job persistence, multi-session handling, and capability enforcement that the embedded
  mode inherits from the same code.

## Alternatives considered

- **Always out-of-process core**: cleanest isolation and simplest license boundary, but adds IPC latency to
  every UI interaction (hover, drag) and complicates the renderer's access to UI-owned GPU surfaces.
- **Library only, no daemon**: fails the headless/agent requirement.
- **Separate server implementation sharing the model crate**: two protocol implementations drift; rejected.
