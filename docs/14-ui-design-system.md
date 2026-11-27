# 14 — Arcline UI Design System

**Status:** Normative, 2026-11-27. Owned by `arc-app`.
**Decision:** [ADR-0015](adr/0015-final-ui-stack-and-design-system.md) — `egui` is the only desktop UI
framework; this document defines how the product looks and behaves.
**Scope:** the standalone editor and the embedded editor. Both use the same tokens, components, and states.

> This is not a component catalogue to build all at once. It is the contract every UI change follows.
> Components are added when a screen needs them, not in advance.

---

## 1. Principles

1. **One visual language.** A panel, button, field, menu, or tooltip looks and behaves the same everywhere.
2. **Tokens before pixels.** Colours, spacing, type, radii, elevation, and motion come from `ui/tokens.rs`.
   A screen may not hard-code a colour or a one-off spacing value.
3. **The editor is the product.** Chrome supports the timeline and viewer; it must not compete with them.
4. **Quiet, dense, legible.** Professional tools are information-dense. Prefer clear hierarchy and generous
   hit areas over decoration.
5. **Every state is designed.** Hover, active, focus, selected, disabled, error, loading, empty, and
   read-only are part of the component, not an afterthought.
6. **Fast by construction.** No continuous animation or repaint when idle. The timeline and viewer are
   allowed dedicated painting paths when the generic component budget is insufficient.
7. **Incremental, not speculative.** Add a component only with a real caller. No framework-within-a-framework.

---

## 2. Token model

All values are logical points, not physical pixels. HiDPI is handled by `egui`'s pixels-per-point.

### 2.1 Colour roles

Use semantic roles in components; raw palette values live only in `tokens.rs`.

| Role | Dark (default) | Light | Use |
|---|---|---|---|
| `bg.window` | `#121316` | `#F4F5F7` | Window background behind cards |
| `bg.surface` | `#1D1F25` | `#FFFFFF` | Cards, panels |
| `bg.surface-raised` | `#24272E` | `#FFFFFF` | Popovers, menus, dialogs |
| `bg.surface-sunken` | `#17191E` | `#E9EBEF` | Timeline lanes, wells, inputs |
| `border.subtle` | `#2F333B` | `#D9DDE3` | Card and divider borders |
| `border.strong` | `#454A54` | `#B8BEC8` | Focused input, active tab |
| `text.primary` | `#E6E8EC` | `#17191E` | Primary labels |
| `text.secondary` | `#8A9099` | `#5C636E` | Secondary labels |
| `text.disabled` | `#60666F` | `#9AA1AB` | Disabled controls |
| `accent` | `#4C8DFF` | `#2563EB` | Primary action, selection |
| `accent.hover` | `#6AA0FF` | `#1D4ED8` | Hover/active accent |
| `accent.muted` | `#2B4D86` | `#DBEAFE` | Selected row/background |
| `danger` | `#FF6B6B` | `#DC2626` | Destructive actions, errors |
| `warning` | `#FFD65C` | `#B45309` | Warnings, unsaved markers |
| `success` | `#59C98B` | `#15803D` | Success, connected |
| `focus-ring` | `#8AB4FF` | `#2563EB` | Keyboard focus outline |
| `selection` | `#4C8DFF` | `#2563EB` | Text/range selection |
| `playhead` | `#FF5C5C` | `#E11D48` | Timeline playhead |

Rules:

- A component never references a hex value directly; it references a role.
- Track/clip colours are **data**, not theme constants: the timeline receives them from the project/theme
  mapping and renders them through the same token layer.
- A new role is added only when two components need the same semantic meaning. Do not add `blue2`.
- Contrast: text on `bg.surface` must meet WCAG AA for the target size; disabled text is exempt but still
  legible.

### 2.2 Typography

| Token | Size | Weight | Use |
|---|---:|---:|---|
| `type.caption` | 10 | 400 | Timestamps, metadata, ruler |
| `type.small` | 11 | 400 | Secondary labels, list metadata |
| `type.body` | 12 | 400 | Default UI text |
| `type.body-strong` | 12 | 600 | Panel titles, selected values |
| `type.subtitle` | 14 | 600 | Dialog titles, section headers |
| `type.title` | 16 | 600 | Empty-state titles |
| `type.display` | 20 | 650 | About/product title |

Fonts:

