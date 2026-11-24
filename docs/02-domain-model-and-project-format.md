# 02 — Domain Model & Project Format

Everything here is normative and lives in `arc-core`: the Rust types, the JSON wire form, and the on-disk
form are the same model in three encodings.

## 1. Entity overview

```
Project
├── meta:            name, created, modified, app_version, schema_version
├── settings:        timebase, resolution, color space, sample rate, autosave policy
├── assets:          AssetId → Asset            (content-referenced media)
├── sequences:       SequenceId → Sequence      (v1: one active sequence)
│   └── tracks:      Track[]                    (video / audio / subtitle, ordered bottom→top)
│       └── clips:   Clip[]
│           ├── source: SourceRef               (asset + in/out, generator, or title)
│           ├── timing: start, duration, speed, freeze
│           ├── transform: position, scale, rotation, opacity, anchor
│           ├── effects: EffectInstance[]        (ordered chain, keyframed params)
│           ├── audio:   gain, pan, fades, channel map
│           └── meta:    label, color, locked, enabled
├── markers:         Marker[]
└── subtitles:       SubtitleDoc[]
```

IDs are opaque, sortable ULID-style values (`cl_01J8…`, `as_…`, `sq_…`, `tr_…`, `fx_…`). Clients treat them
as equality-only.

## 2. Time

The single most important type in the system.

```rust
/// Exact rational seconds. Normalized: den > 0, gcd(num, den) == 1.
pub struct Time { num: i64, den: u32 }   // e.g. 1001/30000 = one NTSC frame

/// A sequence timebase. Stored as the frame duration; the frame rate is its exact reciprocal.
pub struct Timebase { frame_duration: Time }
impl Timebase { pub fn frame_rate(&self) -> Rational; }   // 1 / frame_duration

pub fn frame_at(time: Time, tb: Timebase, mode: Round) -> i64;
pub fn time_of_frame(frame: i64, tb: Timebase) -> Time;
```

### 2.1 Frame rate vs frame duration (canonical terminology)

Two quantities, never interchanged, used identically in every document, schema, and example:

| Term | Meaning | Unit | NTSC 29.97 example | Where it appears |
|---|---|---|---|---|
| `frame_rate` | frames per second | frames/s | `30000/1001` | agent-facing summaries, `project.open` create params, asset probe, text timeline (`fps=`) |
| `frame_duration` | seconds per frame | s | `1001/30000` | stored sequence timebase (`timebase.frame_duration`), internal `Time` math |

Rules:

- `frame_rate × frame_duration = 1`, exactly. Conversion is a rational reciprocal, never a float.
- The word **timebase** names the stored `{frame_duration}` object only. A rate is never labeled "timebase".
- In JSON, the stored form is `{"num": …, "den": …}`; agent-facing summaries may use the string form
  `"30000/1001"`, always under the key `frame_rate`.
- **Assets** report a nominal `frame_rate` from the container. Variable-frame-rate sources (common on
  phones) set `"vfr": true`; their frames are mapped to the sequence timebase by presentation timestamp,
  with the nominal rate used only for display.
- **Time inputs on the wire** are either an exact rational `{"num","den"}` (seconds) or a timecode string
  `"HH:MM:SS:FF"` interpreted at the sequence `frame_rate`. Both are normalized to `Time` before validation;
  a rational that is not on a frame boundary fails with `off_frame_boundary`.

### 2.2 Time rules

1. **No floats in the timing path.** `f64` seconds appear only for display and in effect parameters, never
   in clip boundaries or timing serialization.
2. **Frame-grained by default.** Clip boundaries must land on the project timebase (`off_frame_boundary`
   otherwise). Audio-only clips may set `allow_subframe: true` and then must land on the sample rate.
3. **Speed changes are rational.** `speed: {num, den}` with `num/den ∈ (0, 16]`. `duration` is timeline
   duration; source in/out are source time; `timeline_duration × speed == source_range` is checked with
   i128 intermediates.
4. **NTSC drop-frame** is a display concern (`HH:MM:SS;FF`), never a storage concern.
5. **Time is non-negative**, except keyframe times relative to clip start (pre-roll). Sequence length is
   capped at 24 h as a sanity limit.

