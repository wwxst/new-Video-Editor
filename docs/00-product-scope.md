# 00 — Product Scope

## 1. Vision

Arcline is a professional non-linear editor whose primary interface is a **typed, versioned command
system**, with a native GUI as one of its clients. Most editors are GUI-first and bolt on scripting later;
every AI integration then becomes screen-clicking or a fragile wrapper around `ffmpeg` command lines that
cannot represent an edit.

Arcline inverts this. The document and the commands that transform it are the product. The GUI renders the
document and produces commands. This yields undo, crash recovery, audit, deterministic rendering, and real
agent control from one mechanism.

**Build reality.** Arcline is developed by one person with AI assistance. That constraint is a design input,
not an excuse: scope is vertical-slice driven, dependencies are minimized, and anything that does not
produce user-visible capability in the first six months is deferred.

## 2. Target users

| Persona | Needs | What Arcline gives them |
|---|---|---|
| **Agent developer** building video automation | Programmatic *editing*, not just transcoding; verifiable results | MCP server, JSON-RPC, text timeline view, rendered-frame inspection |
| **Solo creator** | Fast rough cut, captions, decent export | Proxy-backed preview, simple effects, presets, one-key export |
| **Professional editor** (aspirational for v1) | Frame accuracy, multi-track compositing, stability | Rational time, crash-safe autosave, GPU compositing, keyboard-first editing |

## 3. Requirement traceability

| Requirement | Design section | Milestone |
|---|---|---|
| Windows, macOS, Linux | [01 §2](01-system-architecture.md), [04 §6](04-media-pipeline.md), [11 §3](11-risks-and-legal.md) | v1.0 Windows (Phase H); v1.1 macOS; Linux post-1.0 ([ADR-0011](adr/0011-windows-first.md)) |
| Native cross-platform architecture | [01](01-system-architecture.md), ADR-0001/0002/0011 | A–C |
| Multi-track timeline | [02 §3](02-domain-model-and-project-format.md), [07 §4](07-ui-architecture.md) | C |
| GPU rendering | [03](03-render-engine.md) | C |
| Real-time preview | [03 §6](03-render-engine.md), [08 §2](08-performance-reliability.md) | C |
| Effects | [03 §4](03-render-engine.md), [09](09-effects-and-extensions.md) | C, G |
| Subtitles | [02 §3.7](02-domain-model-and-project-format.md) | G |
| Audio processing | [05](05-audio-engine.md) | D |
| Video export | [04 §7](04-media-pipeline.md), [03 §7](03-render-engine.md) | B, G |
| Cross-editor project export (FCPXML, FCP7 XML, OTIO; media packaging; compatibility report) | [12](12-project-interchange.md), [ADR-0012](adr/0012-project-interchange-export.md) | I |
| Agent interfaces (MCP, APIs) | [06](06-agent-interface.md) | F |
| Performance, stability, maintainability | [08](08-performance-reliability.md) | A–I |

## 4. Non-goals for v1.0

Explicit and binding. Each item removed here is weeks returned to the core product.

- **No plugin system.** No OFX, no WASM effects, no VST3. Built-in effects only; the effect manifest keeps
  the door open ([09](09-effects-and-extensions.md)).
- **No HDR, ACES, or OCIO.** Rec.709 SDR with LUT support is the v1 color story.
- **No collaboration or multi-user.** Command logs remain mergeable in principle; live sync is post-1.0.
- **No live GUI agent attach.** Agents run headless against project files; attaching to an open GUI session
  is post-1.0 and unscheduled.
- **No project import from other editors** (export only; see §8). No AAF/EDL, and no native Jianying/CapCut
  draft files.
- **No multicam, cloud rendering, render farms, multi-GPU, or mobile.**
- **No screen-reader certification** in v1; keyboard completeness is required instead.
- **No macOS or Linux in v1.0.** v1.0 is Windows-only; v1.1 is the macOS port; Linux is post-1.0 with no date
  ([ADR-0011](adr/0011-windows-first.md)).
