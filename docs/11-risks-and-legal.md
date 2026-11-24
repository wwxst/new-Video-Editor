# 11 — Risks & Legal

Two sections: the engineering risk register, and the legal/licensing posture that constrains what can ship.
The second is not boilerplate — codec licensing has killed more video products than bugs have.

## 1. Risk register

Scoring: likelihood (L) and impact (I) on 1–5; exposure = L×I.

| # | Risk | L | I | Exposure | Mitigation | Trigger / contingency |
|---|------|---|---|----------|------------|------------------------|
| R1 | **Codec licensing cost or refusal** (H.264/HEVC encode) | 3 | 5 | 15 | Ship hardware-only H.264/HEVC encode + AV1 (royalty-free) software encode in the base product; negotiate software-codec licenses only where business requires; legal review at P1 | If license cost exceeds threshold: AV1-default + hardware H.264 only |
| R2 | **FFmpeg LGPL compliance failure** | 2 | 4 | 8 | LGPL-only build config, dynamic linking, published build scripts and source offer, CI check that forbids `--enable-gpl` components | Legal sign-off required before P1 export ships |
| R3 | **GPU driver bugs / device loss in the field** | 4 | 3 | 12 | Device-loss recovery, capability tiers, per-driver verdict cache, Tier-C reference fallback, crash telemetry by driver | Blacklist specific driver versions in a signed config list |
| R4 | **Widget layer ramp-up exceeds schedule** | 3 | 4 | 12 | Fixed widget inventory, buy layout/text/a11y, egui dev-shell until P2, 2 UI engineers from P2 | Fallback to a license-compatible toolkit behind the same widget interface; ADR-0007 records reversal cost |
| R5 | **Agent writes cause destructive edits** | 3 | 4 | 12 | Journal + undo + dry-run + capability scopes + pre-session snapshot; render output confined to job root | Session "rewind to revision" UI; audit log export |
| R6 | **Performance targets missed on integrated GPUs** | 3 | 3 | 9 | Quality governor with aggressive proxy use; half-res intermediates; explicit min-spec (Tier B) defined at P1 | Publish a supported-hardware list; degrade to proxy-first preview |
| R7 | **Determinism expectations misunderstood** | 2 | 3 | 6 | Contract stated in product docs and export report; reference mode available; cross-device tolerance documented | Support runbook for "my render differs" with report diagnostics |
| R8 | **Long-GOP seek still slow on spinning/network media** | 3 | 3 | 9 | Proxies mandatory above threshold; GOP cache; async proxy generation; clear UI indication | "Optimize media" one-click job |
| R9 | **Audio device variability (Bluetooth, exclusive mode)** | 4 | 2 | 8 | Master clock re-anchor, device-change handling, latency calibration UI, null-device CI | Documented troubleshooting; WASAPI exclusive opt-in |
| R10 | **OFX plugin incompatibility** | 3 | 3 | 9 | Subprocess isolation, conformance suite, curated vendor list, version pinning | Mark plugin "experimental"; bypass on failure |
| R11 | **Scope creep in effects/color** | 4 | 3 | 12 | Explicit effect list per phase; "no node graph in v1" non-goal; plugin tiers absorb the long tail | Cut list pre-agreed per phase |
| R12 | **Key-person dependency (render/GPU)** | 2 | 4 | 8 | Two owners per critical subsystem from P2; ADRs + design reviews; test-first culture | Contractor bench |
| R13 | **Security incident via malicious media or project file** | 2 | 4 | 8 | Decoder/plugin subprocesses, fuzzing, path canonicalization, no auto-exec from projects | CVE response process; signed config revocation |
| R14 | **Schedule slip compounding** | 3 | 3 | 9 | Phase gates with testable exit criteria; critical path identified; parallel tracks planned | Descope P4 items (masks, tracking-lite) before touching P2 quality |

The top three exposures (R1, R3/R4, R11) are the ones to revisit at every phase gate.

## 2. FFmpeg and codec licensing

**Stated plainly: this section requires counsel. It records the engineering constraints and the intended
posture, not legal advice.**

### 2.1 FFmpeg

FFmpeg is LGPL-2.1-or-later by default; certain components (`--enable-gpl`) — including x264 and x265 — pull
the whole build under GPL. Arcline's posture:

