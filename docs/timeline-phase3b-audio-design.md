# Timeline Phase 3B — Arcline Audio Envelope

**Status:** implementation design, 2026-11-28
**Scope:** volume, fade-in, fade-out, hover/selected/muted/locked/disabled states, embedded video audio.
**Non-goals:** keyframe automation UI, audio scrubbing, real-time cpal playback, trim/move/split, Phase 3C.

## 1. Design direction

The reference software was used only to answer interaction questions: when controls appear, which
end is fade-in, whether video audio is a separate track, and how a value is fed back. No competitor
screenshot, icon, colour, node shape or layout is copied here.

Arcline's own envelope language:

- **Volume envelope** — a thin brand-blue curve with a soft dark halo so it remains legible over a
  waveform. It is a *curve*, not a borrowed white rule.
- **Volume node** — a 12 × 12 rounded square, brand-blue fill with a white keyline. A square says
  "one scalar parameter"; it is deliberately unlike the vertical trim bars and the fade diamonds.
- **Fade nodes** — two 10 × 10 diamonds in the waveform teal (`WAVE_EMBEDDED`) with a white keyline.
  A diamond says "a duration endpoint", not "a level".
- **Fade regions** — translucent brand-blue triangles under the ramps, at 18/255 alpha.
- **Readout** — a compact 48 × 13 chip at the lower-right of the audio region, monospace dB. Values
  also appear in the Inspector; the chip is live feedback, not permanent chrome.
- **Selection** — remains the Phase 3A white 1.5 pt clip outline. The envelope never replaces it.

The visual relationship is intentional:

```
volume level  ──────■────────      rounded square = scalar level
fade duration ◇─────┘       ◇     diamonds       = duration endpoints
```

## 2. State model

Audio controls are an **overlay**, not a single enum. The base clip state and the envelope state
compose.

| State | Clip / envelope |
|---|---|
| Normal | No envelope, no nodes, no readout |
| Hover | Envelope + volume node + fade diamonds + readout |
| Selected | White clip outline only; Inspector shows values |
| Selected + Hover | White outline **and** envelope/controls |
| Locked | Lock badge/veil, no controls; not selectable |
| Muted | Audio region dimmed + mute badge, no controls; selection/Inspector remain valid |
| Disabled | Full clip veil, no controls; not selectable |

Selected without hover intentionally does not grow a control cockpit. The Inspector carries the
numbers, and the hover gesture carries the handles. This mirrors the interaction need without
borrowing the reference's visual treatment.

## 3. Geometry and hit priority

All values are in logical points.

- Fade diamonds sit **8 pt inside** the real clip edges when the fade is zero, and at the fade
  boundary when it is non-zero. They never sit on a viewport edge.
- Volume node is at the audio-region centre.
- Volume line hit band is ±4 pt; fade diamond hit area is the 10 pt node expanded by 2 pt; volume
  node is expanded by 2 pt.
- Hit priority, highest first:

```
real trim edge (6/4/0 pt)
  > fade diamonds
  > volume node
  > volume line
  > audio region
  > clip body
```

The priority is tested directly. A locked or muted clip never upgrades an `Audio` hit into an
envelope hit. A clip partly outside the viewport keeps its real geometry; the painter clips it and
does not invent a handle at the viewport boundary.

## 4. Parameters and feedback

The single parameter type is `arc_core::model::ClipAudio`:

- `gain_db: f64` — the timeline maps vertical drag to `[-60, +12] dB`.
- `fade_in: Time`, `fade_out: Time` — horizontal diamond drag; clamped to `[0, clip duration]`.
- `pan` remains in the engine model but is not exposed in 3B.

The same `ClipAudio` is read by:

- the timeline envelope and waveform scaling;
- the Inspector;
- `arc_audio::Mixer` (gain + fades, existing engine path);
- `arc-cli` export (the same mixer, existing path).

The waveform is scaled by gain and the fade envelope. It is never clipped or shortened by a fade;
fades only change the vertical envelope.

## 5. Interaction details

- Volume drag: vertical, 0.8 dB per point, clamped. The node and line move together.
- Fade-in drag: right increases duration; left decreases to zero.
- Fade-out drag: left increases duration; right decreases to zero.
- Drag starts only from an envelope control, so it cannot steal a body click or a trim gesture.
- Clicking an envelope control selects the clip without changing its value.
- Escape cancels an active audio gesture and restores its pre-drag value.
- Ctrl/Cmd+Z and Ctrl/Cmd+Shift+Z (or Ctrl+Y) undo/redo the local audio gesture stack.
- Cursor: `ResizeVertical` over the line/node, `ResizeHorizontal` over fade diamonds, `NotAllowed`
  on locked clips, `Default` on the body.

## 6. Inspector

The right panel is no longer a placeholder. For the primary audio-capable clip it shows:

```
piano_theme.wav
VOLUME       +0.0 dB
FADE IN       0.0 s
FADE OUT      0.0 s
Hover the clip to edit its envelope
```

The values are read from the same `ClipAudio` each frame, so a drag updates the Inspector without a
second model or a repaint timer. A muted clip shows `MUTED · AUDIO CONTROLS HIDDEN`; a locked clip
shows `LOCKED`; no audio shows `No embedded audio`.

## 7. Embedded video audio

For a video with an audio stream, the same control model is drawn in the 16 pt bottom audio strip:

- the thumbnail/header are untouched;
- no extra track or clip is created;
- the selection outline still surrounds the whole video clip;
- the Inspector reads the video's `ClipAudio`;
- a video without audio produces no controls.

## 8. Performance

- No per-clip widget tree: one canvas interaction plus painter primitives.
- No decode on hover: the control geometry uses the waveform envelope already cached by `MediaHub`.
- No second waveform cache; the existing grid and cache are reused.
- No idle repaint or polling; repaint is requested only during a gesture.
- Lane virtualization, zoom and scroll mappings are unchanged.
- Envelope geometry is computed only for visible clips and only when hovered or dragging.

## 9. Undo/redo and document boundary

The 3B timeline is still the C4 presentation model, not the live document. Audio gestures therefore
use a small local `AudioHistory` keyed by `ClipRef`; each entry stores the previous and next
`ClipAudio`. This is intentionally separate from `Engine`'s project history so the mock cannot
silently fork the document.

The parameter type and the mixer/export path are already the engine's real types. When the timeline
is bound to a live `Project`, the same gestures should be emitted as `audio.set_gain` /
`audio.set_fades` commands and the local stack removed. This boundary is recorded in the Phase 3B
report rather than hidden.

## 10. Originality record

What was learned from references: hover reveals controls; left fade ramps up and right ramps down;
video audio lives in the video's bottom strip; values are shown in an inspector. Those are functional
facts.

What is Arcline-specific: the blue haloed curve, rounded-square level node, teal diamond duration
nodes, translucent fade wedges, the lower-right dB chip, the Inspector vocabulary, and the priority
model. No reference bitmap, icon, waveform, font, or panel composition is embedded in the product or
used as a pixel acceptance target.
