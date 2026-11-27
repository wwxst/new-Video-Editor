# Architecture Decision Records

ADRs record load-bearing decisions with their alternatives. This set was revised on **2026-11-24** after
the [architecture review](../../reports/architecture-review.md) reframed the project for solo development;
affected records carry a revision note. From this point on, decisions are superseded rather than rewritten:
a new ADR references the old one, and the old file's status changes to `Superseded by ADR-XXXX`.

| ADR | Title | Status |
|---|---|---|
| [0001](0001-rust-for-core-and-ui.md) | Rust for core and UI | Accepted (revised 2026-11-24) |
| [0002](0002-wgpu-gpu-abstraction.md) | `wgpu` as the GPU abstraction | Accepted (revised 2026-11-24) |
| [0003](0003-ffmpeg-for-media-io.md) | FFmpeg for media I/O, in-process preview decode in v1 | Accepted (revised 2026-11-24) |
| [0004](0004-rational-time-model.md) | Exact rational time model | Accepted |
| [0005](0005-command-driven-document.md) | Command-driven document, single writer | Accepted |
| [0006](0006-embedded-core-and-daemon.md) | Embedded core + headless CLI, one protocol | Accepted; superseded in part by 0013 |
| [0007](0007-native-wgpu-ui.md) | `egui` shell with custom GPU canvases | Accepted (replaces custom toolkit; amended by 0014; confirmed final by 0015) |
| [0008](0008-plugin-strategy.md) | Built-in effects now; plugin tiers deferred | Accepted (revised 2026-11-24) |
| [0009](0009-determinism-contract.md) | Same-machine determinism, tolerance goldens | Accepted (revised 2026-11-24) |
| [0010](0010-text-timeline-for-agents.md) | Read-only text timeline for agents | Accepted (revised 2026-11-24) |
| [0011](0011-windows-first.md) | Windows-first release strategy (v1.0 Windows, v1.1 macOS, Linux post-1.0) | Accepted 2026-11-24 |
| [0012](0012-project-interchange-export.md) | Cross-editor project export (FCPXML, FCP7 XML, OTIO) | Accepted 2026-11-24 |
| [0013](0013-product-modes-and-live-sessions.md) | Two product modes; live GUI sessions, takeover, Agent panel in v1 | Accepted; superseded in part by 0014 |
| [0014](0014-integration-levels-and-native-embedding.md) | Three integration levels; full native editor embedding as the goal | Accepted 2026-11-24; PoC GO 2026-11-25 |
| [0015](0015-final-ui-stack-and-design-system.md) | Final desktop UI stack: `egui` only, with an Arcline design system | **Accepted 2026-11-27 (final)** |
