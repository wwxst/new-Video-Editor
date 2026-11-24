# 11 — Risks & Legal

Two sections: the engineering risk register, and the legal/licensing posture that constrains what can ship.
The second is not boilerplate — codec licensing has killed more video products than bugs have.

## 1. Risk register

Scoring: likelihood (L) and impact (I) on 1–5; exposure = L×I.

| # | Risk | L | I | Exposure | Mitigation | Trigger / contingency |
|---|------|---|---|----------|------------|------------------------|
| R1 | **Solo capacity and scope creep** | 4 | 5 | 20 | Vertical-slice plan, explicit v1 cut list, phase gates, cut Phase G features before extending the schedule | Any phase > 30% over estimate → cut, do not extend |
| R2 | **Codec licensing cost or refusal** | 3 | 5 | 15 | Hardware-only H.264/HEVC encode; AV1 (SVT) software encode; legal review before Phase B ships | If licensing cost exceeds threshold: AV1-default + hardware H.264 only |
| R3 | **FFmpeg FFI build on Windows** (headers/libs, runtime DLLs, LGPL packaging) | 4 | 4 | 16 | Pin a known-good LGPL build early in Phase B; keep build scripts in-repo; use the `ffmpeg` CLI for batch jobs to reduce FFI surface | If FFI blocks Phase B: ship CLI-based export first, defer in-process decode |
| R4 | **In-process decoder crash on hostile media** | 3 | 4 | 12 | Panic isolation per clip, autosave/journal, fuzzed media corpus; subprocess decode is the documented upgrade ([ADR-0003](adr/0003-ffmpeg-for-media-io.md)) | Crash rate above threshold → move decode to a worker process |
| R5 | **`egui` polish or performance ceiling** | 3 | 3 | 9 | Custom-painted timeline/viewer, token theming, measured optimization; dedicated timeline pass only if profiling demands it | If timeline paint misses budget, add a dedicated wgpu pass behind the same interface |
| R6 | **GPU driver bugs / device loss** | 3 | 3 | 9 | Device-loss recovery, driver verdict cache, half-resolution fallback, crash telemetry by driver | Blacklist specific driver versions via signed config |
| R7 | **Long-GOP seek slower than budget** | 3 | 2 | 6 | Proxies above threshold, GOP cache, hardware decode, async proxy jobs with UI indication | "Optimize media" one-click job; lower default proxy threshold |
| R8 | **Audio device variability** (Bluetooth, exclusive mode, sample-rate changes) | 3 | 2 | 6 | Master-clock re-anchor, device-change handling, latency calibration UI, null-device CI | Documented troubleshooting; WASAPI shared mode only |
| R9 | **AI-generated code quality drift** | 3 | 3 | 9 | Small PRs, property tests on the command layer, strict crate dependency rule, golden tests, human review of all FFI and unsafe code | Slow down and refactor before adding features when tests get flaky |
| R10 | **macOS/Linux bit-rot before support** | 3 | 2 | 6 | `cargo check` on all three OSes in CI from Phase A; no platform-specific APIs outside `arc-media`/`arc-app` | Drop to documented "unsupported" until a port phase |
| R11 | **Packaging and signing delays** | 3 | 2 | 6 | Start the Windows installer spike in Phase E, not H; use standard signing services | Portable zip as the fallback distribution |
| R12 | **Data loss** | 2 | 5 | 10 | Journal + atomic saves + forced-kill CI; recovery-to-revision UI; pre-save snapshot option | Support runbook for manual journal recovery |
| R13 | **Determinism expectations misunderstood** | 2 | 2 | 4 | Same-machine contract stated in docs and export report; tolerance-based goldens | Diagnostics in `export_report.json` explain differences |

The top exposures (R1, R2/R3) are revisited at every phase gate.

## 2. FFmpeg and codec licensing

**Stated plainly: this section requires counsel. It records engineering constraints and intended posture,
not legal advice.**

### 2.1 FFmpeg

FFmpeg is LGPL-2.1-or-later by default; `--enable-gpl` components (including x264 and x265) make the whole
build GPL. Arcline's posture:

- Build with **no GPL components**, dynamically link the shared libraries, ship them as separate DLLs/dylibs,
  and provide the LGPL compliance package: exact source, build scripts, modification list, relink
  instructions — published per release.
- Pin the FFmpeg version; track CVEs weekly; treat decoder crash rates as a tracked metric.
- On Linux (post-1.0), prefer system FFmpeg via distro/Flatpak packaging.

### 2.2 Patents (the harder problem)

H.264/AVC and HEVC are covered by patent pools whose terms apply to encoding/decoding products, not just
software licenses. AV1 is royalty-free (AOMedia) and is the only major modern codec with a clear free path.

Intended posture:

| Capability | Posture |
|---|---|
| Decode H.264/HEVC/AV1/ProRes | expected to be covered by platform licenses; confirm with counsel |
| Encode H.264/HEVC | **hardware encoders only** (NVENC/QSV/AMF/VideoToolbox are licensed at the driver/platform level) |
| Encode AV1 | SVT-AV1 (BSD) — royalty-free; recommended default for web delivery |
| Encode ProRes | editorial mezzanine; review Apple's trademark and licensing terms with counsel; widely used by open-source tools but not free of considerations |
| Software H.264/HEVC encoders | **not shipped** (x264/x265 are GPL; OpenH264 has its own terms) |
| Naming | "H.264/AVC", "HEVC/H.265" used descriptively; no certification marks without permission |

Business consequence: every machine with a GPU encoder is fully served, and software-encoder licensing
becomes a commercial decision rather than a launch blocker.

### 2.3 Other components

| Component | License | Note |
|---|---|---|
| `wgpu`, `winit`, `egui`, `cpal`, Rust crates | MIT/Apache-2.0 mostly | audit with `cargo-deny` in CI |
| `cosmic-text`, `rustybuzz`, `glyphon` | MIT/ISC | — |
| Fonts, LUTs, sample media | must be redistributable | asset license register from Phase C |
| OFX / VST3 SDKs | not used in v1 | deferred with the plugin tiers ([09](09-effects-and-extensions.md)); review terms before adding |

## 3. Platform compliance (Windows-first)

- **Windows**: code signing with an EV or Azure Trusted Signing certificate; portable zip plus MSIX;
  consistent signing to build SmartScreen reputation. This is the only v1 delivery target.
- **macOS** (follow-up): Developer ID signing, notarization, hardened runtime, Universal binary.
- **Linux** (post-1.0): Flatpak primary, AppImage optional; system FFmpeg.

## 4. Privacy and data protection

- Crash reporting is opt-in, off by default, disclosed at first run, with a machine-readable setting and a
  CLI override. No media content, file names, or project names leave the machine.
- Bundles contain command types/hashes only ([08 §5](08-performance-reliability.md)).
- GDPR/CCPA: data inventory and retention (90 days for crash reports); deletion request path. No
  third-party trackers in the app.

## 5. Regulatory notes

- Keyboard completeness is a Phase C gate; screen-reader certification is a post-1.0 item.
- The product's cryptography (TLS for future remote transports) is standard publicly available software;
  no unusual export-control obligations anticipated.

## 6. Standing legal tasks

| When | Task | Owner |
|---|---|---|
| Before Phase B ships | Codec posture sign-off; FFmpeg LGPL package reviewed | Legal + developer |
| Before Phase F | Agent threat model reviewed (local path allow-lists, job caps) | Developer + security review |
| Before Phase H | Trademark search ("Arcline"); Windows signing; privacy policy | Legal + developer |
| Every release | `cargo-deny` license audit; dependency CVE review; asset license register | Developer |
