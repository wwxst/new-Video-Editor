# Arcline — Agent-Native Professional Video Editor

> Codename **Arcline**. A native, cross-platform non-linear editor built around a deterministic,
> command-driven core that both humans and AI agents drive through the same interface.
>
> Built by a single developer with AI assistance. Windows-first, portable by design.

**Status:** Architecture v2 (post-review) — pre-implementation. No source code yet.
**Base:** [architecture review](reports/architecture-review.md), 2026-11-24.

---

## What this is

Arcline is a professional NLE (non-linear editor) for Windows, macOS, and Linux, written in Rust with a
`wgpu` rendering core and an `egui` application shell. It is designed from the first commit for two classes
of user:

1. **Humans** editing on a multi-track timeline with real-time GPU preview, effects, titles, audio, and
   reliable export.
2. **Agents** (LLMs, automation scripts) manipulating the same project through a typed command protocol,
   exposed as a local JSON-RPC service and as an MCP server.

The central architectural claim is unchanged: an edit is a **sequence of validated commands applied to an
immutable document snapshot**. The GUI emits commands; agents emit commands; undo, crash recovery, audit,
and the MCP server are consequences of that one decision.

What changed in v2, after review: a solo-calibrated scope, `egui` instead of a custom widget toolkit, six
crates instead of eighteen, a direct compositor instead of a generic render graph, in-process decode, a
simplified agent surface, and a 27-week roadmap to v1.0 on Windows.

---

## Document map

| # | Document | What it settles |
|---|----------|-----------------|
| 00 | [Product Scope](docs/00-product-scope.md) | Vision, users, requirement traceability, v1 non-goals, success criteria |
| 01 | [System Architecture](docs/01-system-architecture.md) | Process model, six-crate graph, threading, protocol, data flow |
| 02 | [Domain Model & Project Format](docs/02-domain-model-and-project-format.md) | Entities, rational time, commands, `.arcline` container, migrations |
| 03 | [Render Engine](docs/03-render-engine.md) | Direct compositor, effects, color, preview scheduler, export, determinism |
| 04 | [Media Pipeline](docs/04-media-pipeline.md) | Probe/decode/encode, hardware acceleration, proxies, caching |
| 05 | [Audio Engine](docs/05-audio-engine.md) | Sample-accurate mixing, device I/O, A/V sync |
| 06 | [Agent Interface](docs/06-agent-interface.md) | Command API, MCP tools, text timeline, local trust model, worked example |
| 07 | [UI Architecture](docs/07-ui-architecture.md) | `egui` shell, custom timeline/viewer canvases, inspector, input |
| 08 | [Performance & Reliability](docs/08-performance-reliability.md) | Budgets, memory policy, crash recovery, testing strategy |
| 09 | [Effects & Extensions](docs/09-effects-and-extensions.md) | Built-in effect manifests; deferred plugin tiers |
| 10 | [Development Roadmap](docs/10-roadmap.md) | Phases A–H, exit criteria, scope cut list, KPIs |
| 11 | [Risks & Legal](docs/11-risks-and-legal.md) | Risk register, codec/FFmpeg licensing, platform compliance |
| ADR | [Decision Records](docs/adr/README.md) | Decisions with alternatives; revised after review |
| — | [Architecture Review](reports/architecture-review.md) | Independent review that motivates v2 |

---

## Decisions at a glance

| Area | Decision | ADR |
|------|----------|-----|
| Language | Rust 2024, single workspace, no GC, no FFI boundary in the hot path | [0001](docs/adr/0001-rust-for-core-and-ui.md) |
| GPU | `wgpu` (Vulkan / Metal / D3D12) via `egui-wgpu`; one device, CPU uploads for video | [0002](docs/adr/0002-wgpu-gpu-abstraction.md) |
| UI | `egui` + `egui_dock` shell; timeline and viewer are custom-painted canvases | [0007](docs/adr/0007-native-wgpu-ui.md) |
| Codecs | FFmpeg `libav*` (LGPL build): preview decode in-process; import check, proxies, and exports in child processes | [0003](docs/adr/0003-ffmpeg-for-media-io.md) |
| Document | Immutable snapshots + single-writer command engine; all mutation via typed commands | [0005](docs/adr/0005-command-driven-document.md) |
| Time | Exact rational time; frame boundaries derived per project timebase | [0004](docs/adr/0004-rational-time-model.md) |
| Deployment | Embedded core in the app + headless CLI/MCP process; one JSON-RPC protocol | [0006](docs/adr/0006-embedded-core-and-daemon.md) |
| Crates | Six: `core`, `media`, `render`, `audio`, `app`, `cli` | [0001](docs/adr/0001-rust-for-core-and-ui.md) |
| Release plan | v1.0 Windows; v1.1 macOS port; Linux post-1.0, unscheduled | [0011](docs/adr/0011-windows-first.md) |
| Plugins | None in v1; internal effect manifests only; OFX/WASM/VST3 deferred | [0008](docs/adr/0008-plugin-strategy.md) |
| Determinism | Bit-exact per machine; tolerance-bounded golden tests; no cross-device contract in v1 | [0009](docs/adr/0009-determinism-contract.md) |
| Agent text | Read-only EDL-like text view; writes go through JSON commands | [0010](docs/adr/0010-text-timeline-for-agents.md) |

---

## Milestones (summary — details in [Roadmap](docs/10-roadmap.md))

| Phase | Weeks | Exit criterion |
|-------|-------|----------------|
| **A Spine** | 1–3 | `arcline-cli render` transcode is frame-accurate; command property tests pass 10k sequences |
| **B Headless pipeline** | 3–7 | Import → trim → export works end-to-end from the CLI with hardware encode |
| **C Compositor + shell** | 6–12 | 1080p preview ≥ 30 fps with 4 tracks and 8 effects; move/trim/split in the GUI |
| **D Audio** | 11–14 | 10-minute playback drifts ≤ 1 frame; preview/export mix hashes match |
| **E Lifecycle** | 13–16 | 100 forced kills during edit/save lose zero edits |
| **F Agent surface** | 15–19 | An agent builds a captioned three-shot cut through MCP |
| **G Finish** | 18–24 | A non-developer edits and exports a 10-minute project |
| **H Harden** | 23–27 | Budgets met; Windows installer; beta with external users |

**v1.0 (Windows): week 27.** v1.1 (macOS port, no new features): +3–4 weeks. Linux: post-1.0, no date.

---

## Repository layout (target)

```
Arcline/
├── crates/
│   ├── arc-core/           # time, model, commands, project file, schema
│   ├── arc-media/          # probe, decode, encode, mux, proxy jobs (FFmpeg)
│   ├── arc-render/         # wgpu compositor, effects, text, frame cache
│   ├── arc-audio/          # cpal output, mixer, offline render
│   ├── arc-app/            # egui shell, timeline/viewer canvases, app binary
│   └── arc-cli/            # headless render, JSON-RPC/MCP server, utilities
├── docs/                   # this proposal
├── reports/                # architecture review
└── assets/                 # icons, default LUT, fonts (licensed)
```

---

## How to read this proposal

Read [01 System Architecture](docs/01-system-architecture.md) first — it is the spine. For the product
differentiator, read [06 Agent Interface](docs/06-agent-interface.md). For scheduling and scope, read
[10 Roadmap](docs/10-roadmap.md); the cut list there is part of the design, not an afterthought.