- UI Latin: **Inter** when bundled; otherwise the platform UI font (`Segoe UI`, `SF Pro Text`, `Noto Sans`).
- CJK: **Noto Sans SC** subset (`assets/fonts/NotoSansSC-Arcline.ttf`, family `Arcline CJK`). A UI that
  shows Chinese must use the bundled font or an explicit fallback chain, not rely on the machine having it.
- Monospace: for timecode/numeric fields only; never for general prose.

Rules:

- No text below 10 pt. 10 pt only for ruler/metadata, never for actions.
- Use the type ramp, not ad-hoc `FontId::proportional(9.5)`.
- Numeric fields use tabular figures where supported.

### 2.3 Spacing

A 4 pt base scale:

```
space.0 = 0
space.1 = 4
space.2 = 8
space.3 = 12
space.4 = 16
space.5 = 20
space.6 = 24
space.7 = 32
space.8 = 40
```

Use:

- Window margin: 12.
- Gap between cards: 10 (a deliberate half-step; keep it as `layout.card-gap`).
- Card inner padding: 14 horizontal, 12 vertical.
- Control horizontal padding: 10; compact 8.
- Row height: 28 compact, 32 default, 36 comfortable.
- Icon-to-label gap: 6; button group gap: 4.

### 2.4 Radii, borders, elevation

| Token | Value | Use |
|---|---:|---|
| `radius.xs` | 2 | Checkbox, tiny badge |
| `radius.sm` | 4 | Inputs, small buttons |
| `radius.md` | 6 | Buttons, menus, popovers |
| `radius.lg` | 10 | Cards, panels, dialogs |
| `border.hairline` | 1 | Dividers and card borders |
| `elevation.0` | none | Flat surfaces |
| `elevation.1` | 1 px border + subtle shadow | Cards |
| `elevation.2` | 8 px blur, 24 % black | Menus, tooltips, popovers |
| `elevation.3` | 24 px blur, 36 % black | Modal/dialog |

Shadows are optional; a border plus contrast is preferred on the timeline. Do not stack multiple shadows.

### 2.5 Iconography

- 16, 20, and 24 pt sizes; 1.5 pt stroke at 16/20, 1.75 pt at 24.
- One icon family, consistent optical weight; no emoji as product iconography.
- Icons are always paired with an accessible label or tooltip.
- Icons are painted from vector paths/SVG, not a font glyph, unless the family is bundled and licensed.
- A new icon is added to the shared icon module and reused; no inline path strings in panels.

### 2.6 Motion

| Token | Duration | Use |
|---|---:|---|
| `motion.instant` | 0 ms | Timeline playhead, drag feedback, hover on dense rows |
| `motion.fast` | 120 ms | Button/hover, focus ring, tooltip fade |
| `motion.normal` | 180 ms | Panel open, menu, selection |
| `motion.slow` | 240 ms | Dialog/modal, larger layout transition |

Easing: `cubic-bezier(0.2, 0, 0, 1)` (fast-out, settle). Respect reduced-motion: use instant for all
non-essential transitions, never animate the timeline viewport, playhead, or ruler.

---

## 3. Themes

- **Dark** is the default product theme.
- **Light** is a first-class theme, not an afterthought. It must be screenshot-tested.
- **High contrast** is an accessibility mode: stronger borders, higher text contrast, focus ring always
  visible; it may drop shadows and translucency.
- Theme selection: system preference on first run, then explicit user setting; switching is instant and
  does not reload the project.
- All colours and shadows are resolved through `Theme`; no module caches a colour at startup.
- The viewer's checkerboard, safe-area guides, and timeline lane colours are theme roles, not hard-coded
  RGB in the canvas code.

---

## 4. Layout system

### 4.1 Four-panel shell

Default proportions (fractions of the inner window area):

- Media: 24.5 %
- Viewer: 51 %
- Inspector: 24.5 %
- Timeline: 34 % of height

Rules:

- Window margin 12; card gap 10; card radius 10; card border 1.
- Splitters are 10 pt hit areas with a 3 pt visual grip; cursor changes on hover.
- Minimum card size 120 × 120; timeline minimum height 180; viewer keeps a 16:9 stage where possible.
- Panel fractions persist in the workspace layout, with clamps (left/right 0.12–0.40, bottom 0.18–0.50).
- Timeline is a working surface, not a titled placeholder: its toolbar is the first row.
- The embedded/compact layout reuses the same cards and splitters; it does not define a second shell.

### 4.2 Density

