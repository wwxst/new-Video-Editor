# 12 — Cross-Editor Project Export

Arcline exports editable projects to other editors so that an edit made by an agent or a human in Arcline
can be finished elsewhere. `.arcline` remains the native format; interchange files are **exports** — lossy
projections with an explicit report of what was lost. Import of foreign projects into Arcline is post-1.0.

Status of this document: architecture defined; target behavior **partially unverified**. Every claim about a
third-party editor is labeled with its evidence level and must be confirmed in the compatibility lab (§8)
before it is presented to users as supported.

## 1. Goals and non-goals

Goals:

1. Export FCPXML, FCP7 XML (`xmeml`), and OpenTimelineIO (OTIO) from any Arcline sequence.
2. Prioritize **Jianying Pro (剪映专业版), Premiere Pro, DaVinci Resolve, Final Cut Pro**.
3. Preserve editable structure: tracks, clips, source ranges, speed, transforms, opacity, transitions,
   audio levels/fades/pan, markers, subtitles; effects where a target can represent them.
4. Package media with the project, optionally consolidated or transcoded for the target.
5. Produce an automatic, per-clip compatibility report for every export.
6. Never present an unverified mapping as supported.

Non-goals (v1.0):

- Importing FCPXML/XML/OTIO into Arcline (post-1.0; OTIO first).
- Writing Jianying or CapCut native draft files (§5.5).
- AAF and EDL export (EDL may be added cheaply post-1.0 for conform workflows).
- Pixel-identical results in the target editor. Built-in effects differ between editors by design.

## 2. Target matrix

Evidence levels: **D** = vendor documentation; **R** = third-party reports only; **U** = unknown / conflicting;
**L** = confirmed in the Arcline compatibility lab (none yet).

| Target | Primary export | Secondary | Evidence today | Notes |
|---|---|---|---|---|
| **Final Cut Pro** | FCPXML (`.fcpxml` / `.fcpxmld` bundle) | — | D | FCPXML is FCP's native interchange format. Verification requires a Mac in the lab. |
| **DaVinci Resolve** | FCPXML | OTIO (`.otio` / `.otioz`, Resolve 18.5+) | D | Supported FCPXML versions lag FCP; the lab picks the default per Resolve version. |
| **Premiere Pro** | FCP7 XML (`xmeml`) | OTIO | D (xmeml) · U (OTIO) | Premiere does not import FCPXML natively. OTIO import/export appeared in the Premiere beta (Oct 2024); general-release status is unconfirmed. |
| **Jianying Pro** | to be decided in the lab: FCP7 XML or FCPXML | — | R | Third-party reports (2022) describe importing Premiere/FCP XML projects. Version, dialect, and fidelity are unverified. |
| **CapCut (desktop)** | none (handoff package only) | — | U | Recent sources report no timeline import/export; older claims of XML support are unreliable. Treated as **unsupported** until the lab proves otherwise. |

Jianying and CapCut are separate products with different feature sets. A result for one is never applied to
the other.

## 3. Architecture

No new crate. The mapping is pure and lives with the model; orchestration lives with the CLI.

```
arc-core::interchange              (pure: document → target IR → bytes, plus loss analysis)
├── ir.rs            flattened timeline: tracks, items, ranges, adjustments, markers, captions
├── profiles/        target profiles (data): per editor + version, what each feature maps to
├── fcpxml.rs        spine + connected-clip lanes, rational time, roles
├── xmeml.rs         track-based, integer frames + <rate><timebase/><ntsc/>
├── otio.rs          Timeline/Stack/Track/Clip/Gap/Transition JSON, metadata.arcline namespace
└── report.rs        per-feature, per-clip outcomes

arc-cli export job                 (child process, like render — 04 §1.2)
├── resolve snapshot at revision R
├── plan: profile + options → which clips need baking, packaging, transcoding
├── bake: render effect/title clips via arc-render + arc-media (ProRes)
├── package: copy / consolidate / transcode media; write sidecars (SRT, LUTs, stems)
├── serialize: one file per selected format
└── write interchange_report.json + interchange_report.md
```

Design rules:

- **Target profiles are data**, versioned with the app (`profiles/premiere-2025.json`, …). Each feature has a
  status (`native`, `approximate`, `bake`, `annotate`, `drop`) and an evidence level. The report is generated
  from the profile, so a mapping that the lab has not confirmed is reported as `unverified`.
- **One intermediate representation (IR)** feeds all writers; the loss analysis runs on the IR, so the
  report is consistent across formats.
- **Deterministic output**: the same snapshot and options produce byte-identical files (stable ordering,
  stable IDs derived from Arcline IDs).
- **Full-fidelity metadata**: OTIO carries the original Arcline clip/effect data under `metadata.arcline`, so
  a future OTIO import can round-trip losslessly into Arcline.

## 4. Format mapping

### 4.1 Time

