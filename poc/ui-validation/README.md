# Arcline UI validation POC (Slint + egui)

This workspace is deliberately outside the six-crate Arcline workspace. It does not change
`crates/` or `main`; it draws the same six-lane scene with both toolkits so the measurements are
like-for-like.

## Build

```powershell
$env:PATH = "$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-gnu\bin;" +
            "$env:USERPROFILE\.local\toolchains\mingw64\bin;" + $env:PATH

cargo build --release -p slint-poc
cargo build --release -p egui-poc
```

Slint uses FemtoVG on wgpu 30. The `skia` feature is kept only as a build experiment; it is not
buildable on windows-gnu in this environment.

## Run

```powershell
# Four panels + GPU preview
.\target\release\slint-poc.exe --clips 200 --frames 0 --video-mode native `
  --scenario selected --width 1600 --height 1000

# Timeline-only, for the egui comparison
.\target\release\slint-poc.exe --clips 1000 --frames 600 --video-mode native --ui timeline

# Native Agent host (in-process child window)
.\target\release\slint-poc.exe --embed-host 1 --embed-seconds 8 --width 1200 --height 800

# 150% scale simulation
.\target\release\slint-poc.exe --scale 1.5 --width 1200 --height 800 --frames 0 --video-mode native
```

Scenarios: `default`, `selected`, `resized`, `scrolled`, `zoomed`, `about`.
Useful flags: `--ui timeline`, `--cull 1`, `--shot <png> --shot-delay-ms <n>`,
`--idle-seconds <n>`, `--startup-marker <json>`, `--result <json>`, `--interact zoom`.

## Measurement

All harnesses are in `../results`:

| Script | Purpose |
|---|---|
| `measure.ps1` | External wall/CPU/peak RSS + the JSON the app wrote |
| `idle-measure.ps1` | First-frame startup + idle CPU/working set |
| `measure-builds.ps1` | Isolated clean/incremental builds (separate target dirs) |
| `interaction-test.ps1` | Real Win32 splitter drag, clip click, zoom button, wheel |
| `ime-test.ps1 -Mode direct/ime` | Chinese Unicode input and Microsoft Pinyin composition |
| `capture-embed-host.ps1` | In-process Win32 host + screen capture |
| `embed-host.ps1` | Cross-process WinForms host (partial embedding probe) |

The full report is `../../reports/slint-feasibility-2026-11-27.md`.

## Layout

```
crates/scene/         deterministic six-track scene + metrics
crates/egui-poc/      egui 0.37 timeline-only comparison
crates/slint-poc/
  ui/app.slint        four panels, media, inspector, IME, About
  ui/timeline.slint   fixed headers, Flickable lanes, ruler, selection, playhead
  ui/theme.slint      design tokens
  src/main.rs         wgpu sharing, model publishing, input, measurement
  src/embed_host.rs   Win32 Agent host for --embed-host
  ui/fonts/ArclineCJK.ttf
```

## Caveats

- The Slint POC is a **validation shell**, not a feature-complete editor. Clip trimming, context
  menus, real media thumbnails/waveforms from `arc-app::timeline`, accessibility, and a native
  file dialog are not ported.
- The Slint numbers are FemtoVG/wgpu, not Skia.
- The egui POC is timeline-only; the full Slint four-panel number is reported separately.
- `--cull 1` is an experiment, not the default: it rebuilds the clip model when the viewport
  leaves the published range, which introduces model-churn spikes.