## 3. Entity detail

### 3.1 Asset

```jsonc
{
  "id": "as_01J8Z…",
  "kind": "video" | "audio" | "image" | "generator",
  "path": "media/interview_a.mov",         // relative to project root; may be missing
  "content_hash": "blake3:…",              // size + mtime + first 4 MB
  "probe": {                               // cached, nullable, refreshed async
    "duration": {"num": 7502, "den": 1001},
    "streams": [ { "index": 0, "type": "video", "codec": "h264",
                   "width": 3840, "height": 2160, "frame_rate": {"num": 30000, "den": 1001},
                   "vfr": false, "pix_fmt": "yuv420p10le", "rotation": 0 } ]
  },
  "proxy": "cache/proxies/as_01J8Z…/half.mp4",
  "waveform": "cache/wave/as_01J8Z….peak",
  "color_source": "auto" | "bt709" | "custom:{lut}"
}
```

Assets are referenced by relative path where possible so a project folder is portable. Missing files do
not fail the open; they become **offline assets** that render as placeholders and can be relinked by hash.

### 3.2 Sequence and tracks

```jsonc
{
  "id": "sq_01J8Z…",
  "name": "Main",
  "timebase": {"frame_duration": {"num": 1, "den": 30}},     // frame_rate 30/1
  "resolution": {"w": 1920, "h": 1080},
  "working_space": "rec709",
  "sample_rate": 48000,
  "tracks": [
    { "id": "tr_V1", "kind": "video", "index": 0, "name": "V1",
      "enabled": true, "locked": false, "blend": "normal", "opacity": 1.0,
      "clips": [ /* Clip[], sorted by start, non-overlapping */ ] }
  ]
}
```

Invariants: clips on a track never overlap; indices are dense and unique; z-order is by index (higher
composites on top); deleting a track deletes its clips atomically; commands address clips by ID, never by
position.

### 3.3 Clip

All times below sit on frame boundaries of the 30 fps sequence in §3.2. Audio fades are sample-grained
(1/4 s = 12,000 samples at 48 kHz) and are exempt from the frame-boundary rule.

```jsonc
{
  "id": "cl_01J8Z…",
  "asset": "as_01J8Z…",
  "start": {"num": 4, "den": 1},
  "source_in": {"num": 110, "den": 1},
  "duration": {"num": 130, "den": 30},
  "speed": {"num": 1, "den": 1},
  "freeze": null,
  "transform": {
    "position": {"x": 0.0, "y": 0.0}, "scale": {"x": 1.0, "y": 1.0},
    "rotation_deg": 0.0, "anchor": {"x": 0.5, "y": 0.5}, "opacity": 1.0
  },
  "effects": [
    { "id": "fx_01J8Z…", "effect": "core.color.correct", "enabled": true,
      "params": {
        "exposure": {"static": 0.15},
        "contrast": {"curve": [
          {"t": {"num": 0, "den": 1}, "v": 1.00, "interp": "linear"},
          {"t": {"num": 2, "den": 1}, "v": 1.10, "interp": "bezier", "ease": [0.4,0,0.2,1]}
        ]}
      } }
  ],
  "audio": {
    "gain_db": -3.0, "pan": 0.0,
    "fade_in": {"num": 1, "den": 4}, "fade_out": {"num": 1, "den": 2},
    "channel_map": "stereo"
  },
  "label": "A-roll", "color": "#ff8a3d", "enabled": true, "locked": false
}
```

**Parameter values** have exactly two forms, for every animated property:

```jsonc
{"static": 0.5}
{"curve": [ Keyframe, … ]}   // sorted, unique times, relative to clip start
```

A keyframe is `{t: Time, v: f32|Vec|bool|string, interp: "linear"|"hold"|"bezier"|"ease", ease?}`. One
representation means the inspector, MCP tools, and the evaluator share one code path.

### 3.4 Effects

An effect is defined by a manifest; the same manifest drives the inspector, MCP discovery, and validation:

```jsonc
{
  "id": "core.color.correct",
  "version": 1,
  "display": "Color Correction",
  "category": ["color"],
  "gpu": { "shader": "builtin:color_correct", "precision": "fp16_ok" },
  "params": [
    { "key": "exposure",   "type": "float", "range": [-5, 5],    "default": 0.0, "unit": "stops" },
    { "key": "contrast",   "type": "float", "range": [0.2, 2.0], "default": 1.0 },
    { "key": "saturation", "type": "float", "range": [0.0, 2.0], "default": 1.0 },
    { "key": "temperature","type": "float", "range": [-100, 100],"default": 0.0 }
  ]
}
```

Effect manifests are versioned. On load, params migrate through per-effect migration functions. An
unresolvable effect renders as a pass-through placeholder and is reported by `project.describe`; it is never
silently dropped. v1 ships six built-ins ([03 §4](03-render-engine.md)); third-party plugins are deferred
([09](09-effects-and-extensions.md)).

### 3.5 Transitions

A transition is a relationship between adjacent clips, not a third clip:

```jsonc
{ "id": "tx_…", "kind": "cross_dissolve", "a": "cl_…", "b": "cl_…",
  "duration": {"num": 1, "den": 2}, "alignment": "center" }
```

Adding a transition does not move clips unless the command explicitly sets `ripple: true` — an
agent-friendly determinism property.

### 3.6 Audio

Gain, pan, fades, channel map per clip; track and master gain/mute/solo. Equalization, compression, and
loudness normalization are post-1.0 ([05 §6](05-audio-engine.md)).

### 3.7 Subtitles and titles

A subtitle track is a `Track { kind: "subtitle" }` whose clips reference a `SubtitleDoc` of timed cues.
This reuses every timeline operation instead of inventing a parallel editing model. Titles are clips whose
source is a text scene with styled runs and keyframable properties.

## 4. Commands

The v1 command set. Every command is a JSON object with `op` and typed fields, validated centrally in
`arc-core::command`.

| Group | Commands |
|---|---|
| Project | `project.rename`, `project.set_settings` |
| Asset | `asset.import`, `asset.relink`, `asset.remove`, `asset.generate_proxy` |
| Track | `track.add`, `track.remove`, `track.reorder`, `track.set_properties` |
| Clip | `timeline.add_clip`, `timeline.remove_clip`, `timeline.move_clip`, `timeline.trim_clip`, `timeline.split_clip`, `timeline.set_speed`, `timeline.set_transform`, `timeline.set_enabled`, `timeline.set_label` |
| Transition | `timeline.add_transition`, `timeline.remove_transition` |
| Effect | `effect.add`, `effect.remove`, `effect.reorder`, `effect.set_params`, `effect.set_keyframes`, `effect.set_enabled` |
| Audio | `audio.set_gain`, `audio.set_pan`, `audio.set_fades`, `audio.set_track_mix` |
| Subtitle | `subtitle.add_cue`, `subtitle.set_cue`, `subtitle.remove_cue`, `subtitle.import` |
| Title | `title.add`, `title.set_text`, `title.set_style` |
| Markers | `marker.add`, `marker.remove` |

Saving, undo/redo, inspection, and rendering are **not** document commands: they do not go through
`project.apply` and are not journaled as edits. They are protocol methods (`project.save`, `history.undo`,
`history.redo`, `inspect.*`, `job.*`) defined in [06 §4](06-agent-interface.md).

**Command shape rule:** commands name entities by ID and express intent (`move_clip`), not mechanism.

**Client-assigned IDs.** Creation commands (`track.add`, `asset.import`, `timeline.add_clip`, `effect.add`,
`title.add`, `timeline.add_transition`, `marker.add`) accept an optional `id` with the correct prefix
(e.g. `"tr_music"`). This lets one atomic batch create an entity and reference it in later commands. If
omitted, the engine assigns a ULID; either way the final IDs are returned in `resulting`. Duplicate IDs fail
with `duplicate_id`.
Ripple semantics are explicit: `trim_clip { ripple: "none" | "track" | "all" }`, default `none`. The GUI's
magnetic behaviors expand into explicit commands before sending, so human and agent edits are recorded
identically.