- Default: 32 pt rows, 12 pt body.
- Compact: 28 pt rows, 11 pt body, used in the embedded layout and dense lists.
- Comfortable: 36 pt rows, 13 pt body, used in settings/dialogs.
- Density is a user preference and a theme token; components read it, not hard-code row heights.

### 4.3 Focus and keyboard

- Visible focus ring on every interactive element; never remove it without a replacement.
- Tab order follows visual order; arrow keys move within lists/menus; Escape closes transient UI.
- Every action reachable by keyboard; the command palette is the escape hatch.

---

## 5. Component library

### 5.1 Location and dependency direction

```
arc-app/src/ui/
  mod.rs
  tokens.rs
  theme.rs
  icons.rs
  components/
    button.rs
    icon_button.rs
    text_input.rs
    number_input.rs
    slider.rs
    select.rs
    menu.rs
    toolbar.rs
    panel.rs
    splitter.rs
    scroll_area.rs
    tooltip.rs
    modal.rs
    tabs.rs
    badge.rs
    progress.rs
    toast.rs
    empty_state.rs
    inspector_row.rs
  patterns/
    dialog.rs
    transport.rs
    list_row.rs
  widgets/
    timeline.rs
    viewer.rs
```

- `arc-app` is the only crate that depends on `egui`.
- `components/` contains visual primitives; `patterns/` composes them for a recurring product need;
  `widgets/` contains the two high-performance canvases.
- A component may depend on `tokens`, `theme`, and `icons`; it may not depend on a specific panel or engine
  type. It accepts typed data and emits a callback/command.
- `widgets/timeline.rs` and `widgets/viewer.rs` may use `egui::Painter` meshes or a dedicated `wgpu` pass;
  they still consume the same colour/type tokens.

### 5.2 First tranche (build only when a screen needs it)

| Component | Variants / states | Needed by |
|---|---|---|
| `Button` | primary, secondary, ghost, danger; sm/md; icon left/right; loading | dialogs, inspector, empty states |
| `IconButton` | 24/28/32; tooltip required | toolbar, panel headers, timeline |
| `TextInput` | single-line, search, error, disabled, read-only; IME-safe | inspector, search, rename |
| `NumberInput` | step, unit, expression entry, drag-to-scrub | inspector, export settings |
| `Slider` | horizontal, value label, marks, disabled | zoom, volume, settings |
| `Select` | single-select, searchable, disabled | inspector, export settings |
| `Menu` / `ContextMenu` | items, separators, shortcuts, disabled, submenu later | menubar, right-click |
| `Toolbar` | grouped actions, separators, overflow later | timeline, viewer |
| `Panel` / `Card` | title, trailing, body, scroll | four panels, dialogs |
| `Splitter` | vertical/horizontal, min/max, snap | four-panel layout |
| `ScrollArea` | vertical/horizontal, styled bar, virtualization hook | media bin, inspector, timeline |
| `Tooltip` | text, shortcut, delay; never covers the pointer target | every icon button |
| `Modal` / `Dialog` | title, body, footer, destructive confirm, Esc | export, settings, consent |
| `Tabs` | underline/pill, overflow later | inspector sections, settings |
| `Badge` / `StatusDot` | neutral, accent, success, warning, danger | job state, track state |
| `Progress` / `Spinner` | determinate, indeterminate, inline | jobs, import |
| `Toast` | info/success/warning/error; action link | background jobs |
| `EmptyState` | icon, title, description, one action | empty media/timeline |
| `InspectorRow` | label, value, reset, mixed state | inspector property grid |

Deliberately **not** in the first tranche: data grids, tree views, color pickers, curve editors, docking
framework, command palette, timeline clip component. Add them when a concrete feature requires them.

### 5.3 Component contract

Every shared component must:

- Expose a typed `Props`/argument struct, not a bag of booleans.
- Render all states: default, hover, active/pressed, focus-visible, disabled, selected, error, loading,
  read-only (where applicable).
- Use tokens for colour, spacing, radius, type, and motion.
- Return `egui::Response` (or a typed result) so callers can test and position it.
- Support keyboard activation and an accessible label/role.
- Never mutate engine/project state; emit a command or callback.
- Never open a file dialog or spawn work directly; those are app-level actions.
- Have a doc comment with a minimal usage example and a list of states.

### 5.4 Patterns

Patterns compose components but do not create a second visual language:

