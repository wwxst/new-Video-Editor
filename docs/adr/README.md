# Architecture Decision Records

ADRs are immutable once accepted. A superseding decision gets a new ADR that references the old one; the old
file's status changes to `Superseded by ADR-XXXX` and nothing else is edited.

Format: **Status · Context · Decision · Consequences · Alternatives considered.**

| ADR | Title | Status |
|---|---|---|
| [0001](0001-rust-for-core-and-ui.md) | Rust for core and UI | Accepted |
| [0002](0002-wgpu-gpu-abstraction.md) | `wgpu` as the GPU abstraction | Accepted |
| [0003](0003-ffmpeg-for-media-io.md) | FFmpeg for media I/O, out of process | Accepted |
| [0004](0004-rational-time-model.md) | Exact rational time model | Accepted |
| [0005](0005-command-driven-document.md) | Command-driven document, single writer | Accepted |
| [0006](0006-embedded-core-and-daemon.md) | Embedded core + optional daemon, one protocol | Accepted |
| [0007](0007-native-wgpu-ui.md) | Native retained-mode UI on `wgpu` | Accepted (with fallback) |
| [0008](0008-plugin-strategy.md) | Three-tier plugin strategy | Accepted |
| [0009](0009-determinism-contract.md) | Tiered determinism contract | Accepted |
| [0010](0010-text-timeline-for-agents.md) | Text timeline projection for agents | Accepted |
