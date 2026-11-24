# Arcline — Agent-Native Professional Video Editor

> Codename **Arcline**. A native, cross-platform non-linear editor built around a deterministic,
> command-driven core that both humans and AI agents drive through the same interface.

**Status:** Architecture proposal — pre-implementation (Phase 0 not started).
**Authored:** 2026-11-24.

---

## What this is

Arcline is a professional NLE (non-linear editor) for Windows, macOS, and Linux, written in Rust with a
`wgpu` rendering core. It is designed from the first commit for two classes of user:

1. **Humans** editing on a multi-track timeline with real-time GPU preview, effects, titles, audio, and
   broadcast-grade export.
2. **Agents** (LLMs, automation scripts, pipeline orchestrators) that manipulate the same project through a
   typed command protocol, exposed as a local JSON-RPC service and as an MCP server.

The central architectural claim is that these are not two products bolted together. An edit is a
**sequence of validated commands applied to an immutable document snapshot**. The GUI emits commands;
agents emit commands; the undo stack, the crash journal, the collaboration layer, and the MCP server are
all consequences of that one decision.

---

## Document map

| # | Document | What it settles |
|---|----------|-----------------|
| 00 | [Product Scope](docs/00-product-scope.md) | Vision, personas, requirement traceability, non-goals, positioning |
| 01 | [System Architecture](docs/01-system-architecture.md) | Process model, crate graph, threading, protocol, data flow |
| 02 | [Domain Model & Project Format](docs/02-domain-model-and-project-format.md) | Entities, rational time, commands, `.arcline` container, migrations |
| 03 | [Render Engine](docs/03-render-engine.md) | Render graph, GPU abstraction, color, preview scheduler, export, determinism |
| 04 | [Media Pipeline](docs/04-media-pipeline.md) | Demux/decode, hardware acceleration, proxies, caching, encode/mux |
| 05 | [Audio Engine](docs/05-audio-engine.md) | Sample-accurate graph, device I/O, A/V sync, loudness |
| 06 | [Agent Interface](docs/06-agent-interface.md) | Command API, MCP surface, text timeline DSL, security, worked example |
| 07 | [UI Architecture](docs/07-ui-architecture.md) | Widget layer, docking, timeline renderer, inspector, input |
| 08 | [Performance & Reliability](docs/08-performance-reliability.md) | Budgets, memory policy, crash recovery, testing strategy |
| 09 | [Plugin System](docs/09-plugin-system.md) | OFX bridge, WASM sandbox, native ABI, permissions |
| 10 | [Development Roadmap](docs/10-roadmap.md) | Phases, exit criteria, staffing, schedule, KPIs |
| 11 | [Risks & Legal](docs/11-risks-and-legal.md) | Risk register, codec/FFmpeg licensing, platform compliance |
| ADR | [Decision Records](docs/adr/README.md) | Ten load-bearing decisions with alternatives considered |

---

## Decisions at a glance

| Area | Decision | ADR |
|------|----------|-----|
| Language | Rust 2024, single workspace, no GC pauses, no FFI boundary for the hot path | [0001](docs/adr/0001-rust-for-core-and-ui.md) |
| GPU | `wgpu` (Vulkan / Metal / D3D12) with a thin capability-tiered device layer | [0002](docs/adr/0002-wgpu-gpu-abstraction.md) |
| Codecs | FFmpeg `libav*` behind an owned decode/encode abstraction, hardware-first | [0003](docs/adr/0003-ffmpeg-for-media-io.md) |
| Document | Immutable snapshots + single-writer command engine; all mutation via typed commands | [0005](docs/adr/0005-command-driven-document.md) |
| Time | Exact rational time; frame boundaries derived per project timebase | [0004](docs/adr/0004-rational-time-model.md) |
| Deployment | Embedded core + optional headless daemon speaking one JSON-RPC protocol | [0006](docs/adr/0006-embedded-core-and-daemon.md) |
| UI | Own retained-mode widget layer on `wgpu`; no Electron, no Qt | [0007](docs/adr/0007-native-wgpu-ui.md) |
| Plugins | OFX-compatible host + WASM sandbox for untrusted/agent-authored effects | [0008](docs/adr/0008-plugin-strategy.md) |
| Determinism | Bit-exact per device; tolerance-bounded across devices; reference mode for goldens | [0009](docs/adr/0009-determinism-contract.md) |
| Agent text | Round-trippable EDL-like text projection of the timeline as a first-class resource | [0010](docs/adr/0010-text-timeline-for-agents.md) |

---

## Milestones (summary — details in [Roadmap](docs/10-roadmap.md))

| Phase | Weeks | Exit criterion |
|-------|-------|----------------|
| **P0 Kernel** | 1–6 | `arcline render clip.mp4 -o out.mp4` produces a frame-accurate transcode |
| **P1 Pipeline** | 5–14 | 1080p30 timeline with 8 tracks and 20 effects previews in real time |
| **P2 Editing** | 10–24 | A human edits and exports a 10-minute project without touching a config file |
| **P3 Agent Surface** | 18–30 | An agent produces a captioned 3-shot edit end-to-end through MCP |
| **P4 Finishing** | 25–40 | Broadcast deliverables: HDR, loudness-normalized, subtitle burn-in, ProRes |
| **P5 Ecosystem** | 36–52 | Third-party OFX plugin runs unmodified; WASM effect authored by an agent ships |
| **P6 1.0** | 50–60 | Signed installers on 3 OSes, crash-free-session ≥ 99.5%, docs complete |

---

## Repository layout (target)

```
Arcline/
├── Cargo.toml              # workspace
├── crates/
│   ├── arc-time/           # rational time, timebase, frame math
│   ├── arc-model/          # document entities, schema, migrations
│   ├── arc-command/        # command types, validation, apply engine, undo
│   ├── arc-media/          # probe, demux/decode, hardware accel
│   ├── arc-cache/          # frame/proxy/waveform caches
│   ├── arc-render/         # render graph, wgpu device layer, passes
│   ├── arc-effects/        # built-in effects + parameter schemas
│   ├── arc-audio/          # audio graph, resampling, devices, metering
│   ├── arc-text/           # shaping, titles, subtitles
│   ├── arc-encode/         # muxing, presets, encoder orchestration
│   ├── arc-protocol/       # JSON-RPC types, capability tokens, versioning
│   ├── arc-server/         # daemon, transports, MCP facade
│   ├── arc-cli/            # `arcline` binary (render/probe/serve)
│   ├── arc-ui/             # widget tree, docking, theme, input
│   ├── arc-app/            # desktop shell binary
│   ├── arc-plugin-host/    # OFX + WASM + C ABI
│   └── arc-testkit/        # golden-frame harness, generators, fixtures
├── docs/                   # this proposal
└── assets/                 # icons, default lut, fonts (licensed)
```

---

## How to read this proposal

If you read one thing, read [01 System Architecture](docs/01-system-architecture.md) — it is the spine.
If you are the agent-integration stakeholder, read [06 Agent Interface](docs/06-agent-interface.md) first;
it is the differentiator and it constrains the document model. If you are scheduling, read
[10 Roadmap](docs/10-roadmap.md) and [11 Risks & Legal](docs/11-risks-and-legal.md) together; the legal
posture around codecs affects what Phase 1 can ship.
