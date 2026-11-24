# 00 — Product Scope

## 1. Vision

Arcline is a professional non-linear editor whose primary interface is a **typed, versioned command
system**, with a native GUI as one of its clients. Today's editors are built GUI-first and then given a
scripting API that reaches into private state; automation is an afterthought and every AI integration ends
up as brittle screen-clicking or a wrapper around `ffmpeg` command lines that cannot represent an edit.

Arcline inverts this. The document and the commands that transform it are the product. The GUI is a
high-quality renderer of that document and a producer of those commands. This yields, for free:

- **Undo/redo, crash recovery, and audit** — the journal is the history.
- **Deterministic rendering** — commands + document + renderer version fully determine output.
- **Agent control with real semantics** — an agent manipulates clips, keyframes, and effects, not
  timestamps in a shell string.
- **Collaboration later** — a command log is a CRDT-shaped merge problem, not a rewrite.

## 2. Target users

| Persona | Needs | What Arcline gives them |
|---|---|---|
| **Agent developer** building video automation (marketing, newsroom, e-learning) | Programmatic access to *editing*, not just transcoding; verifiable results | MCP server, JSON-RPC, text timeline, frame-grab inspection, dry-run validation |
| **Solo creator / YouTuber** | Fast rough cut, captions, decent export, low friction | Proxy-backed 4K preview, auto-caption import, presets, one-key export |
| **Professional editor** | Frame accuracy, multi-track compositing, color, audio, reliable long sessions | Native performance, GPU effects, keyboard-first workflow, crash-safe autosave |
| **Post house / pipeline engineer** | Scripted batch deliverables, plugin compatibility, reproducibility | Headless daemon, render farm mode, OFX support, deterministic reference renders |

## 3. Requirement traceability

Every requirement from the project brief, mapped to where it is designed and when it ships.

| Requirement | Design section | Milestone |
|---|---|---|
| Windows, macOS, Linux | [01 §2](01-system-architecture.md), [04 §5](04-media-pipeline.md) | P0 (CI), P6 (signed installers) |
| Native cross-platform architecture | [01](01-system-architecture.md), ADR-0001/0007 | P0–P2 |
| Multi-track timeline | [02 §3](02-domain-model-and-project-format.md), [07 §4](07-ui-architecture.md) | P2 |
| GPU rendering | [03](03-render-engine.md) | P1 |
| Real-time preview | [03 §6](03-render-engine.md), [08 §2](08-performance-reliability.md) | P1 |
| Effects | [03 §4](03-render-engine.md), [09](09-plugin-system.md) | P1 (built-in), P5 (third-party) |
| Subtitles | [02 §3.7](02-domain-model-and-project-format.md), [04 §6](04-media-pipeline.md) | P3 (import/burn-in), P4 (styling) |
| Audio processing | [05](05-audio-engine.md) | P2 (mixing), P5 (VST3) |
| Video export | [04 §7](04-media-pipeline.md), [03 §7](03-render-engine.md) | P0, P4 (finishing formats) |
| Agent interfaces (MCP, APIs) | [06](06-agent-interface.md) | P3 |
| Native technologies, performance, stability, maintainability | [01](01-system-architecture.md), [08](08-performance-reliability.md) | All phases |

## 4. Non-goals (v1.0)

Stating these prevents scope drift and is itself an architectural constraint.

- **No cloud rendering service.** Arcline ships a local/on-prem daemon. A hosted tier is a later business
  decision, not a v1 architecture requirement; the daemon protocol is the seam if it ever happens.
- **No NLE-for-mobile.** Touch UI is out of scope; the timeline model is pointer+keyboard first.
- **No motion-graphics compositor** (After Effects class). Keyframed transforms, masks, and a curated
  effect set — but no expression language or node-based compositor in v1.
- **No built-in screen recording, live streaming, or asset generation** (AI video gen). We integrate by
  importing results, not by generating media.
- **No Windows 7 / macOS ≤ 13 / 32-bit targets.** Minimums: Windows 10 21H2, macOS 13, Ubuntu 22.04 /
  glibc 2.35 (Flatpak as the primary Linux channel).
- **No cloud sync of projects** in v1; the container format is designed so that it can be added without a
  migration.

## 5. What "professional" means here, concretely

A term that is otherwise marketing. In this project it means:

1. **Frame accuracy.** Every trim, cut, and seek lands on an exact frame of the project timebase; audio is
   sample-accurate to 1/48000 s. No float drift, ever — enforced by the type system ([02 §2](02-domain-model-and-project-format.md)).
2. **Color management.** Linear-light RGBA16F internal pipeline, explicit input/output transforms, HDR10
   (PQ) and HLG output, 10-bit and 12-bit intermediate option.
3. **Uncompromised session stability.** Autosave every N commands, append-only journal, crash recovery
   that restores the last committed command, no data loss on GPU device loss.
4. **Broad-codec ingest, certified-format egress.** ProRes, DNxHR, H.264/HEVC, AV1, image sequences,
   WAV/AIFF/FLAC/Opus, with loudness-normalized audio.
5. **Predictable performance.** Published budgets ([08 §2](08-performance-reliability.md)) and CI gates on them, not
   "it feels fast on my machine."
6. **Interoperability.** Import/export EDL, FCPXML and OTIO for round-tripping with other tools; OFX
   plugin compatibility.

## 6. Competitive positioning

| Product | Strength | Where Arcline differs |
|---|---|---|
| Premiere Pro | Ecosystem, familiarity | Arcline is agent-first and deterministic; no subscription-locked automation surface |
| DaVinci Resolve | Color, free tier | Arcline is lighter, codec-honest, and scriptable end-to-end; color is good, not a node-graph suite |
| Final Cut Pro | Magnetic timeline speed | Arcline keeps a conventional, explicit timeline (agents need determinism, not inference) |
| CapCut / Descript | Consumer speed, transcript editing | Arcline targets multi-track professional structure with a transcript view as an *overlay*, not the model |
| Programmatic (Shotstack, Remotion, FFmpeg scripts) | Fully automatable | Arcline is a real editor: preview, audio, color, plugins — with the same programmability |

The unoccupied position: **a real NLE whose API is not a shadow of the GUI but its equal.**

## 7. Success criteria for v1.0

- An agent, given a folder of raw footage and a brief, can produce a reviewable 60-second cut with
  captions, music ducking, and a branded end card, using only MCP tools, in under 10 minutes of wall time,
  and a human can then open the same project and finish it by hand.
- A 10-minute 1080p30 project with 8 video tracks, 20 effects, and 6 audio tracks previews at ≥ 30 fps on
  a mid-tier discrete GPU and on Apple silicon.
- Zero lost-edit incidents across a 30-day internal soak with forced-kill testing.
- Third-party OFX plugin binary runs unmodified within one release of 1.0.
