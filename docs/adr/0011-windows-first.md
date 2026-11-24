# ADR-0011 — Windows-first delivery strategy

**Status:** Accepted, 2026-11-24.

## Context

The product must eventually run on Windows, macOS, and Linux. The original plan targeted three signed
installers by week 60 with a ~10-person team. For a solo developer, maintaining three FFmpeg build matrices,
three signing/notarization pipelines, and three platform QA cycles multiplies release overhead and delays
the first usable product by months.

## Decision

**Windows is the only shipping target through v1.0.**

- Develop and test on Windows; keep all platform-specific code behind narrow modules in `arc-media` and
  `arc-app`.
- Run `cargo check` for macOS and Linux in CI from Phase A to prevent bit-rot.
- macOS support begins after Windows v1 (estimated +3–4 weeks: FFmpeg build, Metal validation,
  notarization, DMG).
- Linux is post-1.0, Flatpak-first, potentially community-assisted.
- Rust/`wgpu`/`egui`/`cpal` are already cross-platform, so this is a sequencing decision, not a
  portability compromise.

## Consequences

- Roughly 15–20% of release overhead is removed from the v1 schedule.
- Feedback and bug reports concentrate on one platform, which is appropriate for a first release.
- macOS users wait; Linux users wait longer. This is an explicit product trade, not an oversight.
- The cross-platform architecture remains real: no platform APIs leak outside the designated modules, and
  compile checks keep the other targets buildable.

## Alternatives considered

- **Three-OS v1** (original decision): broader reach, but packaging and platform QA would dominate the
  schedule for a single developer.
- **macOS-first**: excellent developer hardware, but the primary audience and test machine for this project
  is Windows.
- **Linux-first**: cheapest packaging, smallest professional-video audience.
