# 09 — Plugin System

Three tiers, chosen by the trust and performance profile of the plugin. The tier is visible in the UI and
enforced by the host.

| Tier | Format | Trust | Performance | Ships |
|---|---|---|---|---|
| 1 | Built-in: WGSL + manifest | fully trusted | native GPU | P1 |
| 2 | WASM component | untrusted (agent-authored, downloaded) | GPU passes via host ABI; CPU limited | P5 |
| 3 | OFX (OpenFX) native | third-party vendor binary, user-installed | CPU (GPU via host helper), tiled | P5 |

Audio plugins (VST3) run in their own subprocess ([05 §8](05-audio-engine.md)); they are not part of this
tiering.

## 1. Tier 1 — built-in effects

Described in [03 §5](03-render-engine.md). The manifest is the contract; adding an effect requires a
shader, a manifest, a CPU reference for tests, and golden fixtures. No dynamic loading — these are compiled
into the binary for determinism and startup speed.

## 2. Tier 2 — WASM effects (agent-authored)

The strategic reason for this tier: an agent can *write* an effect (WGSL + parameters) and run it safely,
without a human installing native code.

- **Container**: WebAssembly Component Model, `wit` interface versioned (`arcline:effect@1`).
- **Memory model**: the host exposes inputs as read-only buffers and one output buffer; the guest never
  receives a raw pointer or a `wgpu` handle.
- **GPU access**: the guest supplies WGSL source + parameter layout through the WIT interface; the *host*
  compiles and runs it under the same sign-off as built-ins (shader validation, capability limits: no
  atomics on host buffers, no arbitrary storage writes, bounded loop analysis for Tier C).
- **Fuel and memory limits**: instruction fuel per frame budget, 64 MB guest memory, 5 s compile timeout.
  Exceeding fuel disables the effect for the session with a typed error.
- **Determinism**: the WASM interpreter itself is deterministic; the shader it produces follows the
  [03 §8](03-render-engine.md) contract.
- **Distribution**: effects are files in `~/.arcline/effects/<name>/` with `manifest.json`, `effect.wasm`,
  and optional `shader.wgsl`. A hash pins the version; the project records the hash so a project does not
  silently change behavior when the effect is updated.

```
effects/agent_motion_blur/
├── manifest.json     # id, version, params (same schema as built-ins), wit version
├── effect.wasm       # optional; may be omitted if the effect is pure WGSL
└── shader.wgsl       # fragment/compute entry points against shader_abi 1
```

## 3. Tier 3 — OFX host

OFX is the industry's cross-vendor plugin standard (Resolve, Nuke, Fusion, Baselight all host it). Hosting
it is a large adoption lever: existing plugin libraries work unmodified.

### 3.1 Host architecture

```
┌─────────────────────────── App process ───────────────────────────┐
│ arc-plugin-host                                                    │
│  ├── OFX bundle loader (dlopen, symbol table validation, ABI ver)  │
│  ├── plugin registry (hosts, contexts: filter/general, params)     │
│  ├── instance manager (clip-level instances ↔ EffectInstance)      │
│  └── CPU render bridge: tiled RGBA16F buffers ⇄ render graph node  │
└────────────────────────────────────────────────────────────────────┘
```

- **Process isolation**: OFX plugins are loaded in a dedicated **plugin host subprocess** with its own
  address space. A crash is caught, reported once, and the effect is bypassed with a visible badge. This
  contradicts the desire for zero-copy, and that is an accepted cost: third-party native code is the single
  most likely crash source in any NLE.
- **Rendering**: OFX is predominantly CPU/tile-based. The host renders the effect's input tile at the
  effect's declared thread-safety level (parallel tile render if the plugin declares support), converts
  from RGBA16F linear to the plugin's expected color space per OFX color management, and returns the tile.
  GPU-native OFX plugins (OpenGL-based) are supported only on platforms where the host can share a context;
  otherwise they fall back to the CPU path and are flagged "slow" in the UI.
- **Parameter bridge**: OFX parameters map to our manifest model 1:1 for common types (double, int, bool,
  choice, RGB, string, push-button) and keyframes map to OFX animation timelines. Unsupported parameter
  types are surfaced read-only, never silently dropped.
- **License/versioning**: OFX plug-ins are version-pinned per instance; a project records
  `{bundle, version, hash}` so a render can report exactly which binary produced it.

### 3.2 Compatibility target

v1 goal: **the free/demo builds of the major OFX suites run without modification** on Windows and macOS
(Resolve's bundled OpenFX set and a curated list of open-source plugins). Linux support follows if demand
justifies it (many commercial vendors do not ship Linux OFX).

## 4. Permissions and trust

- **Tier 1**: no permissions.
- **Tier 2**: no filesystem, no network, no syscalls by construction (WASM sandbox); only the declared
  buffer interface.
- **Tier 3**: native code — full user privileges once loaded, so consent is explicit: a modal listing the
  bundle path, vendor, and hash; a global "trusted vendors" list; and the `plugin:load` capability for
  agent sessions is denied by default. Plugins are never auto-installed from a project file.

## 5. SDK and documentation

- **Effect SDK**: Rust crate (`arc-effect-sdk`) that generates the manifest + shader scaffolding, a WIT
  package for WASM, and a CLI (`arcline effect new/test/package`). The test harness renders the effect
  against the golden corpus and reports cost class.
- **Agent-facing**: `effect_describe` returns the manifest for any installed effect ([06 §6](06-agent-interface.md));
  installing a WASM effect is a human action, but *writing* one is a supported agent workflow ("make the
  logo pulse" → agent writes a WGSL effect, user reviews and installs).
