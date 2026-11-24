# ADR-0014 — Three integration levels; full native editor embedding as the goal

**Status:** Accepted, 2026-11-24. Supersedes in part [ADR-0013](0013-product-modes-and-live-sessions.md)
(decision 5: "no native in-window embedding"). Mechanism confirmation pending the embedding PoC
([13 §2.4](../13-product-modes-and-live-sessions.md)).

## Context

ADR-0013 limited Embedded mode to a companion window plus an MCP Apps view, because current third-party agent
hosts do not offer a way to host a native window. That reasoning holds for third-party hosts, but it does not
apply to our own agent client, which can provide a native parent window. The product goal is the **full,
interactive editor inside the agent client**; a preview panel is not an acceptable substitute.

## Decision

1. Define three integration levels ([13 §2](../13-product-modes-and-live-sessions.md)):
   - **L1** — our own agent client: full editor embedded natively (v1.0 goal).
   - **L2** — third-party clients whose extension API exposes a native window handle: same embedding through a
     thin host adapter, built per host once confirmed.
   - **L3** — all other clients: companion window (full editor) + MCP Apps view + image results.
2. One mechanism for L1 and L2: **out-of-process native child window** — `arcline.exe` creates its window as a
   child of a host-supplied handle and renders the full `egui` + `wgpu` editor; a small **embed channel** on the
   existing named pipe handles bounds, visibility, DPI, focus, and liveness.
3. `arc-app` runs on **`egui-winit` + `egui-wgpu`** directly instead of `eframe`, so the same app runs
   top-level or as a child window. Rust, `wgpu`, `egui`, the six crates, and the shared core are unchanged.
4. Runtime failures degrade to L3 without losing the session.
5. Run a **one-week embedding PoC directly after Phase A**. If it fails its go criteria, a time-boxed second
   spike evaluates shared-texture composition (DXGI shared handle / DirectComposition) as the L1 mechanism. The
   goal is kept either way.
6. This repository ships the embed-channel specification and a Win32 reference host; building our agent
   client is a separate effort.

## Consequences

- The editor contains no host-specific code; every host integration is a thin adapter.
- Out-of-process embedding isolates crashes and works with any host technology, at the cost of cross-process
  window hazards on Windows (shared input state, focus, DPI) that the PoC must measure.
- macOS has no cross-process `NSView` parenting; the v1.1 port must choose a different mechanism.
- Dropping `eframe` adds a small host-loop module that `eframe` previously provided.
- Roadmap grows by about 3 weeks (PoC + production embedding).

## Alternatives considered

- **Companion window and MCP Apps only** (ADR-0013): fails the product goal for our own client.
- **In-process embedding (editor as a library inside the client)**: ties the client to Rust/egui, shares a
  process and GPU device with the client, and loses crash isolation; rejected.
- **Web UI for embedding**: contradicts the native architecture and duplicates the editor.
- **Shared-texture composition as the primary mechanism**: no window-hierarchy hazards, but requires the host to
  composite GPU textures and forward all input; kept as the no-go fallback.
- **Reparenting into hosts without cooperation**: unsupportable; rejected.