| Format | Representation | Exactness |
|---|---|---|
| FCPXML | rational seconds, e.g. `"1001/30000s"` | exact; maps directly from `Time` |
| FCP7 XML | integer frames, `<timebase>30</timebase><ntsc>TRUE</ntsc>` for 30000/1001 | exact for frame-aligned times |
| OTIO | `RationalTime{value, rate}` with float `rate` | values written as frame counts at `rate = num/den`; CI verifies zero-frame drift over 24 h |

Timecode display (DF/NDF, [02 §2.3](02-domain-model-and-project-format.md)) is written as metadata only — FCPXML
`tcFormat="DF"|"NDF"`, FCP7 XML `<displayformat>`, OTIO `metadata.arcline` — and never changes timing.
Embedded source timecode is written where the format supports it.

Sub-frame audio boundaries (`allow_subframe`) are exact in FCPXML and rounded to the nearest frame in
FCP7 XML and OTIO; each rounding is listed in the report.

### 4.2 Structure

| Arcline | FCPXML | FCP7 XML | OTIO |
|---|---|---|---|
| Sequence | `project` / `sequence` with `format` | `sequence` | `Timeline` + `Stack` |
| Video track V1 | primary storyline (spine), gaps filled | `video/track[1]` | `Track(kind=Video)` |
| Video tracks V2+ | connected clips on lanes 1..n, anchored to spine items/gaps | `video/track[n]` | `Track(kind=Video)` |
| Audio tracks | connected clips on negative lanes, **roles** per track (dialogue/music/effects) | `audio/track[n]` | `Track(kind=Audio)` |
| Clip + source range | `asset-clip` with `offset/start/duration` | `clipitem` `in/out/start/end` | `Clip` + `source_range` + `ExternalReference` |
| Speed (constant) | `timeMap` | `Time Remap` / speed filter | `LinearTimeWarp` |
| Freeze frame | `timeMap` hold | still clip | `FreezeFrame` |
| Cross dissolve | `transition` (Cross Dissolve) | `transitionitem` (Cross Dissolve) | `Transition(SMPTE_Dissolve)` |
| Markers | `marker` | `marker` | `Marker` |

FCPXML's magnetic model is the hardest mapping: every upper-track clip must anchor to a spine element, so the
writer splits gaps on the spine at anchor points. This is covered by dedicated golden tests.

### 4.3 Properties and fallbacks

| Feature | FCPXML | FCP7 XML | OTIO | Fallback when not native |
|---|---|---|---|---|
| Transform (position/scale/rotation) | `adjust-transform` | Basic Motion | not standardized | `metadata.arcline` + report; optional bake |
| Opacity | `adjust-blend` | Opacity | not standardized | as above |
| Transform/opacity keyframes | keyframe animation | keyframes | none | bake, or first value + report |
| Audio gain / fades | `adjust-volume` + fades | levels + keyframes | not standardized | gain folded into metadata; stems in package |
| Pan | `adjust-panner` | pan filter | none | metadata + report |
| Track gain/mute | role/lane volume (approx.) | track settings | track `enabled` only | folded into clip gain + report |
| Built-in effects (color, LUT, blur, key, sharpen, vignette) | no portable equivalent | no portable equivalent | generic `Effect` name only | **annotate** (default) or **bake** |
| Titles | title generator (template-dependent) | text generator (limited) | none | **bake** to ProRes 4444 with alpha (default) + text in a marker |
| Subtitles | `caption` elements (target-dependent) | none | none | **SRT sidecar always**; embedded captions where the profile marks them native |
| VFR sources | — | — | — | warning; optional CFR transcode in the package |

**Fallback modes** (user/agent choosable per export):

- **annotate** — clip exported clean; a marker plus metadata names the effect and its parameters; LUTs are
  copied into the package as `.cube` files. Fully editable, look must be re-applied.
- **bake** — the clip is rendered with its effects to ProRes 422 HQ (ProRes 4444 for alpha) with handles and
  placed in the timeline; the original clip is kept directly beneath it, disabled. Faithful look, still
  editable.
- **drop** — effect omitted, recorded in the report. Never the default.

## 5. Media packaging

### 5.1 Package layout

```
Launch_Teaser_export/
├── Launch_Teaser.fcpxml          (or .xml / .otio, one per selected format)
├── media/                        original or consolidated media
├── baked/                        rendered effect/title clips
├── subtitles/                    SRT per subtitle track
├── luts/                         .cube files referenced by annotations
├── stems/                        optional WAV stems per audio track
├── reference/                    optional H.264 reference render of the full sequence
├── interchange_report.json
└── interchange_report.md
```

### 5.2 Packaging modes

| Mode | Behavior |
|---|---|
| `reference` | media stays in place; paths point to the originals |
| `copy` (default) | full media files copied into `media/` |
| `consolidate` | only used ranges plus handles (default 1 s) transcoded into `media/`; source offsets rewritten so timecodes stay correct |

Optional `transcode: prores422 | dnxhr_hq` converts sources the target handles poorly (profile-driven, e.g.
VFR phone HEVC). Transcodes and bakes run in the export child process and use the same encoders as render.

### 5.3 Paths

