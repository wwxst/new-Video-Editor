# ADR-0002 — `wgpu` as the GPU abstraction

**Status:** Accepted, revised 2026-11-24 after architecture review.

## Context

The editor must render on Vulkan (Windows/Linux), Metal (macOS), and D3D12 (Windows) with real-time
compositing and deterministic output. Maintaining three native backends triples the defect-prone surface.
The UI also needs the same device as the viewer to avoid interop copies.

## Decision

Use **`wgpu`** as the single GPU abstraction, with the `wgpu::Device`/`Queue` created by `egui-wgpu` shared
by the UI, the composite preview, and offscreen export. Shaders are WGSL.

**v1 constraint:** decoded video reaches the GPU by **CPU upload of YUV planes + GPU color conversion**.
Platform zero-copy interop (D3D12 shared textures, IOSurface) is a later optimization, not a v1 feature.

## Consequences

- One code path across platforms; backend differences funnel through a small wrapper.
- One device/queue for UI and rendering: no copies between egui and the compositor, no second adapter.
- Uniform upload path means no platform-specific decode-surface code in v1 — a large simplification.
- Costs: an upload copy per frame (acceptable at 1080p/4K preview and export); no hardware-video-surface
  interop until a measured need appears.
- Device loss is recoverable: drop GPU resources, recreate, re-render, document untouched.
- Backend bugs exist; a signed config can pin a backend per platform if a driver regression appears.

## Alternatives considered

- **Three native backends**: maximum control and interop, ~3× work; revisit only if a v1 blocker appears.
- **OpenGL**: deprecated on macOS, inconsistent compute.
- **bgfx / Dawn**: C++ integration cost outweighs benefits.
- **Vulkan + MoltenVK**: loses native Metal quality on Apple silicon and adds a translation layer.
