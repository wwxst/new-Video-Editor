# ADR-0004 — Exact rational time model

**Status:** Accepted · 2026-11-24

## Context

Video time is not decimal. NTSC rates are 30000/1001 and 24000/1001; a "30 fps" project and a "29.97 fps"
source must coexist; audio is quantized at 48,000 Hz; a two-hour timeline accumulates error fast if timing
uses floating point. Editors are judged on frame accuracy, and agents will produce thousands of edits that
must be validated exactly.

## Decision

Represent all timing as **exact rationals** (`Time { num: i64, den: u32 }`, normalized) with an explicit
project **timebase** (frame duration as a rational). All arithmetic uses checked `i128` intermediates.
Floating point is forbidden in timing paths; conversions to frames require an explicit rounding mode.
Serialization uses `{"num","den"}` objects.

## Consequences

- Zero drift: `sum of clip durations == sequence duration` is an exact assertion, testable in CI.
- NTSC rates are first-class; 23.976/29.97/59.94 and PAL rates need no special cases.
- Frame boundaries are reasoned about explicitly (`frame_at(t, tb, mode)`), which surfaces rounding policy
  in code review instead of hiding it.
- Costs: more verbose arithmetic than `f64`, overflow handling everywhere (`TimeOverflow`), and a learning
  curve for contributors used to seconds-as-float. Mitigated by a tiny, heavily tested API (`arc-core::time`) and
  ergonomic constructors (`T::secs(4)`, `tb.frame(120)`).
- Audio addresses samples by exact rational mapping to sample index, so sub-frame audio edits are possible
  when explicitly allowed ([02 §2](02-domain-model-and-project-format.md)).

## Alternatives considered

- **`f64` seconds**: simple, drifts, and makes equality/validation unreliable — rejected outright.
- **Integer ticks at a high fixed rate** (e.g., 1 ns): fits most rates but not exactly; mixing 30000/1001
  and 48000 Hz requires choosing a rate divisible by both frame durations and 48,000 — possible with a
  large common multiple, but the values become opaque and overflow risk grows. Rationals are exact by
  construction.
- **SMPTE timecode as the storage type**: loses sub-frame audio positions and forces rate interpretation at
  every boundary; kept as a display format only.
