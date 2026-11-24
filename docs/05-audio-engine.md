# 05 — Audio Engine

Audio is where editors feel broken: drift, clicks, device-dependent latency, and previews that disagree
with exports. The rules below exist to make those impossible, with a deliberately small v1 surface.

## 1. Goals and constraints

| Requirement | Target |
|---|---|
| A/V sync | ≤ 1 frame drift over 10-minute playback (NTSC included) |
| Sample accuracy | clip boundaries and fades land on exact samples |
| Preview vs export parity | master-bus output hash-equal between real-time (at 1× transport) and offline render, per §7 |
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
   │  per-clip: gain, pan, fades, clip-speed resampling (varispeed, §5)
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
- **Two different speed concepts, never mixed:**
  - **Clip speed** (`timeline.set_speed`, part of the document) changes the edit. Audio is produced by
    deterministic source-time mapping plus resampling (**varispeed**: pitch follows speed). This runs inside
    the mix graph and is identical in preview and export. Pitch-preserving clip speed is post-1.0.
  - **Transport speed** (J/K/L shuttle, 0.25×–4×) changes only how fast the user monitors the timeline. It is
    applied by a WSOLA-class stretcher in the **monitoring stage**, after the master bus, and never reaches
    an export. Scrub grains are also produced in the monitoring stage.

## 6. DSP and analysis

**v1 (shipping):**

- Gain, constant-power pan, fades (linear, equal-power, S-curve), mute/solo with proper solo logic.
- Meters: peak and RMS, computed on the audio worker; the UI polls at 30 Hz.
- Offline mix for export using the identical graph.

**Deferred:**

- 4-band EQ, compressor, limiter, EBU R128 loudness normalization and true-peak limiting (post-1.0; v1.1 is the macOS port only — see [ADR-0011](adr/0011-windows-first.md)).
- VST3 hosting (post-1.0, subprocess-isolated). Not part of the v1 architecture.

## 7. Preview/export parity

The signal chain has two parts with a defined **parity tap** between them:

```
clip readers → clip DSP (gain, pan, fades, clip-speed varispeed) → tracks → master bus
                                                                              │
                                                                     ── parity tap ──
                                                                              │
             monitoring stage (real-time only): transport WSOLA, scrub grains,
             monitor volume, device sample-rate conversion → device
```

Parity contract:

- **What is compared:** the master-bus output at the parity tap, as f32 samples, hashed.
- **When it must match:** real-time playback at **1× transport speed** versus `Graph::render_offline(range)`
  for the same range. Both call the same `process(block)` methods; clip speed is part of both.
- **What is excluded:** everything in the monitoring stage (transport WSOLA, scrub, monitor volume, device
  resampling). These are real-time-only by definition and never appear in an export, so they are not hashed.
- **Block alignment:** the offline renderer uses the same block boundaries relative to the range start as
  the real-time path in the test (fixed block size, null device), so parameter-change timing is identical.
- Any node that cannot run offline is bypassed by an explicit mode flag, never by a separate implementation.

CI renders a mixed fixture project (including clips at 0.5× and 2× clip speed) offline and through the
real-time path on a null device at 1× transport, and compares the hashes. The transport stretcher has its own
tests (§8) without a hash requirement.

## 8. Testing

- **Sample-exact golden audio**: fades, pan laws, gain ramps, solo logic, speed mapping.
- **Parity test**: real-time capture at the parity tap (1× transport) vs offline render, hash equality on
  every PR (§7).
- **Transport stretcher**: duration accuracy and no discontinuities at 0.25×–4× (listening tests plus
  automated click detection); no hash comparison.
- **Soak**: multi-hour playback with random edits under allocation/lock detection on the callback.
- **Device matrix**: null device in CI; one real device per shipping OS before release.
