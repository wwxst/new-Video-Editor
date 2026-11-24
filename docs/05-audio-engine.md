# 05 — Audio Engine

Audio is where amateurs' editors feel broken: drift, clicks, latency that varies by device, and preview
that does not match export. The rules below exist to make all three impossible.

## 1. Goals and hard constraints

| Requirement | Target |
|---|---|
| A/V sync | ≤ 1 frame drift over 2 h playback (NTSC included) |
| Sample accuracy | every clip boundary lands on an exact sample; fades sample-accurate |
| Preview vs export parity | same DSP graph, same parameters, same result (asserted by hash in CI) |
| Click-free editing | no zipper noise, no discontinuities at edits or seeks |
| Loudness | EBU R128 measurement, true-peak limiting, -14/-16/-23 LUFS targets |
| Real-time safety | audio callback: no allocation, no locking, no syscalls |

## 2. Sample model

- Internal format: **f32 planar, 48 kHz** (per-sequence configurable 44.1/48/96 kHz).
- Sources are resampled once at the decode boundary (high-quality polyphase, `rubato`-class), never inside
  the mixing graph.
- The timeline is quantized to samples: `sample_index = round(t_seconds × sr)` computed in exact rational
  arithmetic, so a frame boundary at 1001/30000 s maps deterministically to a sample boundary.
- Channel layouts: mono, stereo, 5.1 (LT/RT downmix policy documented), with per-clip channel maps.

## 3. Graph

```
 Sources (clip audio readers / generator / mic monitor)
    │  per-clip: gain, pan, fades, speed-aware resampling
    ▼
 Track strips:  EQ (4-band) → compressor (opt) → track gain/pan
    ▼
 Buses:  dialogue / music / SFX (optional grouping)
    ▼
 Master:  bus EQ → loudness stage (opt) → limiter → meter
    ▼
 Device
```

Same graph runs in two modes:

1. **Real-time**: pulled by the device callback in blocks of 64–512 frames (device-dependent), with a
   lock-free SPSC ring decoupling the callback from a mixing worker.
2. **Offline (export)**: pulled in a single thread, block size 1024, block-by-block, with the same DSP
   code. Any node that cannot run offline (e.g. live input monitoring) is bypassed by mode flag, not by a
   separate implementation.

Parity is enforced structurally: `arc-audio::Graph::render_offline(range)` and the real-time path call the
same `process(block)` methods. CI renders a mixed project offline and through the real-time path (captured
with a null device) and compares hashes at sample precision.

## 4. Playback: the clock

The audio device clock is the **master clock** for playback ([03 §6](03-render-engine.md)).

- On start, the audio engine reports `(stream_time, sample_position)` pairs; the video scheduler converts
  sample position to presentation time with latency compensation for the output buffer.
- Device changes (unplugging headphones, switching to Bluetooth) are detected and handled by rebuilding the
  stream and re-anchoring the clock with an offset; playback continues, no restart from zero.
- Bluetooth latency is compensated by measuring reported latency where the platform provides it, and by an
  optional user calibration (tap-to-sync) where it does not. Drift is unavoidable with Bluetooth; the policy
  is honest: audio is master, video compensates, and the UI shows a "latency-compensated" indicator.

## 5. Editing interactions

- **Scrub audio** plays short grains (30–60 ms) pitch-preserved at low volume, unless disabled. Grain
  scheduling lives in the audio worker, never in the callback.
- **Seek** invalidates the ring buffer and pre-fills it before resuming; the callback only ever sees
  contiguous, pre-computed data — hence no clicks.
- **Clip edits during playback** (drag, trim, mute) are applied at the next block boundary from an
  atomically swapped parameter block; no partial-block discontinuities.
- **Playback speed** (0.25×–4×) uses a time-stretch engine (WSOLA-class) for preview only; export speed
  changes are source-mapping (sample-accurate), not the preview stretcher.

## 6. DSP nodes (v1)

Gain, pan (constant-power), fades (linear, equal-power, S-curve), 4-band EQ, compression, limiter,
loudness normalization (EBU R128 / ITU-R BS.1770-4), mute/solo (solo logic at track level), channel matrix
down/up-mix. De-noise, de-ess, reverb are P5+ candidates, not v1.

All nodes are sample-accurate and parameter-automatable with the same keyframe model as video
([02 §3.3](02-domain-model-and-project-format.md)), evaluated per block with linear interpolation of
control values at sample granularity where needed (gain ramps).

## 7. Metering and loudness

- Metering: peak, RMS, short-term LUFS, true-peak (4× oversampled), goniometer (stereo correlation).
  Computed on the audio worker; the UI polls at 30 Hz. Meters never affect the callback.
- Analysis pass for export: integrated loudness, LRA, true-peak max over the whole program.
- Targets: broadcast -23 LUFS (EBU), streaming -14 LUFS (Spotify/YouTube convention), podcast -16 LUFS.
  The exporter writes measured values into `export_report.json` and can fail the job on a target miss
  (`strict_loudness: true`), which matters enormously to automated pipelines.

## 8. VST3 (P5)

VST3 hosting is isolated in a **plugin host subprocess** (never in-process, never on the audio callback's
thread): the host renders the plugin in blocks and returns audio through shared memory, adding a
configurable latency that is reported to the graph and compensated. Crash or hang ⇒ plugin bypassed, session
unharmed. *Not in v1*: instrument plugins, side-chain routing beyond stereo, MIDI editing.

## 9. Testing

- **Sample-exact golden audio**: 40 fixtures (fades, EQ sweeps, limiter, loudness targets, speed ramps).
- **Parity test**: real-time capture vs offline render hash equality.
- **Soak**: 8 h continuous playback with random edits under AddressSanitizer and a callback-violation
  detector (allocation hooks, lock detector).
- **Device matrix CI**: null device + one real device per OS in the release gate.
