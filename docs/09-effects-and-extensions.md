# 09 — Effects & Extensions

v1 has **no plugin system**. It has a clean built-in effect model whose manifest format is designed so that
third-party tiers can be added after 1.0 without breaking projects. This document defines the built-in
model and records what is deferred, and why.

## 1. Built-in effects

An effect is:

- one WGSL entry point (or a small chain of passes),
- a JSON manifest (parameters, ranges, defaults, units, quality tiers),
- a CPU reference implementation used by tests and the software fallback,
- golden fixtures.

The manifest is the contract: it drives the inspector ([07 §3](07-ui-architecture.md)), MCP discovery
([06 §6](06-agent-interface.md)), and validation. There is one manifest per effect, versioned with the
project schema.

```jsonc
{
  "id": "core.blur.gaussian",
  "version": 1,
  "display": "Gaussian Blur",
  "category": ["blur"],
  "gpu": { "shader": "builtin:gaussian_blur", "quality_tiers": ["draft", "good", "best"] },
  "params": [
    { "key": "radius", "type": "float", "range": [0, 200], "default": 8, "unit": "px" },
    { "key": "quality", "type": "choice", "options": ["draft", "good", "best"], "default": "good" }
  ]
}
```

v1 ships six effects ([03 §4](03-render-engine.md)). The shader ABI is deliberately small and versioned
(`shader_abi: 1`), so future plugin tiers and agent-authored effects can target the same interface.

## 2. Authoring workflow

`arc-render` owns a `builtin` registry: a compile-time list of manifests and shader modules. Adding an
effect is:

1. Write the WGSL pass(es) against the ABI.
2. Add the manifest.
3. Add a CPU reference and a golden fixture.
4. Register it in the builtin list.

No dynamic loading, no ABI/version negotiation at runtime, no sandbox. This is appropriate for a
single-developer v1: every effect is reviewed, tested, and deterministic on the machines we support.

## 3. Deferred: third-party effects

The architecture review removed the three-tier plugin plan from v1. The reasons are concrete:

- **OFX** is native C in the process, cannot be sandboxed cheaply, and needs a parameter/animation bridge,
  color-space negotiation, tiling, and threading — weeks of work before the first third-party plugin runs.
- **WASM effects** are strategically attractive (agents could author effects) but require a host ABI,
  fuel/memory limits, shader validation, and a distribution format.
- **VST3** needs subprocess audio hosting and crash isolation.

None of these serve a v1 user. The manifest and shader ABI are the seams that make them additive later:

| Tier | Plan (post-1.0) |
|---|---|
| OFX | Subprocess host speaking a small render RPC; plugin instances referenced by `{bundle, version, hash}` |
| WASM | Component-model guest that supplies WGSL + parameter layout through a host interface; fuel and memory caps |
| VST3 | Audio-plugin subprocess with reported latency compensation |

When added, plugins will be capabilities, not defaults: loading native code is an explicit user action.

## 4. Deferred: color-management extensions

- **HDR10/HLG** output, ACES working spaces, and OCIO configs are post-1.0. v1 is Rec.709 SDR with
  linear-light internals and LUT support ([03 §8](03-render-engine.md)).
- The internal pipeline is already RGBA16F linear, which is the expensive structural prerequisite; adding
  HDR later is a transform and metadata problem, not a rewrite.

## 5. Deferred: interchange

EDL, FCPXML, and OTIO import/export are post-1.0. The document model is designed to map onto them, but
each adapter is a compatibility project with its own test corpus and no v1 user. Project files are the
native interchange format until then.

## 6. What this buys

Removing the plugin tiers deletes roughly a crate's worth of code, two sandboxes, a second process model,
and a compatibility matrix — while leaving the effect model extensible in the only way that matters for v1:
adding a built-in effect is a small, local, testable change.
