# 02 — Domain Model & Project Format

Everything here is normative: the Rust types, the JSON wire form, and the on-disk form are the same model
in three encodings. The file format is a versioned projection of the in-memory document.

## 1. Entity overview

```
Project
├── meta:            name, created, modified, app_version, schema_version
├── settings:        timebase, resolution, color space, sample rate, cache policy
├── assets:          AssetId → Asset            (content-addressed references)
├── sequences:       SequenceId → Sequence      (v1: exactly one active sequence, model supports many)
│   └── tracks:      Track[]                    (video / audio / subtitle, ordered bottom→top)
│       └── clips:   Clip[]
│           ├── source: SourceRef               (asset + in/out, or nested sequence, or generator)
│           ├── timing: start, duration, speed, freeze
│           ├── transform: position, scale, rotation, opacity, anchor
│           ├── effects: EffectInstance[]        (ordered chain, each with keyframed params)
│           ├── audio:   gain, pan, fades, channel map
│           └── meta:    label, color, locked, enabled
├── markers:         Marker[]                   (sequence- or asset-scoped)
└── subtitles:       SubtitleDoc[]              (one per language track)
```

All entity IDs are opaque, sortable, time-ordered identifiers (ULID-style: 128-bit, 48-bit millisecond
timestamp + 80 bits randomness), rendered as `cl_01J8ZK…`, `as_…`, `sq_…`, `tr_…`, `fx_…`. Rationale:
sortable by creation, collision-free across agents collaborating on the same session, and debuggable in
logs. They are *not* meaningful to clients except for equality.

## 2. Time

**The single most important type in the system.**

```rust
/// Exact rational seconds. Normalized: den > 0, gcd(num, den) == 1.
pub struct Time { num: i64, den: u32 }   // e.g. 1001/30000 = one NTSC frame

/// A project timebase is a frame duration as a rational.
pub struct Timebase { frame_duration: Time }   // 1/30 or 1001/30000

/// Convert to frame index with an explicit, documented rounding mode.
pub fn frame_at(time: Time, tb: Timebase, mode: Round) -> i64;
pub fn time_of_frame(frame: i64, tb: Timebase) -> Time;
```

Rules:

1. **No floats in the timing path.** Ever. `f64` seconds appears only at the UI boundary for display and
   in effect parameters, never in clip boundaries, never in serialization of timing.
2. **Frame-grained by default.** Clip boundaries must land on the project timebase (validation error
   `off_frame_boundary` otherwise). Audio-only clip boundaries are additionally validated against the
   sample rate, permitting sub-frame positions when `allow_subframe: true` is set on the clip — this exists
   for audio-synced edits, not for video.
3. **Speed changes are rational.** `speed: {num, den}` with `num/den ∈ (0, 16]` — no 1.0000001 drift.
   `duration` is always *timeline* duration; source in/out are *source* time, and the invariant
   `timeline_duration × speed = source_range` is checked in exact arithmetic (i128 intermediates).
4. **NTSC drop-frame** is a *display* concern (`HH:MM:SS;FF`), never a storage concern.
5. **Time is monotone and non-negative**; negative times are a validation error, except keyframe times
   which may be negative relative to a clip start (pre-roll handles).

Operations are checked: overflow yields `TimeOverflow`, and total project length is capped at 24 h per
sequence (a hard sanity limit, configurable by build).

## 3. Entity detail

### 3.1 Asset

```jsonc
{
  "id": "as_01J8Z…",
  "kind": "video" | "audio" | "image" | "sequence" | "generator",
  "path": "media/interview_a.mov",        // relative to project root; may be missing
  "content_hash": "blake3:…",             // of first 4 MB + size + mtime-at-import (fast fingerprint)
  "probe": {                              // cached, nullable, refreshed async
    "duration": {"num": 7502, "den": 1001},
    "streams": [ { "index": 0, "type": "video", "codec": "h264",
                   "width": 3840, "height": 2160, "fps": {"num": 30000, "den": 1001},
                   "pix_fmt": "yuv420p10le", "color": {"primaries": "bt709", "transfer": "bt709"} } ]
  },
  "proxy": "cache/proxies/as_01J8Z…/half.mp4",   // optional
  "waveform": "cache/wave/as_01J8Z….peak",       // optional peak pyramid
  "color_source": "auto" | "bt709" | "bt2020pq" | "custom:{lut}"
}
```

Assets are referenced by relative path inside the container where possible, so a project folder is
portable. A missing file does not fail the project open; it produces an **offline asset** that renders as
a placeholder and is relinked by hash on demand.

### 3.2 Sequence and tracks

```jsonc
{
  "id": "sq_01J8Z…",
  "name": "Main",
  "timebase": {"frame_duration": {"num": 1001, "den": 30000}},
  "resolution": {"w": 1920, "h": 1080},
  "working_space": "rec709" | "rec2020" | "acescg",
  "sample_rate": 48000,
  "tracks": [
    { "id": "tr_V1", "kind": "video", "index": 0, "name": "V1",
      "enabled": true, "locked": false, "blend": "normal", "opacity": 1.0,
      "clips": [ /* Clip[] sorted by start time, non-overlapping invariant */ ] }
  ]
}
```

