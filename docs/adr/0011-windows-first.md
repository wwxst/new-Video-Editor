# ADR-0011 — Windows-first release strategy

**Status:** Accepted, 2026-11-24. Clarified 2026-11-24 (explicit release numbering and CI tiers).

This ADR is the **single normative definition** of the release plan. Other documents reference it.

## Context

The product must eventually run on Windows, macOS, and Linux. For a solo developer, three FFmpeg build
matrices, three signing pipelines, and three platform QA cycles would dominate the schedule and delay the
first usable product by months. The stack (Rust, `wgpu`, `egui`, `cpal`, FFmpeg) is already portable, so the
question is sequencing, not architecture.

## Decision

### Release plan

| Release | Platform | Scope | Target |
|---|---|---|---|
| **v1.0** | Windows 10 21H2+ / 11, x64 | The full v1 feature set (Phases A–H) | ~week 27 |
| **v1.1** | macOS 13+, universal (Apple silicon + x86_64) | **Port only**: same features as v1.0, plus VideoToolbox decode/encode, CoreAudio, Metal validation, notarized DMG. No new product features. | v1.0 + 3–4 weeks |
| **Linux** | Ubuntu 22.04+ class, Flatpak | Port of the then-current release; system or Flatpak-provided FFmpeg | **Post-1.0, no date commitment**; started when there is demonstrated demand |

New features after v1.0 (audio DSP, round-trip text timeline, live GUI attach, plugins, HDR, interchange)
ship in later releases on all supported platforms; none of them is part of v1.1.

Windows on ARM is not a v1.0 target.

### Engineering rules that keep the later ports cheap

- Platform-specific code lives only in `arc-media` (hardware decode/encode selection) and `arc-app`
  (packaging, file associations). `arc-core`, `arc-render`, `arc-audio`, `arc-cli` stay platform-neutral.
- No Windows-only APIs outside those modules; file paths go through `std::path`, never string-joined.

### CI tiers

| Platform | Until v1.0 | From v1.1 work |
|---|---|---|
| Windows | build, full test suite (unit, property, golden, forced-kill), packaging | same |
| macOS | `cargo check` on every PR | full test suite + notarized packaging |
| Linux | `cargo check` + `arc-core` tests on every PR | unchanged until the Linux port starts |

## Consequences

- One shipping platform for v1.0 concentrates QA and feedback, and removes roughly 15–20% of release
  overhead from the schedule.
- macOS users wait for v1.1; Linux users wait longer, with no date. This is an explicit product trade.
- Version numbers carry meaning: v1.1 means "macOS", not "more features", which keeps the port small and
  predictable.
- Compile checks keep the other targets buildable, so the ports are porting work, not rescue work.

## Alternatives considered

- **Three-OS v1.0** (original proposal): broader reach; packaging and platform QA would dominate a solo
  schedule.
- **macOS-first**: excellent hardware, but the developer's primary machine and audience are Windows.
- **Linux-first**: cheapest packaging, smallest professional-video audience.
- **Simultaneous Windows + macOS v1.0**: roughly doubles release overhead in the final phase; rejected.
