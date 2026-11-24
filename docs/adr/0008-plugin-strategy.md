# ADR-0008 — Built-in effects now; plugin tiers deferred

**Status:** Accepted, revised 2026-11-24 after architecture review.

## Context

Professional users eventually expect third-party effects (OFX), agent-authored effects (WASM), and VST3
audio plugins. The original proposal planned all three for v1. Each requires a host ABI, sandboxing,
parameter bridging, distribution, and a compatibility matrix — months of work with no v1 user.

## Decision

Ship **built-in effects only** in v1. Keep a versioned WGSL shader ABI and JSON effect manifest that are
designed to be targeted by future plugin tiers. Defer OFX, WASM, and VST3 to post-1.0.

## Consequences

- Removing the tiers deletes two sandboxes, a second process model, an ABI compatibility matrix, and a
  substantial amount of code — while leaving an extension seam.
- Every v1 effect is reviewed, tested, and deterministic; no compatibility surprises.
- Adding a built-in effect is a small, local change (shader + manifest + fixture).
- Agents can still *use* every effect through manifests; they cannot author and install new native code,
  which is the correct trust boundary for v1.
- Future tiers remain additive: OFX via a subprocess render RPC, WASM via a component-model host, VST3 via
  an audio subprocess with latency compensation ([09 §3](../09-effects-and-extensions.md)).

## Alternatives considered

- **Three tiers in v1** (original decision): ecosystem and agent-authoring value, but weeks of work before
  the editor itself exists; rejected for a solo project.
- **OFX only**: ecosystem, no sandbox, native crash risk.
- **WASM only**: safe, but no ecosystem and a performance ceiling for finishing effects.
- **Proprietary C ABI**: full control, no ecosystem.
