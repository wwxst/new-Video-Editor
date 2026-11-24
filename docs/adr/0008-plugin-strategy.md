# ADR-0008 — Three-tier plugin strategy

**Status:** Accepted · 2026-11-24

## Context

Professional users expect third-party effects; the industry standard is OpenFX (OFX), which is native C and
cannot be sandboxed in-process. Agents, meanwhile, benefit from *authoring* small effects, which must be
safe to run without human review. One format cannot satisfy both trust profiles.

## Decision

Three tiers, explicit to the user:

1. **Built-in**: WGSL + manifest, compiled in, fully trusted, deterministic.
2. **WASM components**: untrusted; host-mediated buffer access, fuel/memory limits; used for agent-authored
   and downloaded effects.
3. **OFX**: native third-party binaries, hosted in a subprocess with crash isolation and per-bundle consent.

## Consequences

- Ecosystem compatibility without compromising the core: OFX brings existing plugin libraries.
- Agent-authored effects become a safe, supported workflow — a differentiator no incumbent offers.
- Two sandboxes to maintain; the OFX bridge is a substantial component (parameter/animation mapping, color
  space negotiation, tiling, threading) and is scheduled in P5, after the core effect model is proven.
- Performance ceiling for OFX is CPU-tile-bound; this is inherent to the ecosystem, not our design, and the
  UI flags "slow" third-party effects.
- Security posture improves: native code is never loaded into the main process, and WASM code cannot touch
  the filesystem or network at all.

## Alternatives considered

- **OFX only**: ecosystem yes, safety no; excludes agent authorship and inherits native crash risk.
- **WASM only**: safe but ecosystem-empty; performance ceiling too low for pro finishing effects.
- **Proprietary C ABI SDK**: full control, no ecosystem; would take years to attract vendors.
- **In-process OFX**: fastest interop, worst failure mode; rejected.
