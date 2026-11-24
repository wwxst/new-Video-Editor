# ADR-0009 — Same-machine determinism, tolerance goldens

**Status:** Accepted, revised 2026-11-24 after architecture review.

## Context

Agents and pipelines need reproducible renders; golden tests need a stable reference. GPU floating point
differs across vendors and drivers, so a cross-device bit-exact promise would be false. The original
proposal defined a three-tier contract with a CPU reference-render mode — correct, but more machinery than
a solo project needs in v1.

## Decision

1. **Same machine, same driver, same renderer version ⇒ bit-exact output.** Rendering twice yields identical
   frames; CI asserts this.
2. **Across machines ⇒ tolerance-bounded.** Golden tests compare with PSNR ≥ 45 dB; effects with
   non-associative math are documented as excluded.
3. **Attribution.** Every export report records renderer version and shader-cache hash so a result can be
   explained.
4. No reference-render mode and no cross-device bit-exactness claim in v1.

## Consequences

- The contract is honest and testable with a software adapter in CI.
- Goldens are regenerated deliberately and reviewed when rendering changes; a shader change is a renderer
  version bump, not a silent difference.
- Users can reproduce their own renders; support can diagnose differing outputs from the report.
- Some effects are excluded from cross-machine comparison; the exclusion is documented per effect.

## Alternatives considered

- **Bit-exact everywhere**: impossible; would fail on real hardware.
- **No guarantee**: unacceptable for agent verification and goldens.
- **CPU-only rendering**: deterministic but far too slow.
- **Full three-tier contract with reference mode** (original decision): unnecessary for v1; the same-machine
  guarantee and tolerance goldens cover the practical need. Reference mode can be added if disputes arise.
