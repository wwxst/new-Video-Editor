# ADR-0015 — Final desktop UI stack: `egui` only, with an Arcline design system

**Status:** Accepted, 2026-11-27 — **final**.
**Confirms:** [ADR-0007](0007-native-wgpu-ui.md)'s `egui` decision.
**Closes:** the "future toolkit swap / Slint fallback" clauses in ADR-0007 and the earlier reviews.
**Supersedes in part:** the forward-looking parts of ADR-0007; it does not change the existing six-crate
architecture, the command model, or the embedding mechanism in [ADR-0014](0014-integration-levels-and-native-embedding.md).

## Context

Arcline is planned for a 5–6 year product life. The UI must stay fast on the timeline and viewer, work as
a top-level editor and as a native child window in an Agent client, support CJK input and HiDPI, and be
maintainable by a very small team. The project has now evaluated the plausible alternatives, including a
full Slint 1.19.1 + wgpu 30 validation on the current Windows toolchain.

The evaluation found that Slint can implement the four panels, timeline, zero-copy preview, native child
window, Chinese input, and licence requirements. It also found that the Slint retained-mode timeline was
10–13× slower than `egui` on the same 1,000-clip scene, with 5–7 % of frames over 20 ms, and that its
Skia renderer could not be built with the current windows-gnu toolchain. The POC is archived on branch
`poc/slint-ui`; its code and results are evidence, not a migration candidate.

The important long-term risk is no longer "which toolkit"; it is **UI drift**: bespoke buttons, colours,
spacing, focus states, and animation in every panel. The project needs one design system and one component
library, not another toolkit evaluation.

## Decision

### 1. The stack is fixed

| Layer | Decision |
|---|---|
| Language | Rust 2024 |
| Desktop UI | **`egui` + `egui-winit` + `egui-wgpu` — the only desktop UI framework** |
| Window management | `winit` via the `egui-winit` host loop |
| GPU | `wgpu` for UI, compositor, effects, and preview presentation |
| Media | FFmpeg 9.1 LGPL, in-process preview decode |
| Audio | `cpal` output, `arc-audio` mixer |
| Agent surface | MCP, JSON-RPC, SDK, Events over the existing command model |

**Not adopted:** Slint, MLT, any second retained-mode UI toolkit, any web/Electron/Tauri UI.

**No further UI-toolkit selection work is planned.** New UI proposals must fit this ADR and the design
system; "replace the toolkit" is out of scope unless this ADR is explicitly superseded by a new decision
backed by a production failure, not by a preference.

### 2. Keep six crates; put the component library in `arc-app`

The six-crate architecture is unchanged:

```
arc-core → arc-media / arc-render / arc-audio → arc-cli → arc-app
```

The shared UI library lives in **`arc-app/src/ui/`**, not in a seventh crate:

```
arc-app/src/ui/
  tokens.rs        # colour roles, spacing, type ramp, radii, elevation, motion
  theme.rs         # dark / light / high-contrast theme assembly
  components/      # Button, TextInput, Slider, Select, Menu, Toolbar, Panel, ...
  patterns/        # inspector row, transport bar, dialog layout, empty state, ...
  widgets/         # timeline + viewer; high-performance custom painting
```

Rules:

- `arc-core`, `arc-media`, `arc-render`, `arc-audio`, and `arc-cli` must not depend on `egui`.
- `arc-app` owns all `egui` types, theme state, focus, and component composition.
- Components emit commands or typed callbacks; they never mutate the project or engine state directly.
- No second copy of a control. If a module needs a variation, add a typed variant to the shared component.
- A new component is added only when at least two screens need it, or when a current screen needs the
  states/accessibility that a one-off cannot provide.

### 3. Design system is normative

[Document 14 — UI Design System](../14-ui-design-system.md) is the single normative source for colours,
typography, icons, spacing, radii, elevation, themes, component APIs, states, motion, and the component
acceptance checklist. All UI code follows it. A module may not introduce its own palette, spacing scale,
button style, or hover treatment.

### 4. Two product modes, one UI

Standalone and Embedded modes use the same components and tokens:

