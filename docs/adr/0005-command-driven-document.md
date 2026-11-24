# ADR-0005 — Command-driven document, single writer

**Status:** Accepted · 2026-11-24

## Context

The editor needs undo/redo, crash recovery, agent control, auditability, and eventually collaboration. Each
of these is usually built separately: an observer-based undo stack, a separate autosave system, a scripting
API that mutates state directly, and a diff/merge layer. The result is four sources of truth that drift.

## Decision

Every state change is a **typed, serializable command** applied by a **single-writer engine** to an
**immutable document snapshot**, which is published to readers atomically. There is no other mutation path.
The inverse of each command is derived during apply and stored for undo and journal replay.

## Consequences

- Undo, journal, crash recovery, audit log, dry-run, and the agent API are all consequences of one mechanism.
- Readers (renderer, UI, agents) never lock and never see partial state: they hold an `Arc<Project>`.
- Validation is centralized and testable without a GPU or a window.
- Every feature must be expressible as a command. This is a real cost: some UI affordances (e.g., a
  transient drag preview) need client-side ghost state, and some bulk operations need well-designed batch
  commands (e.g., `trim_clip { ripple: "all" }`) rather than loops of small ones.
- Apply performance becomes a first-class concern; persistent data structures keep it O(log n) and
  benchmarks gate it ([08 §2](08-performance-reliability.md)).
- Collaboration becomes a tractable problem later: command logs merge, state does not.

## Alternatives considered

- **Mutable model + observers**: fast to start, but undeclared mutation creeps in, undo requires inverse
  tracking anyway, and agent access would need a parallel safe layer.
- **Native CRDT document**: attractive for real-time multi-user, but complicates validation (operations
  commute by construction, which fights invariants like "clips never overlap") and is premature for 1.0.
- **Event sourcing without typed commands** (log of opaque events): loses validation and schema generation;
  the command type *is* the schema.
- **Script/eval interface** (Lua/JS bindings straight into the model): a security and correctness liability;
  rejected in favor of explicit commands.
