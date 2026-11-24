# ADR-0012 — Cross-editor project export

**Status:** Accepted, 2026-11-24. Replaces the earlier v1 deferral of interchange formats (previously in
00 §4 and 09 §5).

## Context

Agent-built edits are most valuable when a human can finish them in the editor they already use. Users
named Jianying Pro, Premiere Pro, DaVinci Resolve, and Final Cut Pro as priorities, and CapCut as a target to
investigate. Research (2026-11) found:

- Final Cut Pro uses FCPXML (vendor-documented).
- DaVinci Resolve imports FCPXML and OTIO (18.5+), including `.otioz` bundles.
- Premiere Pro imports FCP7 XML but **not FCPXML** natively; its OTIO support appeared in beta (Oct 2024) and
  general-release status is unconfirmed.
- Jianying Pro reportedly imports Premiere/FCP XML (third-party reports only).
- CapCut desktop reportedly has no timeline import/export; older contrary claims are unreliable.

## Decision

1. Keep `.arcline` as the only native format. Interchange is export-only in v1.0.
2. Implement three writers from one intermediate representation: **FCPXML**, **FCP7 XML (`xmeml`)**, and
   **OTIO**. FCP7 XML is added beyond the original FCPXML + OTIO request because it is the only
   documented editable path into Premiere Pro.
3. Drive mapping decisions from versioned **target profiles** with evidence levels; anything not confirmed in
   the compatibility lab is reported as `unverified`.
4. Preserve editability first: unsupported effects are **annotated** by default, or **baked** with the
   original kept beneath; titles are baked by default; subtitles always ship as SRT.
5. Package media (`reference` / `copy` / `consolidate`, optional transcode) and write a compatibility report
   for every export.
6. CapCut receives a handoff package, not an editable project, until an import path is verified.
7. Never write reverse-engineered native draft formats (Jianying/CapCut).
8. Place the mapping in `arc-core::interchange` (pure) and the export job in `arc-cli` (child process). No new
   crate.

Details: [12 — Cross-Editor Project Export](../12-project-interchange.md).

## Consequences

- v1.0 grows by one phase (~5 weeks). (Calendar weeks in the original text were superseded by the sequential
  roadmap recalculation of 2026-11-24; see [10](../10-roadmap.md).)
- Agents can hand a rough cut to a human in another editor with an honest account of what survived.
- Lab work is recurring: target editors change, so profiles are re-verified when versions change.
- Final Cut Pro verification needs access to a Mac even though v1.0 ships on Windows only.
- Three writers is more code than two, but FCP7 XML is simple and track-based; the IR keeps writers small.

## Alternatives considered

- **FCPXML + OTIO only**: leaves Premiere dependent on a beta OTIO feature or third-party converters.
- **OTIO only, relying on adapters**: OTIO's own FCPXML/FCP7 adapters exist in Python; shipping Python
  contradicts the native stack, and adapter fidelity would still need verification.
- **Native Jianying/CapCut drafts**: highest fidelity for those apps, but undocumented, version-fragile,
  reportedly encrypted in newer versions, and potentially against vendor terms; rejected.
- **Defer interchange to post-1.0** (previous plan): simpler v1.0, but loses the key "agent drafts, human
  finishes anywhere" workflow.