- **No motion-graphics compositor** (masks/tracking/node graphs are post-1.0).

## 5. What "professional" means here

1. **Frame accuracy.** Every trim, cut, and seek lands on an exact frame of the project timebase; audio is
   sample-accurate. Floats are banned from the timing path ([02 §2](02-domain-model-and-project-format.md)).
2. **Color correctness.** Linear-light RGBA16F internal pipeline, explicit input/output transforms,
   Rec.709 output, 3D LUT support.
3. **Session stability.** Autosave, append-only journal, crash recovery to the last committed command.
4. **Codec breadth.** Broad ingest; H.264/HEVC hardware encode, AV1 software encode, ProRes mezzanine.
5. **Predictable performance.** Published budgets with CI gates ([08 §2](08-performance-reliability.md)).
6. **Agent parity.** Anything a human can do, an agent can do through typed commands.

## 6. Competitive positioning

| Product | Strength | Where Arcline differs |
|---|---|---|
| Premiere Pro | Ecosystem | Arcline is agent-first and deterministic; far smaller surface |
| DaVinci Resolve | Color suite | Arcline is lighter and fully scriptable; color is good, not a suite |
| Final Cut Pro | Magnetic timeline | Arcline keeps an explicit timeline — agents need determinism |
| CapCut / Descript | Consumer speed | Arcline targets multi-track structure and real audio |
| Shotstack / Remotion / FFmpeg scripts | Fully automatable | Arcline is a real editor with preview, audio, and effects — equally scriptable |

The unoccupied position remains: **a real NLE whose API is not a shadow of the GUI but its equal**, built
by one person with AI.

## 7. Success criteria for v1.0 (Windows)

- An agent, given footage and a brief, produces a reviewable 60-second cut with captions, music, and a title
  through MCP only — and a human can open the same project and finish it.
- A 10-minute 1080p30 project with 4 video tracks and 8 effects previews at ≥ 30 fps on mid-tier hardware.
- Zero lost acknowledged edits across a 30-day soak with forced-kill testing
  ([08 §4.7](08-performance-reliability.md)).
- A non-developer completes import → cut → export without reading documentation.
- An Arcline sequence exported for Premiere Pro, DaVinci Resolve, and Final Cut Pro opens as an editable
  timeline with cuts, tracks, audio levels, transitions, and markers intact, with a compatibility report that
  lists every approximation; Jianying Pro reaches the same bar if the compatibility lab confirms an import
  path.

## 8. Cross-editor project export

Arcline edits must be finishable in other editors. Requirements (design: [12](12-project-interchange.md),
decision: [ADR-0012](adr/0012-project-interchange-export.md)):

1. **Native format stays `.arcline`.** Interchange files are exports; v1.0 does not import them.
2. **Formats:** FCPXML and OpenTimelineIO (OTIO), plus FCP7 XML (`xmeml`) because Premiere Pro does not
   import FCPXML natively.
3. **Priority targets:** Jianying Pro, Premiere Pro, DaVinci Resolve, Final Cut Pro.
4. **CapCut:** investigated in the compatibility lab. No editable path is assumed; until one is verified,
   CapCut receives a handoff package (media, SRT, stems, baked titles, reference render, cut list).
5. **Editability first:** timelines, clips, source ranges, speed, transforms, transitions, audio levels and
   fades, markers, and subtitles are preserved wherever the target can represent them.
6. **Explicit fallbacks:** unsupported effects are annotated (default) or baked with the original kept
   beneath; titles are baked to alpha clips by default; subtitles always ship as SRT; nothing is silently
   dropped.
7. **Media packaging:** reference, copy, or consolidate (used ranges + handles), with optional transcode for
   target-unfriendly sources.
8. **Automatic compatibility report** for every export: per clip and feature, with outcome and evidence
   level; unverified mappings are labeled as such.
9. **Agent access:** the same export is available to agents via MCP and JSON-RPC.
