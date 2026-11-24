# Project Schema Changelog

The `schema_version` integer in `manifest.json`/`project.json` is monotonic. Loaders accept any version
`<= CURRENT` and migrate forward through pure functions (`docs/02 §7`). This file records every change,
with the migration function name and its test fixture.

| Schema | Date | Change | Migration | Fixture |
|---|---|---|---|---|
| 7 | 2026-11-24 | Baseline for this proposal: sequences, tracks, clips, keyframed effect params, transitions, subtitles, masters | — | `fixtures/schema/v7_min.arcline` |

Rules for contributors:

1. Any change to `arc-core` model entities requires a `schema_version` bump and an entry here in the same PR.
2. A migration must be forward-only, deterministic, and covered by a round-trip fixture.
3. Removing a field requires a deprecation window of at least one minor release where it is accepted and
   ignored, then removed in the next schema version.
