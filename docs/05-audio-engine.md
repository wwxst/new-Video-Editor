# 05 — Audio Engine

Audio is where editors feel broken: drift, clicks, device-dependent latency, and previews that disagree
with exports. The rules below exist to make those impossible, with a deliberately small v1 surface.

## 1. Goals and constraints

| Requirement | Target |
|---|---|
| A/V sync | ≤ 1 frame drift over 10-minute playback (NTSC included) |
| Sample accuracy | clip boundaries and fades land on exact samples |
| Preview vs export parity | same DSP graph, same parameters, hash-equal in CI |
| Click-free editing | no discontinuities at edits, seeks, or parameter changes |
| Real-time safety | audio callback: no allocation, no locking, no syscalls |

## 2. Sample model

- Internal format: **f32 planar, 48 kHz** (per-sequence configurable 44.1/48/96 kHz).
- Sources are resampled once at the decode boundary; never inside the mixing graph.
- `sample_index = round(t_seconds × sample_rate)` is computed in exact rational arithmetic, so frame
  boundaries map deterministically to sample boundaries ([02 §2](02-domain-model-and-project-format.md)).
- Channel layouts: mono, stereo, 5.1 downmix policy documented.

## 3. Graph (v1)

```
Clip readers (audio from arc-media, or generator)
   │  per-clip: gain, pan, fades, speed-aware resampling
   ▼
Track strips: gain, pan, mute/solo
   ▼
Master: gain, meters
   ▼
Device
```

This is the whole v1 graph. Equalization, compression, limiting, and loudness normalization are deferred to
post-1.0 ([§6](#6-dsp-and-analysis)); shipping a correct small graph beats a wide half-tested one.

## 4. Playback and the clock

The audio device clock is the **master clock** ([03 §6](03-render-engine.md)).

- The audio engine reports `(stream_time, sample_position)`; the video scheduler converts sample position
  to presentation time with output-latency compensation.
- Device changes (unplugging headphones, switching output) are detected, the stream is rebuilt, and the
  clock is re-anchored with an offset; playback continues instead of restarting.
- Bluetooth latency is compensated where the platform reports it, with an optional tap-to-sync calibration.
  Drift is unavoidable on Bluetooth; audio remains master and the UI shows a latency-compensated indicator.

## 5. Editing interactions

- **Scrub audio** plays 30–60 ms pitch-preserved grains at low volume (toggleable). Scheduling lives in the
  audio worker, never in the callback.
- **Seek** invalidates the ring and pre-fills it before resuming; the callback only sees contiguous,
  pre-computed data — hence no clicks.
- **Edits during playback** apply at the next block boundary from an atomically swapped parameter block.
- **Playback speed** (0.25×–4×) uses a WSOLA-class stretcher for preview only; export speed changes are
  source-mapping, not the stretcher.

## 6. DSP and analysis

**v1 (shipping):**

- Gain, constant-power pan, fades (linear, equal-power, S-curve), mute/solo with proper solo logic.
- Meters: peak and RMS, computed on the audio worker; the UI polls at 30 Hz.
- Offline mix for export using the identical graph.

**Deferred:**

- 4-band EQ, compressor, limiter, EBU R128 loudness normalization and true-peak limiting (post-1.0; v1.1 is the macOS port only — see [ADR-0011](adr/0011-windows-first.md)).
- VST3 hosting (post-1.0, subprocess-isolated). Not part of the v1 architecture.

## 7. Preview/export parity

Parity is structural, not tested-in:

- `Graph::render_offline(range)` and the real-time path call the same `process(block)` methods.
- CI renders a mixed fixture project offline and through the real-time path (null device) and compares
  hashes at sample precision.
- Any node that cannot run offline is bypassed by an explicit mode flag, never by a separate
  implementation.

## 8. Testing

- **Sample-exact golden audio**: fades, pan laws, gain ramps, solo logic, speed mapping.
- **Parity test**: real-time capture vs offline render hash equality on every PR.
- **Soak**: multi-hour playback with random edits under allocation/lock detection on the callback.
- **Device matrix**: null device in CI; one real device per shipping OS before release.
