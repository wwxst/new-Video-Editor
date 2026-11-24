# ADR-0001 — Rust for core and UI

**Status:** Accepted · 2026-11-24

## Context

The product is a long-lived, performance-critical, cross-platform native application that must call into
C libraries (FFmpeg, OFX plugins, platform audio APIs), run real-time audio, and remain stable while
processing untrusted files. It must be maintainable by a small team for a decade. Memory-safety defects in
an editor mean lost work, so the cost of a use-after-free is not a crash — it is a corrupted project.

## Decision

Use **Rust (2024 edition)** for every component we author: core, media, renderer, audio, app shell, and
CLI. The only non-Rust code is third-party C (FFmpeg) accessed through an owned, narrow FFI layer.

## Consequences

- Memory safety and data-race freedom in the hot paths without a GC: no frame-time spikes from collection,
  and audio callbacks can be allocation-free by construction.
- One language across engine and UI removes an IPC or binding layer from the critical path.
- Strong type system makes the domain model (rational time, IDs, units) enforceable rather than documented.
- Ecosystem is strong for all our needs: `wgpu`, `winit`, `tokio`, `serde`, `rapier`-class numerics.
- Costs: compile times (mitigated with workspace splitting, `sccache`, `thin` LTO in dev), a smaller pool of
  engineers with video-domain experience, and the need to own FFmpeg bindings rather than rely on thin crates.
- Long-term: a single-language codebase is easier to audit and refactor than a C++/Rust/JS mix; the
  investment in Rust expertise compounds.

## Alternatives considered

- **C++**: mature video ecosystem and Qt, but memory-safety burden on a small team, painful dependency
  management, and UB risk in exactly the code (time math, decode buffers) where bugs are silent.
- **C# / .NET (Avalonia)**: excellent tooling and UI story, but GC pauses in audio/video paths, heavier
  native interop, and startup cost.
- **Go**: simple concurrency, but GC, weak GPU story, and no zero-cost abstractions for per-frame work.
- **Swift**: excellent on Apple platforms only; violates cross-platform requirement.