- Standalone: the normal four-panel editor.
- Embedded: the same editor as an `egui-winit` child window, with a compact layout where the host reserves
  a smaller region. No forked widget set and no separate embedded-only UI toolkit.

The Agent client may wrap or host the editor, but it does not get a different visual language.

### 5. Performance rules that are now mandatory

These were validated during the Slint POC and apply to the `egui` product:

1. **GPU-shared preview / zero-copy presentation.** The compositor's `Rgba8UnormSrgb` texture is presented
   through `egui_wgpu` as a user texture or a custom paint callback. No CPU readback in the UI preview path.
2. **On-demand repaint.** `egui` requests a repaint only when input, state, animation, or a job event
   requires it. The `winit` loop uses a wait policy when idle; no continuous polling. Idle UI CPU target is
   ≤ 1 % of one core; measured target is 0 % when the window is fully idle.
3. **Virtualized timeline.** Paint only visible clips and LOD levels; never build one widget per clip for
   the entire timeline.
4. **Custom canvases are allowed** where the generic component path cannot meet budget: timeline and viewer
   may use `egui::Painter` meshes or a dedicated `wgpu` pass behind the existing canvas interface. They
   still use the design tokens for colour and typography.

### 6. Dependency and upgrade policy

- Pin exact minor versions in `Cargo.lock`; upgrade `egui`, `egui-wgpu`, `egui-winit`, `winit`, and `wgpu`
  as a tested group, not independently.
- Keep a short "UI dependency upgrade" checklist: compile, 1,000-clip timeline benchmark, IME, HiDPI at
  100/150/200 %, embedded child window, and a visual smoke test.
- No speculative abstraction layer for a future toolkit. The boundary is the existing `arc-app` UI module
  and the command/event protocol; that is enough for testing and for the embedded mode.

### 7. Research records are retained, not used as options

- `poc/slint-ui` remains an archived branch with the Slint POC, screenshots, measurements, and
  `reports/slint-feasibility-2026-11-27.md`.
- `docs/architecture/technology-validation.md` and `reports/architecture-review.md` remain as historical
  research records. Their "future Slint/Qt swap" language is superseded by this ADR.
- No Slint dependency, crate, generated code, or build step enters the product workspace.

## Consequences

**Positive**

- One UI stack, one renderer integration, one component library, one theme.
- The editor keeps the measured `egui` performance headroom (0.57 ms UI work at 1,000 clips; 0 late frames
  in the control run).
- The long-term cost is in design-system discipline, not toolkit migration.
- Standalone and Embedded keep the same UI; the Agent client does not fork the editor.

**Negative / accepted**

- `egui` still does not look native by default; the design system and custom components carry the product
  look. This is a deliberate cost.
- `egui` API churn is real; the pinned upgrade group and visual/perf checklist contain it.
- A future toolkit change would be a new ADR with production evidence, not an open-ended fallback.

## Compliance checklist (for UI PRs)

- [ ] Uses `arc-app/src/ui` tokens/components; no new raw colours, font sizes, radii, or spacing values.
- [ ] Dark and light themes both render; focus/hover/active/disabled/selected states exist.
- [ ] Keyboard path and accessible label exist for every interactive control.
- [ ] No CPU readback added to the preview path.
- [ ] No continuous `request_repaint`; idle behaviour tested.
- [ ] Timeline changes stay virtualized and within the 8 ms paint budget.
- [ ] No new UI framework, no Slint/MLT dependency.
- [ ] Screenshot or visual test updated for visible changes.

## Alternatives considered

- **Slint** — validated end to end; rejected for the editor because of the measured retained-mode cost,
  Skia build constraint, and migration cost. Archived on `poc/slint-ui`.
- **Custom retained-mode toolkit** — the original v1 proposal; rejected for schedule and long-tail
  accessibility/IME work (ADR-0007).
- **Qt** — strong chrome and docking, but heavy `wgpu` integration and LGPL/commercial complexity.
- **Web/Electron/Tauri** — violates the native requirement and complicates GPU video surfaces.
- **Keep evaluating toolkits** — rejected. The product cost now is UI consistency and delivery, not
  another bake-off.