Track invariants (validation, not convention):

- Clips on a track never overlap in time. (Overlapping media is expressed with multiple tracks.)
- `index` ordering is explicit and stable; z-order is by index, higher index composites on top.
- Deleting a track deletes its clips atomically; commands address clips by ID, never by index.

### 3.3 Clip

```jsonc
{
  "id": "cl_01J8Z…",
  "asset": "as_01J8Z…",
  "start": {"num": 4, "den": 1},              // timeline position, 4.000 s
  "source_in": {"num": 110, "den": 1},        // source offset
  "duration": {"num": 130, "den": 30},        // 4.333… s timeline
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
          {"t": {"num": 0, "den": 1},       "v": 1.00, "interp": "linear"},
          {"t": {"num": 2, "den": 1},       "v": 1.10, "interp": "bezier", "ease": [0.4,0,0.2,1]}
        ]}
      } }
  ],
  "audio": {
    "gain_db": -3.0, "pan": 0.0,
    "fade_in":  {"num": 1, "den": 4}, "fade_out": {"num": 1, "den": 2},
    "channel_map": "stereo"
  },
  "label": "A-roll", "color": "#ff8a3d", "enabled": true, "locked": false
}
```

**Parameter values** are one of two forms, uniformly, for every effect and transform property:

```jsonc
{"static": 0.5}                      // constant
{"curve": [ Keyframe, … ]}           // keyframed, sorted, unique times
```

A keyframe is `{t: Time (relative to clip start), v: f32|Vec|bool|string, interp: "linear"|"hold"|"bezier"|"ease", ease?: cubic-bezier}`.
Making *everything* keyframable with one representation means the inspector, the MCP tools, and the
evaluator each have exactly one code path. Evaluation is pure: `eval(param, local_time) -> value`.

### 3.4 Effects

An effect is defined by a manifest — the same manifest an agent reads to know what parameters exist:

```jsonc
{
  "id": "core.color.correct",
  "version": 3,
  "display": "Color Correction",
  "category": ["color"],
  "gpu": { "shader": "builtin:color_correct", "passes": 1, "precision": "fp16_ok" },
  "params": [
    { "key": "exposure", "type": "float", "range": [-5, 5], "default": 0.0, "unit": "stops" },
    { "key": "contrast", "type": "float", "range": [0.2, 2.0], "default": 1.0 },
    { "key": "saturation", "type": "float", "range": [0.0, 2.0], "default": 1.0 },
    { "key": "lut", "type": "asset_ref", "optional": true }
  ]
}
```

Effects are versioned. A project stores `effect: "core.color.correct"` and the instance's params; on load,
the current manifest version's migration function upgrades old params (e.g. `brightness/contrast` →
`exposure/contrast` in v3). Unresolvable effect IDs load as a **missing-effect placeholder** that passes
through and is reported by `project.describe`, never silently dropped.

### 3.5 Transitions

A transition is a relationship between two adjacent clips, not a third clip:

```jsonc
{ "id": "tx_…", "kind": "cross_dissolve", "a": "cl_…", "b": "cl_…",
  "duration": {"num": 1, "den": 2}, "alignment": "center" }
```

Stored on the track alongside clips. The compositor evaluates transitions in the overlap window they
create logically; the underlying clip timing is not silently rewritten (an important agent-friendly
property: adding a transition does not move clips, unless the command explicitly says `ripple: true`).

### 3.6 Audio (model side)

Gain, pan, fades, channel maps as above; per-track and master bus gain/EQ/compressor live on `Track` and a
`MasterBus` node. VST3 insert slots are a later schema extension (`schema_version` bump), reserved now.

### 3.7 Subtitles

A subtitle track is a `Track { kind: "subtitle" }` whose clips reference a `SubtitleDoc` — a list of cues
with text, in/out, and an optional style reference. This reuses every timeline operation (move, trim,
split) instead of inventing a parallel editing model. Burn-in is a render pass over the cue list; export to
sidecar SRT/VTT/ASS is a serializer.

## 4. Commands

The complete v1 command set, grouped. Every command is a JSON object with `op` and typed fields; every
command has a precise, documented effect and a validation rule. This list *is* the editing surface for
both GUI and agents.