## 5. Undo, redo, journal

- During apply the engine derives `InverseCommand` values and keeps them in memory; `history.undo` applies
  them in reverse as a new revision. Undo history is not persisted across sessions; the last 5,000 batches
  are available as an edit-history panel.
- The **journal** records every committed batch as its **resolved forward commands** (all IDs and times
  normalized), so replay is deterministic. Undo and redo are journaled as the batches they produce.
- The journal lives in a sidecar directory next to the container, never inside the ZIP (a ZIP cannot be
  appended safely). Record format, write/fsync policy, recovery algorithm, and data-loss guarantees are
  specified once, in [08 §4](08-performance-reliability.md).

## 6. File format: `.arcline`

A ZIP container (stored for media, deflated for JSON):

```
project.arcline
├── manifest.json     schema_version, app_version, ids, checksums, settings
├── project.json      canonical document (sorted keys, no whitespace)
├── media/            embedded media (collect mode only)
├── assets/           fonts, LUTs, still images used by the edit
├── cache/            thumbnails, waveforms, proxies (regenerable, optional)
└── previews/         poster frame (optional)
```

Rules:

- **Canonical JSON** so an unchanged project hashes identically — "did this command change anything?" is
  a cheap comparison.
- **Checksums** per entry in `manifest.json`; corrupt caches are discarded, a corrupt `project.json`
  triggers the recovery UI.
- **Save modes:** *link* (default; media stays in place) and *collect* (media copied into `media/`, relative
  paths only).
- **Autosave and journal** live in the sidecar directory `<name>.arcline.autosave/`; explicit save replaces
  the container atomically. Procedures are in [08 §4](08-performance-reliability.md).
- Target document size ≤ 5 MB for a 2-hour, 2,000-clip project.

## 7. Schema versioning and migration

- `schema_version` is monotonic; loaders accept `<= CURRENT` and migrate forward-only through pure
  functions `migrate_vN_to_vN1(Value) -> Value`.
- Every schema change requires a migration, a fixture in `fixtures/schema/`, and an entry in
  [schema/CHANGELOG.md](schema/CHANGELOG.md).
- Protocol version and schema version are independent: commands are validated against the live schema, not
  serialized against it.

## 8. Worked example: minimal project document

```jsonc
{
  "schema_version": 7,
  "meta": { "id": "prj_01J8Z…", "name": "Demo", "created": "2026-11-24T09:40:00Z" },
  "settings": { "sequence": "sq_01J8Z…", "autosave": {"every": 30, "unit": "commands"} },
  "assets": {
    "as_01J8Z": { "kind": "video", "path": "media/a.mov", "probe": { /* … */ } }
  },
  "sequences": {
    "sq_01J8Z": {
      "name": "Main",
      "timebase": { "frame_duration": { "num": 1, "den": 30 } },
      "resolution": { "w": 1920, "h": 1080 },
      "tracks": [
        { "id": "tr_V1", "kind": "video", "index": 0, "clips": [
          { "id": "cl_01J8Z", "asset": "as_01J8Z",
            "start": { "num": 0, "den": 1 }, "source_in": { "num": 0, "den": 1 },
            "duration": { "num": 5, "den": 1 },
            "speed": { "num": 1, "den": 1 }, "effects": [] }
        ] },
        { "id": "tr_A1", "kind": "audio", "index": 0, "clips": [] }
      ]
    }
  }
}
```

## 9. Invariants (enforced centrally, tested by property tests)

1. Every referenced ID exists, except explicit unresolved states (offline asset, missing effect).
2. Clips on a track are sorted and non-overlapping.
3. `duration > 0`; `start >= 0`; `timeline_duration × speed == source_range` exactly.
4. Keyframe times strictly increase within a parameter curve.
5. Track indices are dense and unique within a sequence.
6. Revision strictly increases per session; a batch maps `revision n → n+1`.
7. Applying a batch and its inverse restores a document whose canonical JSON hash is identical.

Invariant 7 is the strongest statement and is checked by randomized property tests over command sequences;
it is where most document-model defects will be caught.
