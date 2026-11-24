# ADR-0009 — Tiered determinism contract

**Status:** Accepted · 2026-11-24

## Context

Agents and pipelines need to verify renders; golden tests need a stable reference. But GPU floating-point
results differ across vendors and drivers, and even within a vendor across driver versions. Promising
bit-exact output everywhere would be false, and a false promise in a verification product is worse than a
narrower true one.

## Decision

State and enforce a **tiered contract**:

1. **Bit-exact** for the same device class, driver version, and Arcline version (including shader cache
   hash). Same machine, same output, every time.
2. **Tolerance-bounded** across devices: PSNR ≥ 50 dB and max channel difference ≤ 2 LSB at 10-bit for
   pure-GPU paths; effects with non-associative iteration are documented as excluded.
3. **Reference mode** (`--render-mode=reference`): fp32, no fast-math, pinned kernels, CPU/lavapipe; the
   arbiter for golden tests and disputes.
4. **Attribution**: every export report records renderer version, shader cache hash, driver, adapter, and
   encoder so any result can be explained.

## Consequences

- Honest, testable, and useful: users can reproduce their own renders; CI can gate on goldens; disagreements
  have an arbiter.
- Some effects (e.g., certain temporal or iterative filters) are excluded from the cross-device tier; the
  exclusion list is maintained and surfaced in `effect_describe`.
- Reference mode is slow and is not offered as a normal export path — it exists for verification.
- Determinism becomes a compatibility surface: changing a shader or kernel is a *renderer version* bump,
  not a silent change, and old projects record the version they were created with.

## Alternatives considered

- **Claim bit-exact everywhere**: impossible; would fail on real hardware and destroy trust.
- **No determinism guarantee**: unacceptable for agent verification and golden testing.
- **CPU-only rendering**: deterministic but far too slow for real-time preview.