| Group | Commands |
|---|---|
| Project | `project.create`, `project.rename`, `project.set_settings`, `project.save`, `project.close` |
| Asset | `asset.import`, `asset.relink`, `asset.remove`, `asset.set_color_source`, `asset.generate_proxy` |
| Track | `track.add`, `track.remove`, `track.reorder`, `track.set_properties` |
| Clip | `timeline.add_clip`, `timeline.remove_clip`, `timeline.move_clip`, `timeline.trim_clip`, `timeline.split_clip`, `timeline.set_speed`, `timeline.set_transform`, `timeline.set_enabled`, `timeline.set_label` |
| Transition | `timeline.add_transition`, `timeline.remove_transition` |
| Effect | `effect.add`, `effect.remove`, `effect.reorder`, `effect.set_params`, `effect.set_keyframes`, `effect.set_enabled` |
| Audio | `audio.set_gain`, `audio.set_pan`, `audio.set_fades`, `audio.set_track_mix` |
| Subtitle | `subtitle.add_cue`, `subtitle.set_cue`, `subtitle.remove_cue`, `subtitle.import` |
| Text/title | `title.add`, `title.set_text`, `title.set_style` |
| Markers | `marker.add`, `marker.remove` |
| History | `history.undo`, `history.redo`, `history.jump` |
| Jobs | `render.submit`, `render.cancel` (not document commands; see [06 §6](06-agent-interface.md)) |

**Command shape rule:** a command names entities by ID and expresses intent (`move_clip`), not mechanism.
Two consequences: commands are composable into batches, and the same intent can be replayed against a
diverged document with predictable results (or a clean `not_found` error).

**Ripple semantics are explicit, never implicit.** `trim_clip { ripple: "none" | "track" | "all" }`. The
default is `none`; the GUI's magnetic behaviors are UI presets that expand into explicit commands before
sending. An agent's edit and a human's edit are therefore recorded identically.

## 5. Undo, redo, journal

- The engine keeps `Vec<InverseCommand>` derived during apply; undo is applying inverses in reverse.
- `history.jump(n)` rewinds by replaying inverses to a target revision — the same code path as undo, so
  there is no "undo drift."
- The **journal** (`.arcline/journal.bin`, append-only, length-prefixed records with CRC) records every
  applied batch with its inverse. A crash strategy: replay journal on last-good snapshot. Journal is
  compacted into the snapshot on save.
- Undo history is *not* persisted across sessions by default (professional editors generally do not);
  the journal exists for recovery, and its last 5,000 batches are available as an "edit history" panel.

## 6. File format: `.arcline`

A ZIP container (stored, not deflated, for `media/`; deflated for JSON) with this layout:

```
project.arcline
├── manifest.json          schema_version, app_version, ids, checksums, settings
├── project.json           the document, canonical form (sorted keys, no whitespace)
├── journal/               last journal segments (optional, for recovery)
├── media/                 embedded media (optional; "link" vs "collect" mode)
├── assets/                fonts, LUTs, still images that are part of the edit
├── cache/                 thumbnails, waveforms, proxies (optional, regenerable)
└── previews/              poster frame, contact sheet (optional)
```

Design rules:

- **Canonical JSON** (serde with deterministic key order) so that an unchanged project hashes identically.
  This is what makes "did this command actually change anything?" a cheap comparison rather than a diff.
- **Checksums** for every entry in `manifest.json`; on open, a background verifier reports corruption
  without blocking (a corrupt cache entry is discarded; a corrupt `project.json` triggers the recovery UI).
- **Two save modes:** *link* (default; media stays in place, absolute paths recorded) and *collect* (media
  copied into `media/`, relative paths only — what you send to a client).
- **Autosave** writes `project.json` + journal to a sibling `.arcline.autosave/` directory; the real file is
  replaced atomically (write temp → fsync → rename) on explicit save.
- Project size target: manifest + document ≤ 5 MB for a 2-hour, 2,000-clip project. If exceeded, the
  document is split into a paged form (`project.json` + `pages/`), transparently.

## 7. Schema versioning and migration

- `schema_version` is a monotonically increasing integer; the loader accepts `<= CURRENT`.
- Migrations are pure functions `fn migrate_vN_to_vN1(Value) -> Value`, chained, each with its own test
  fixture. Migration is **forward-only**; opening a future-schema project yields a clear error with the
  required app version.
- Every schema change requires: a migration, a fixture in `arc-testkit/fixtures/schema/`, and an entry in
  `docs/schema/CHANGELOG.md`.
- The *protocol* version and the *schema* version are independent: an old CLI can speak protocol 1.0 to a
  new daemon that has schema 9, because commands are validated against the live schema, not serialized
  against it.

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

1. Every ID referenced exists (except explicit unresolved states: offline asset, missing effect, dangling
   transition endpoint → validation error).
2. Clips on a track are sorted and non-overlapping.
3. `duration > 0`; `start >= 0`; `timeline_duration × speed == source_range` exactly.
4. Keyframe times are strictly increasing within a parameter curve.
5. Track indices are dense and unique within a sequence.
6. Revision is strictly increasing per session; a command batch maps `revision n → n+1`.
7. Applying a command batch and its inverse restores a document whose canonical JSON hash is identical.

Invariant 7 is the strongest statement and is checked by a randomized property test over command
sequences — it is where most document-model bugs will be caught.
