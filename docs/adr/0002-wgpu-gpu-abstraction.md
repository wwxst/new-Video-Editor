# ADR-0002 — `wgpu` as the GPU abstraction

**Status:** Accepted · 2026-11-24

## Context

The editor must render on Vulkan (Windows/Linux), Metal (macOS), and D3D12 (Windows) with real-time
compositing, compute-heavy effects, and deterministic output for tests. Maintaining three native graphics
backends triples the surface area for the most defect-prone subsystem. Hardware video decode and encode
must interoperate with the same device.

## Decision

Use **`wgpu`** as the single GPU abstraction, wrapped in `arc-render::Device` which owns capability tiers,
validation policy, pipeline caching, and device-loss recovery. Shaders are WGSL (with optional SPIR-V
escape hatch if a backend gap demands it).

## Consequences

- One code path across platforms; backend differences funnel through a small, testable layer.
- WebGPU semantics give us a modern, validated API (explicit bind groups, no global state).
- Compute shaders are first-class — effects like blur/ranges/keying are written as compute where it wins.
- **Gaps are real and must be planned for**: `wgpu` does not expose hardware-video-decode interop textures on
  all backends. Mitigation: decoded frames arrive via shared memory, upload YUV planes, and convert on the
  GPU ([04 §3](04-media-pipeline.md)). On Windows/macOS a zero-copy external-texture path is added behind
  the device abstraction where the backend permits; it is an optimization, not a correctness requirement.
- Backend bugs exist; the device layer can be pointed at a different backend per platform via a signed
  config if a driver regression appears.
- Validation overhead is removed in release builds (`wgpu` has no debug validation in release), but we keep
  a validation-enabled debug build in CI.

## Alternatives considered

- **Three native backends (Vulkan/Metal/D3D12)**: maximum control and access to video interop, but ~3× the
  work and three sets of bugs. Revisit only if a v1 blocker appears.
- **OpenGL**: deprecated on macOS; inconsistent compute; rejected.
- **bgfx / Dawn**: C++ APIs and narrower feature sets; integration cost with Rust outweighs benefits.
- **Vulkan-only (+ MoltenVK)**: workable, but loses native Metal quality on Apple silicon and adds a
  translation layer on the most constrained platform.