- `dialog`: title, body, destructive footer, Esc/Enter.
- `transport`: play/pause, step, timecode, quality badge.
- `list_row`: leading icon/thumb, title, metadata, trailing action, selected state.
- `property_row`: label, control, reset, description/validation.
- `empty_state`: icon, title, explanation, primary action.
- `job_strip`: progress, cancel, error detail.
- `context_menu`: item groups, shortcut labels, destructive item last.

### 5.5 Component governance

- **Add only with a caller.** Two real screens or one screen with a state/accessibility requirement that a
  one-off would duplicate.
- **No parallel controls.** If a panel needs a different look, add a variant to the shared component.
- **No local palettes.** A module may not define `const BLUE: Color32`.
- **Token changes** update this document and the visual regression set.
- **Deprecation:** mark with `#[deprecated]` and a replacement; remove after all callers migrate.
- **Ownership:** one owner for `ui/`; changes to tokens/components need a UI review, even a small one.

---

## 6. High-performance areas

The timeline and viewer are allowed dedicated rendering paths; they are not an excuse to fork the design.

### 6.1 Timeline

- Paint only visible clips/tracks; LOD by zoom.
- Use a single canvas painter/mesh or a dedicated `wgpu` pass when `egui` shapes exceed budget.
- Fixed left header; horizontal/vertical scroll; ruler and playhead share one time-to-pixel mapping.
- Selection, hover, trim handles, snapping guides, and waveforms use theme roles.
- No per-clip widget tree for the whole timeline; no per-frame model rebuild on scroll.
- Budget: 8 ms CPU paint for 8 tracks / 2,000 visible clip-seconds; 60 fps scroll/drag.

### 6.2 Viewer

- Present the compositor texture through `egui_wgpu` user texture/custom callback; no CPU readback.
- Fit/100 %/1:1, safe guides, quality badge, timecode overlay.
- The checkerboard and overlays use theme roles.

### 6.3 Idle

- `request_repaint` only for input, active animation, or job/agent events.
- The `winit` loop uses a wait policy when there is no animation.
- Empty/idle windows must not spin. Target: 0 % CPU when fully idle; ≤ 1 % with a blinking caret.

---

## 7. Accessibility and input

- Accessible role/label for every interactive control; icon-only buttons require a tooltip and label.
- Keyboard complete: tab order, focus ring, Enter/Space activation, Escape dismissal, arrow navigation.
- IME: never intercept composition; text fields must pass through preedit/commit; test with Chinese IME.
- HiDPI: logical units only; test 100 %, 150 %, 200 %; no pixel-snapped 1 px borders that disappear.
- Reduced motion and high contrast are themes, not separate widgets.
- Color is never the only signal; pair with icon/text/shape.

---

## 8. Testing and review

### 8.1 Per-component

- Unit test for state transitions and disabled/error behavior.
- Snapshot/visual test for dark/light/high-contrast at 100 % and 150 % (when the harness is available).
- Keyboard smoke test (Tab, Enter, Escape, arrow).
- No hard-coded colours/spacings in the diff.

### 8.2 Per-screen

- Four-panel layout at 1280×720, 1600×1000, and embedded 1200×800.
- Empty, loading, error, and selected states.
- IME input in a text field; Chinese text renders with the bundled font.
- Idle CPU/GPU check; no continuous repaint.

### 8.3 Performance gate

- 1,000-clip scene: UI work p50 ≤ 2 ms, p95 ≤ 8 ms on the reference machine; no frame > 20 ms in a
  600-frame scroll run.
- 8 tracks / 2,000 visible clip-seconds: timeline paint ≤ 8 ms.
- Preview path: no readback; prepare cost dominated by compositor, not presentation.
- Any regression > 15 % fails CI per [08](08-performance-reliability.md).

### 8.4 PR checklist

- [ ] Tokens/components used, no one-off styling.
- [ ] States and keyboard/accessibility covered.
- [ ] Dark and light themes checked.
- [ ] No CPU readback added.
- [ ] No continuous repaint added.
- [ ] Timeline stays virtualized; budget unchanged.
- [ ] No new UI framework/dependency (Slint/MLT excluded).

---

## 9. Evolution

- The design system is versioned with `arc-app`; there is no separate crate.
- New components are added in the first tranche order only when needed; no speculative catalogue.
- A visual redesign updates tokens and this document first, then components, then screens.
- The design system does not block high-performance exceptions, but exceptions must use its tokens and
  document why the generic path was insufficient.
- Historical research (custom toolkit, Slint, Qt) is retained in ADRs and the research record; it is not an
  open option. New toolkit proposals require a new ADR with production evidence.