FCPXML and FCP7 XML reference media by absolute file URL; files are written with absolute paths at the
package location. If the package is moved, every target's relink function applies;
`arcline-cli export --reroot <dir>` rewrites paths without re-exporting. OTIO uses relative
`target_url`s; `.otioz` (media bundled) is produced for Resolve when selected.

### 5.4 CapCut handoff package

Because no editable import path is verified, CapCut gets a **handoff package**: media (consolidated by
default), SRT subtitles, audio stems, baked titles, a reference render, and a human-readable cut list
(`inspect.timeline` text). The user rebuilds the edit with everything prepared. If the lab confirms an import
path in a future CapCut version, a profile is added; nothing else changes.

### 5.5 Native draft formats (excluded)

Jianying and CapCut store projects as undocumented draft folders (`draft_content.json` / `draft_info.json`).
Generating them would rely on reverse engineering, the format changes between versions (recent versions
reportedly encrypt it), and it may conflict with the vendors' terms. Arcline does not write native drafts. If
a vendor publishes an import format or API, it becomes a new profile.

## 6. Compatibility report

Every export writes `interchange_report.json` (machine-readable, returned inline by `job.status`) and
`interchange_report.md` (human-readable).

```jsonc
{
  "arcline_revision": 42,
  "target": "premiere", "profile": "premiere-2025", "format": "xmeml-5",
  "evidence": "D",                         // weakest evidence level among used mappings
  "summary": { "clips": 38, "native": 31, "approximate": 3, "baked": 2, "annotated": 2, "dropped": 0,
               "unverified": 4 },
  "items": [
    { "clip": "cl_a", "timecode": "00:00:00:00", "feature": "effect:core.color.correct",
      "outcome": "annotated", "detail": "exposure=0.2, contrast=1.08 — re-apply in Lumetri" },
    { "clip": "cl_m", "feature": "audio:fade_out", "outcome": "native", "evidence": "D" },
    { "clip": "cl_t", "feature": "title", "outcome": "baked", "file": "baked/cl_t.mov" }
  ],
  "warnings": [ { "kind": "vfr_source", "asset": "as_b", "hint": "enable transcode for Premiere" } ],
  "validation": { "schema": "pass", "frame_count_check": "pass" }
}
```

Rules: every Arcline feature used in the sequence appears in the report with an outcome; `unverified`
mappings are counted separately and shown first in the Markdown report; `validation` records the structural
checks of §7.

## 7. Validation

Automated (CI, per PR):

- **FCPXML**: validate against Apple's published FCPXML DTD for the emitted version.
- **OTIO**: load with the reference `opentimelineio` library (dev-only dependency) and compare durations and
  ranges with the Arcline snapshot.
- **FCP7 XML**: XML well-formedness plus a structural schema maintained in-repo.
- **Frame-count check**: every writer's total duration and every clip range match the snapshot exactly.
- **Golden files**: 12 reference projects (cuts, multi-track, speed, transitions, titles, subtitles, audio
  fades, NTSC and integer rates, mixed-rate sources) → byte-stable outputs.

Manual (compatibility lab, §8): import into each target and record the result.

## 8. Compatibility lab

A repeatable manual test, run when an exporter or target version changes:

1. Export the 12 reference projects to each target with each fallback mode.
2. Import into the current version of each editor (Windows: Premiere, Resolve, Jianying Pro, CapCut; a Mac
   is required for Final Cut Pro).
3. For each feature, record: imported correctly / approximated / missing / import failed, plus editor version.
4. Update the target profiles and their evidence levels to **L**; attach screenshots and notes in
   `docs/interchange/lab-results/`.
5. Any feature not confirmed stays `unverified` in reports.

The first lab session is the **opening task** of the interchange phase; its results choose the default format
for Jianying Pro and Resolve, and decide whether Premiere OTIO is offered.

## 9. Agent and CLI surface

- JSON-RPC: `job.submit {kind: "interchange", spec}` — no new method ([06 §4.1](06-agent-interface.md)).
- MCP: `interchange_export` tool ([06 §6.1](06-agent-interface.md)); status and report via `render_status`.
- CLI: `arcline-cli export <project> --target premiere|resolve|fcp|jianying|capcut [--format …]
  [--package reference|copy|consolidate] [--effects annotate|bake] [--out <dir>]`.

```jsonc
"spec": {
  "targets": ["premiere", "resolve"],        // format chosen from each target's profile unless given
  "format": null,                             // or "fcpxml" | "xmeml" | "otio"
  "package": { "mode": "copy", "handles": "00:00:01:00", "transcode": "none",
               "stems": false, "reference_render": true },
  "effects": "annotate",                      // "annotate" | "bake" | "drop"
  "titles": "bake",                           // "bake" | "native"
  "output_dir": "exports/launch_teaser"
}
```

## 10. Licensing

- OTIO is Apache-2.0; the reference library is used only in CI for validation.
- FCPXML and FCP7 XML are publicly documented formats; Arcline writes them without Apple software.
- Editor names are used descriptively for compatibility; no affiliation is implied.
- No reverse-engineered vendor formats are written (§5.5).