- Build FFmpeg with **no GPL components**, dynamically link the shared libraries, ship on Windows/macOS as
  separate shared libraries, and provide the LGPL compliance package: exact source, build scripts,
  modification list, and relink instructions, published per release.
- On Linux, prefer system FFmpeg (distro packaging already handles licensing) via a runtime dependency,
  with our own LGPL build as a Flatpak-provided fallback.
- Pin the FFmpeg version, track CVEs (a weekly dependency review is a standing task), and treat decoder
  crash rates as a tracked metric.

### 2.2 Patents (the harder problem)

H.264/AVC and HEVC are covered by patent pools (Via LA/MPEG LA, Access Advance, and others) whose terms
apply to *encoding and decoding products*, not just the software license. AV1 was designed to be
royalty-free (AOMedia) and is the only major modern codec with a clear free path.

Intended posture:

| Capability | Posture |
|---|---|
| Decode H.264/HEVC/AV1/ProRes | expected to be covered by platform licenses; confirm with counsel |
| Encode H.264/HEVC | **hardware encoders only** in the base product (NVENC/QSV/AMF/VideoToolbox are licensed at the driver/platform level) |
| Encode AV1 | SVT-AV1 (BSD) — royalty-free, default recommended for web |
| Encode ProRes/DNxHR | editorial mezzanine formats; review Apple/Avid trademark and licensing terms; used widely by open-source tools but not free of considerations |
| Software H.264/HEVC encoders (x264/x265/openh264) | **not shipped** unless a commercial license is obtained; `libopenh264` (BSD, Cisco patent coverage for Cisco-distributed binaries) is an option for builds where the business accepts its terms |
| Naming | "H.264/AVC" and "HEVC/H.265" used descriptively; do not use certification marks without permission |

The business consequence is concrete and is why Phase 1 exit criteria say "hardware encoders": the product
is fully functional on every machine with a GPU encoder, and the software-encoder question becomes a
commercial decision rather than a launch blocker.

### 2.3 Other components

| Component | License | Note |
|---|---|---|
| `wgpu`, `winit`, Rust crates | MIT/Apache-2.0 (mostly) | audit via `cargo-deny` in CI |
| OpenFX headers | BSD-style | compatible |
| VST3 SDK | GPLv3 or Steinberg proprietary | hosting terms reviewed before P5; subprocess isolation is also a licensing boundary |
| Fonts, LUTs, sample media | must be redistributable | asset license register maintained from P2 |
| `rustybuzz`/`cosmic-text`/`taffy` | MIT/ISC | — |

## 3. Platform compliance

- **Windows**: code signing with an EV or Azure Trusted Signing certificate; MSIX and portable zip; SmartScreen
  reputation handled by consistent signing.
- **macOS**: Developer ID signing, notarization, hardened runtime; `VideoToolbox` and `Metal` require no
  special entitlement; no sandbox requirement for direct distribution.
- **Linux**: Flatpak as the primary channel (bundles our LGPL FFmpeg; sandbox portal access for files);
  AppImage for portable use; tarball for distro packagers.

## 4. Privacy and data protection

- Telemetry is **opt-in for analytics, opt-out for crash reports**, disclosed at first run, with a
  machine-readable setting and a CLI override; no media content, file names, or project names ever leave
  the machine.
- Crash bundles include command *types* and hashes, not document content.
- GDPR/CCPA: a documented data inventory, retention period (crash reports 90 days), and a deletion request
  path. No third-party trackers in the app.

## 5. Accessibility and other regulatory notes

- Target WCAG-equivalent behavior for the app's own accessibility tree; EU Accessibility Act expectations
  and ADA are reviewed before 1.0. Keyboard completeness is a P2 gate precisely because it is both an
  accessibility requirement and a scripting convenience.
- Export control: the product's cryptography (TLS for optional network transports) is standard, publicly
  available software; no unusual obligations anticipated.

## 6. Standing legal tasks

| When | Task | Owner |
|---|---|---|
| Before P1 export ships | Codec posture sign-off; FFmpeg LGPL package reviewed | Legal + architect |
| Before P3 | Agent security threat model reviewed externally; audit-log retention policy | Security + legal |
| Before P5 | OFX/VST3 hosting terms review; plugin distribution policy | Legal |
| Before P6 | Trademark search ("Arcline" may collide); installer signatures; privacy policy | Legal + release |
| Every release | `cargo-deny` license audit; dependency CVE review; asset license register | Release engineer |
