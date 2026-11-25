# HANDOVER — foundation checkpoint (Phases A, P done; B functionally complete; P0 fixed; FFmpeg 9.1)

Checkpoint date: 2026-11-25. Supersedes the earlier WIP handover of `720a99a`.

**State: the workspace builds against the pinned FFmpeg 9.1 package, clippy exits 0 (pre-existing lint
warnings only), and 136 tests pass** (Windows 11, Intel integrated GPU; see §3 for the exact counts and the three-run evidence in
`reports/ffmpeg-9.1-differential.md`). All P0 audit defects are fixed, the FFmpeg 8.1.3 → 9.1
upgrade is implemented (spec steps 0–3), and the follow-up work closed twelve more audit findings
(F-06, F-07, F-08, F-11, F-12, F-13, F-14, F-24, F-26, F-28, F-29, F-31). The remaining findings
are deferred with a written reason and recommendation in
[reports/audit-follow-up-2026-11-25.md](reports/audit-follow-up-2026-11-25.md). Phase B closure
batch 2 adds HEVC export, the malformed-media corpus with error convergence, and CI
([report](reports/phase-b-exit-2026-11-25.md)). The upgrade and the follow-up commits are merged
into `main` (`e5d444f` plus the follow-up commits). Architecture docs under `docs/` remain the
source of truth; this file records implementation status, deviations, and how to continue.

> **Read first: [audit of `88e0707`](reports/audit-2026-11-25-88e0707.md)** (2026-11-25) and its
> [follow-up status table](reports/audit-follow-up-2026-11-25.md). Its P0 list (F-01, F-02, F-03,
> F-04, F-05, F-09) is fixed with regression tests; the FFmpeg 9.1 upgrade (F-19, F-20, F-21, plus
> the shim/tools parts of F-09) is recorded in [third_party/ffmpeg](third_party/ffmpeg); F-06, F-07
> and F-08 (accuracy) and F-11…F-14, F-24, F-26, F-28, F-29, F-31 were fixed in the follow-up.

---

## 1. Getting a working environment (Windows)

Everything is user-local; no admin rights or system PATH changes were made.

| Component | Location | Why |
|---|---|---|
| Rust 1.100.0, `x86_64-pc-windows-gnu` | `~/.rustup/toolchains/stable-x86_64-pc-windows-gnu/bin` | Only toolchain on this machine. `~/.cargo/bin` proxies do not exist. |
| MinGW-w64 GCC 16.2.0 UCRT (WinLibs r3) | `~/.local/toolchains/mingw64` | `windows-sys` 0.61 links via raw-dylib and **needs `dlltool.exe`**; also the C compiler for the libav shim. Download SHA-256 `3c69305081e4a647144339a936244fcf7d3d9e558319e1ad7be2cd2886d4b6a1` (matched the published checksum). |
| FFmpeg **9.1 "Gauss" LGPL 2.1+** (in-repo build) | `~/.local/toolchains/ffmpeg-9.1-lgpl` | Headers + GNU import libs for the shim; LGPL `ffmpeg`/`ffprobe` for subprocess jobs; built by [third_party/ffmpeg/build.sh](third_party/ffmpeg/build.sh) from the verified official tarball (`identity 9.1-arcline-f414c1c54e`) |
| FFmpeg **8.1.3 LGPL** (previous pin, rollback only) | `~/.local/toolchains/ffmpeg-8.1-lgpl` | Kept installed for rollback (spec §11/AC-15); `scripts/dev-env.sh` prefers 9.1, `ARCLINE_FFMPEG_DIR` overrides |
| MSYS2 UCRT64 (build host for FFmpeg) | `~/.local/toolchains/msys64` | GCC 16.2.0 rev7, binutils 2.47, NASM 3.02, pkgconf 3.0.11; exact package lock in [third_party/ffmpeg/msys2-packages.txt](third_party/ffmpeg/msys2-packages.txt) |

```bash
. scripts/dev-env.sh        # Rust + MinGW + ARCLINE_FFMPEG_DIR, FFmpeg bin first on PATH
cargo build --workspace
cargo test --workspace
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets   # see §6.1 for why
cargo run -p arc-app --release --example embed_host          # embedding PoC report (opens windows ~30 s)
```

`scripts/dev-env.sh` puts the LGPL FFmpeg `bin/` **before** the GPL "essentials" build that is also on this
machine's PATH. Never let tests or exports pick up the GPL build.

On another machine you need the same three things: Rust (GNU or MSVC), `dlltool` (GNU target only), and an
LGPL FFmpeg dev package pointed to by `ARCLINE_FFMPEG_DIR` (`include/`, `lib/`, `bin/`). The MSVC target is
untested — see §6.

**FFmpeg pin status.** FFmpeg **9.1** is the pinned, shippable version since 2026-11-25
([spec](docs/specs/ffmpeg-9.1-upgrade.md), implemented; [build pipeline](third_party/ffmpeg/README.md)):

- **D1** LGPL version 2.1 or later (no `--enable-version3`, no version-3 libraries); **D2** Media
  Foundation excluded (`h264_mf`/`hevc_mf` absent); **D3** in-repo source build from the verified
  official tarball (SHA-256 `1a4d394a…2433`, release key `FCF986EA…58D8`), no prebuilt package used;
  **D4** the P0 audit fixes landed first. The same decisions are in
  [manifest.json](third_party/ffmpeg/manifest.json).
- The build has **no GPL/nonfree** components, only the `file` and `pipe` protocols, no
  `handle`/`libcurl`, and no HLS/DASH/concat demuxers. Its manifest records every shipped file's
  SHA-256, the host toolchain and an SBOM; `third_party/ffmpeg/verify.sh` re-runs the checks.
- The shim is pinned at compile time (lavu 61, lavc 63, lavf 63, sws 10, swr 7) and the process
  checks the loaded versions and licences at startup; mismatched DLLs/tools are refused unless the
  developer sets `ARCLINE_ALLOW_ANY_FFMPEG=1` (recorded in the export report). See
  [spec §4.5](docs/specs/ffmpeg-9.1-upgrade.md).
- After swapping the package, `cargo clean -p arc-media` is no longer required (the build script
  tracks the headers and import libraries), but it is harmless.
- The 8.1.3 package remains installed for rollback (AC-15); point `ARCLINE_FFMPEG_DIR` at it and
  revert the version-pin commit. Project files are unaffected (no schema change).
- The declared `rust-version = "1.85"` is still too low: `egui` 0.37 needs 1.95 (F-26). Use 1.100.

## 2. What exists

```
crates/
  arc-core    time, ids, model, effects (manifests as data), error, command, engine, journal, store, runtime
  arc-media   libav shim (csrc/arc_av.c) + Rust wrappers, subprocess probe/import check, encoder, fixtures
  arc-render  builtin effect manifests, pure frame planning, wgpu direct compositor + WGSL
  arc-audio   sample-accurate mixer (block-size independent), WAV writer
  arc-cli     inspect (describe, text timeline), export job, JSON-RPC session, `arcline-cli` binary
  arc-app     egui-winit + egui-wgpu host loop (top-level / embedded child / pop-out), embed channel, PoC workload
              examples/embed_host.rs — Win32 reference host + PoC measurement
docs/poc/embedding-poc-2026-11-25.md — PoC result (GO)
third_party/ffmpeg/  pinned FFmpeg 9.1 build recipe, configure profile, manifest, SBOM, licences, package lock
scripts/dev-env.sh
```

### Phase status (docs/10)

| Phase | Status | Evidence |
|---|---|---|
| **A Spine** | **Done** (dependency-rule CI check still missing) | Exit criteria met: 10,000-case property test, timecode boundary tests, `new` + `apply` produce valid containers |
| **P Embedding PoC** | **Done — GO** | All automated criteria pass (docs/poc, ADR-0014 outcome) |
| **B Headless pipeline** | **Exit criteria met** | Frame-exact `import → edit → render` through FFV1, H.264 (`h264_qsv`), HEVC (`hevc_qsv`), AV1 (`libsvtav1`), ProRes and DNxHD; long-GOP/B-frame/MPEG-TS random access is frame-exact (F-08) and lossy/resampled audio survives the mixer's block pattern (F-06/F-07); a generated malformed corpus converges to typed errors and a mixed good/broken export completes (goal B); CI is green on all four jobs — core on ubuntu/windows/macos and the Windows media job — in runs 7 and 9 (goal C). The `Quarantined` branch is covered by a generated stub tool (crash/hang/garbage, bounded time) — see [Phase B closure](reports/phase-b-exit-2026-11-25.md) |
| **C Compositor + shell** | Compositor done; **preview pipeline (C1) and scheduler (C2) done; GUI editor not started** | C1: `arc_cli::preview` composes frames identical to the export path, with a byte-bounded decoded-source cache ([report](reports/phase-c-c1-2026-11-26.md)). C2: `arc_cli::preview_scheduler` is latest-wins with scrub/full quality, cancel and bounded queues, tested with a stub pipeline ([report](reports/phase-c-c2-2026-11-26.md)). `arc-app` still shows the PoC workload |
| **D Audio** | Mixer + export done; **real-time playback (cpal) not started** | |
| **E Lifecycle** | Journal, recovery, atomic saves, forced-kill done; **crash breadcrumbs, quarantine on relaunch, hang watchdog, relink not started** | |
| **F Agent surface** | **Started**: JSON-RPC session with 11 of 15 methods; **MCP, local IPC, live attach, control methods, notifications over RPC not started** | |
| J, G, I, H | Not started | |

### Commands available today

```
arcline-cli new <p.arcline> [--name N] [--fps 29.97] [--res 1920x1080]
arcline-cli describe <p> [--exact]          JSON project.describe
arcline-cli timeline <p>                    text timeline (docs/schema/timeline-text.abnf)
arcline-cli apply <p> [--file f.json]       JSON command batch from file/stdin, then save
arcline-cli probe <media>                   subprocess import check → ok | quarantined | offline
arcline-cli import <p> <media...>           import check + asset.import, paths relative to the project
arcline-cli render <p> -o out --codec ffv1|h264|hevc|av1|prores|dnxhd [--from TC --to TC] [--snapshot doc.json]
arcline-cli rpc [--client id]               JSON-RPC 2.0 session on stdio
arcline [--embed-parent HWND --embed-channel stdio] [--poc-seconds N]
```

stdout carries exactly one JSON document per command; errors print `{"error":{kind,...}}` and exit 2; usage
errors exit 64. `render` also writes `<output>.report.json`.

JSON-RPC methods implemented: `project.open` (incl. `create`), `project.describe`, `project.apply`,
`project.save`, `history.undo`, `history.redo`, `inspect.timeline` (with `range`), `effect.list`,
`job.submit` (`kind: "render"`, runs a child `arcline-cli render --snapshot`), `job.status`, `job.cancel`.
Missing versus docs/06 §4.1: `history.list`, `inspect.frame`, `control.request`, `control.release`, and pushed
notifications.

## 3. Test results (all green, 136 tests, 1 ignored)

Three consecutive full `cargo test --workspace` runs and a clean clippy run on the FFmpeg 9.1 switch
are recorded in [reports/ffmpeg-9.1-differential.md](reports/ffmpeg-9.1-differential.md) §5; the
per-crate table below is the current suite.

| Crate / suite | Tests | What is proven |
|---|---|---|
| arc-core unit (time, ids, model, lock errors) | 16 | Exact time; all 8 rates; NDF/DF vectors and rejections; **exhaustive 24 h DF round trip**; sample mapping; canonical JSON |
| arc-core `tests/engine.rs` | 14 | Commands, overlap/off-boundary/unknown errors with paths, atomic batches, stale revision, dry run, idempotency, `internal.restore` ban, trim/split/ripple/transitions, manifest validation, undo/redo hashes, control + auto-takeover, rollback of unconfirmed batches, journal replay, **replayed redo has no phantom entry (F-12)**, **speed ≠ 1 split/trim (F-13)**, **locked track removal refused (F-29)** |
| arc-core `tests/property.rs` | 1 (10,000 cases) | **Invariant 7**: apply → undo → identical canonical hash; redo → identical; every committed state validates |
| arc-core `tests/journal.rs` | 9 | Torn tail at every byte offset, CRC/magic/length corruption, short writes, mid-record failure truncation, failed truncate, durable `fsync` failure, group-flush failure, CRC32C vectors |
| arc-core `tests/store.rs` | 8 (+1 helper) | Container round trip, future schema refused, save/reopen/clean close, lock, torn-tail recovery with `.rejected`, ack + notifications, **forced kill via child-process `abort()` at 1, 29, 30, 31, 75 edits**, **snapshot rotation publishes only with its journal (F-14)** |
| arc-core `tests/recovery.rs` | 7 | P0 recovery/creation regressions: rejected replay (F-01), evidence split, create over locked/existing project (F-04), stale sidecar and foreign snapshot id checks (F-05) |
| arc-media unit + `tests/decode.rs` | 11 + 3 | ffprobe mapping, runtime identity, ffprobe parse; **frame-accurate in-process decode** (sequential, random, repeated, backward) at 29.97 and 25 fps for FFV1 in MKV; sample-addressed audio; import check ok/offline |
| arc-media `tests/malformed.rs` | 1 | Generated malformed corpus (9 fixtures under 2 MB: empty, text, truncated, corrupt header/index/payload): the import check refuses every unreadable file and the decode boundary returns typed errors, never a crash |
| arc-media `tests/quarantine.rs` | 1 | A generated stub `ffmpeg`/`ffprobe` (proxying to the real tools) simulates a crash (0xC0000005), a hang (killed at the 20 s probe timeout) and garbage output: `Quarantined` vs `Offline` are distinguished, the process survives and real media still imports afterwards |
| arc-media `tests/param_change.rs` | 2 | Mid-stream resolution change (F-02) scaled, not overflowing; MP2 stereo→mono (F-03) resampled safely |
| arc-media `tests/audio_blocks.rs` | 2 | The mixer's 4096+1 block pattern is bit-identical to one continuous read for FLAC 48/44.1 kHz, MP3, Ogg/MKV Opus and AAC (**F-06**) |
| arc-media `tests/audio_anchor.rs` | 2 | One Opus stream remuxed to Ogg and MKV decodes bit-identically; reads at 20/50/80 s align with the continuous decode (**F-07**) |
| arc-media `tests/seek_accuracy.rs` | 4 | Every frame exact for MPEG-2 open GOP/MKV, MPEG-2/TS with a 1.433 s start, MPEG-4 Part 2/MP4 with B-frames and H.264, sequential, backward, repeated and from fresh decoders (**F-08**) |
| arc-media `tests/paths.rs` | 2 | Local-file-only opening; URL/device paths refused before I/O; spaces/non-ASCII file: URLs |
| arc-media `tests/runtime.rs` | 3 | Runtime identity/licence/ident, verified tools and encoder set, `libsvtav1` encode+decode |
| arc-media `tests/differential.rs` | 1 | FFmpeg 8.1.3-vs-9.1 corpus harness (skipped unless `ARCLINE_CORPUS_DIR` is set; step-1 baseline committed) |
| arc-render unit + `tests/plan.rs` + `tests/render.rs` | 1 + 1 + 5 | Manifests; frame planning incl. dissolve weights and handles; GPU bit-exact determinism, 1-LSB pass-through, color correct within 2 LSB of the CPU reference, transforms, linear-light opacity/dissolve, every effect runs or is reported |
| arc-audio unit | 3 | **Block-size parity** (1…4096 identical) with a synthetic source, positions/gain/pan/fades/varispeed, WAV header. Parity through the real decoder for AAC/MP3/Opus/resampled media still shows the P2 F-06 defect |
| arc-cli `tests/preview.rs` | 14 | Phase C C1: preview frame equals the export frame (single layer and two-track transformed overlay), half quality at native source decode, decoded-source cache hits/steps and byte-budget eviction, quality switch, unavailable asset/title warnings, out-of-range/missing-sequence typed errors, relinked asset reopens |
| arc-cli `tests/preview_scheduler.rs` | 6 | Phase C C2: latest-wins coalescing (200 requests → 2 renders), slow render never queues the newest, settle upgrade to full quality, cancel drops pending/in-flight work, shutdown/drop leave no worker |
| arc-app unit | 1 | PoC embed messages gated on a PoC session (**F-31**) |
| arc-cli `tests/cli.rs`, `render_e2e.rs`, `rpc.rs` | 4 + 6 + 4 | CLI contract incl. `new --overwrite`; import checks and scheme/device rejection; **end-to-end: every output frame verified exact** (FFV1, H.264, ranges) with audio and the §4.5 `ffmpeg` report block; AV1/ProRes/DNxHD decode; RPC semantics, create/locked/existing, out-of-process render job |

Not part of `cargo test`: the embedding PoC (`--example embed_host`). Run it manually; result recorded in
`docs/poc/embedding-poc-2026-11-25.md`.

**Now covered by tests** (added with the P0 fixes, the upgrade and the follow-up): recovery after a
rejected replay record (F-01), creation over an existing or locked project (F-04), stale/foreign
sidecars (F-05), mid-stream resolution and channel changes (F-02, F-03), non-file asset paths and
client-supplied `asset.import` (F-09), lossy and resampled audio through the real decoder in the
mixer's block pattern (F-06), container-independent Opus alignment (F-07), long-GOP / B-frame /
MPEG-TS random access (F-08), replay of a redo (F-12), speed ≠ 1 split and in-trim (F-13), snapshot
rotation failure (F-14), lock-error mapping (F-24), locked-track removal (F-29), PoC embed gating
(F-31).

**Still not covered:** the deferred findings in
[reports/audit-follow-up-2026-11-25.md](reports/audit-follow-up-2026-11-25.md) §3 — path
allow-lists (F-10), engine panic containment (F-15), rotation/SAR (F-16), streaming export audio
(F-17), oversized sources (F-18), VFR-flagged media (F-22), mid-stream colour tags (F-23),
job/temp-file hygiene (F-27), idempotency binding (F-30), VBR probe duration (F-32).

## 4. Bugs found and fixed by the tests (keep these tests)

1. DF timecode past the 24 h label range produced an unparseable `24:00:00;00` → now an error.
2. `split_clip` generated effect IDs at apply time, so redo/replay diverged → IDs are part of the resolved command.
3. MKV stores 1 ms timestamps, so strict `pts ≤ target` picked the previous frame → decoder aims at the frame middle.
4. Runtime replied before publishing the confirmed revision (race under parallel load) → publish first.

## 5. Implementation decisions and deviations not yet reflected in `docs/`

Fold these into docs/02–06 (or an ADR) when convenient; none contradicts an ADR.

| Topic | Decision in code |
|---|---|
| Undo | Engine-only `internal.restore` command with the prior tracks/markers/changed assets/name; journaled; clients get `forbidden_op` |
| Resolution | `apply_command` rewrites commands in place (IDs, exact times, effect defaults, split effect IDs); the resolved form is journaled and redone |
| Validation | Full `validate_project` after every batch (O(n)); not yet benchmarked against the 200 µs budget |
| Model | `sequences` map with exactly one sequence; track z-order = vector position (no stored `index`); clip source is `asset` **or** `title`; transforms are static (not keyframable); effect params are f64 `{"static"}` / `{"curve"}` |
| Ripple | `none`/`track`/`all` on `remove_clip` and out-edge `trim_clip`; in-edge ripple → `unsupported_ripple` |
| Audio clips | Must be frame-aligned (no `allow_subframe` yet); fades are sample-grained; a video asset with audio may sit on an audio track; no linked A/V clips |
| Speed ≠ 1 | `start`/`duration` stay frame-aligned; `source_in` may be any exact time when `speed != 1` (it is derived from a frame-aligned timeline position), and must stay on the frame grid at speed 1 (F-13) |
| CLI undo | Removed: history is session-scoped; use `arcline-cli rpc` |
| Decode | In-process libav through `csrc/arc_av.c` (opaque handle; no FFmpeg structs in Rust). YUV→RGB happens in swscale on the CPU (BT.709/601 by stream tags) — **docs/03 §10 GPU YUV upload is a later optimization**. Video seeks back off until a frame at or before the target is decoded (F-08); audio reads come from a one-second history buffer, a jump seeks 12,000 frames early and decodes the pre-roll (F-06) |
| Import check | `ffprobe` + `ffmpeg -t 1 -f null` at start/middle/end in child processes with 20 s timeouts; Windows NTSTATUS exit codes count as crashes |
| Export | `arcline-cli render` decodes in-process (it is already a child of the GUI/session), encodes by piping RGBA to an `ffmpeg` child; encoder chosen by a one-frame trial (here `h264_qsv`; fallbacks `h264_mf`, `libopenh264`). Never x264 |
| Render input | `render` reads the saved container, or `--snapshot` (what `job.submit` writes); it never takes the project lock |
| Effects | GPU: color correct, vignette, chroma key, sharpen, gaussian blur. **LUT is a manifest only** (no LUT asset parameter yet) — reported as unsupported, passed through |
| Titles | Planned but **not rendered** (no text renderer yet); export skips them with a warning in the report |
| Audio pan | Constant-power normalized to unity at center (hard pan = +3 dB on one side) |
| Timeline → samples | `Round::Nearest` (29.97 frames are 1601.6 samples) |
| Embed channel | PoC uses stdio; production moves the same messages onto the named pipe (docs/13 §3) |
| App loop | `egui-winit` + `egui-wgpu` directly, no `eframe` (ADR-0007 amendment) |
| macOS lock | `SessionLock` is strong on Windows (no-share handle), weak elsewhere (Linux `/proc` check, macOS unchecked) |
| Not yet implemented, although documented (audit §5.2) | import-check cache by content hash; rotation in the compositor; `catch_unwind` / engine panic containment; `--root`/`--output-root`; `job.status` progress and `queued`; `MOVEFILE_WRITE_THROUGH` on rename; schema fixture `fixtures/schema/v7_min.arcline`. Each is deferred with a reason in [reports/audit-follow-up-2026-11-25.md](reports/audit-follow-up-2026-11-25.md) §3 |
| Blur radius | manifest range is 0–200 px; the shader caps the kernel at 64 taps per side, so radii above 64 px use a truncated kernel |
| Software H.264 | the encoder *candidates* still list `h264_mf`/`libopenh264` as fallbacks, but D2 excludes Media Foundation and the build excludes `libopenh264`, so neither exists in the shipped 9.1 package (audit F-19 closed in the build). Never x264 |
| FFmpeg | 9.1 in-repo build (D1 LGPL v2.1+, D2 no Media Foundation, D3 source build, D4 P0 first); compile-time ABI pin + run-time version/licence check; tools resolved from the bundle only. Details: [third_party/ffmpeg](third_party/ffmpeg), §9 |

## 6. Known issues and unverified areas

1. **rustc 1.100 incremental ICE under clippy** for `arc-core` (`evaluate_obligation`). Workaround:
   `CARGO_INCREMENTAL=0 cargo clippy …`, or `cargo clean -p arc-core`. Builds and tests are unaffected.
2. **MSVC target untested.** The shim should compile with `cl /std:c11` (compound literals are used); `build.rs`
   passes `-std=c11` only where supported — add `/std:c11` for MSVC if needed.
3. **CI is green** (`.github/workflows/ci.yml`, ADR-0011): runs 7 (`4b4be80`) and 9 (`1ea91e2`, current
   `main`) finished with **all four jobs success** — `core` on ubuntu/windows/macos (Rust 1.95 + clippy,
   FFmpeg-free crates) and the Windows `media` job (MSYS2 + the in-repo FFmpeg 9.1 build with caching,
   then the full workspace build/test/clippy under Rust 1.95 and the WARP software adapter). The two media
   causes from batch 3 are fixed: the FFmpeg package now installs where the cache/`ARCLINE_FFMPEG_DIR`
   expect it, and the GPU tests are serialized per test binary. Failures emit check-run annotations,
   readable without a token. The hardware-encoder tests skip themselves on machines without a GPU.
   Run links: [Phase B closure](reports/phase-b-exit-2026-11-25.md) §3.2.
4. **GPU tests need an adapter.** They ran on Intel integrated graphics here; on CI use the fallback adapter.
5. **Embedding PoC manual items open:** keyboard focus round-trip, IME placement, DPI change across monitors,
   Electron host (Electron not installed).
6. **macOS embedding:** winit 0.30 `with_parent_window` supports Windows and X11 only (risk R21).
7. `cargo` warns that workspace dependency `arc-cli` is unused (nothing depends on it yet; `arc-app` will).
8. The export copies every decoded frame (`to_vec`) per item per frame; fine for the foundation, a known
   optimization target.
9. Performance budgets (docs/08 §2) are **not yet measured**, except the PoC frame pacing.
10. **Audit defects.** Every finding is tracked in
    [reports/audit-follow-up-2026-11-25.md](reports/audit-follow-up-2026-11-25.md) with its commit and
    regression test, or with the reason it is deferred. Fixed: F-01 (journal restarts after a rejected
    record), F-02/F-03 (shim parameter changes), F-04 (lock-before-create + additivity), F-05 (sidecar
    identity), F-06 (audio served from a history buffer), F-07 (container-independent Opus alignment),
    F-08 (seek back-off until a frame at or before the target), F-09 (local files only + session-layer
    import checks), F-12 (replayed redo pops the redo stack), F-13 (speed ≠ 1 source_in rule), F-14
    (snapshot published only after its journal opens), F-19/F-20/F-21 (licence posture, reproducible
    pinned build, runtime identity), F-24 (lock-error mapping), F-26 (MSRV), F-28 (EAGAIN packet kept),
    F-29 (locked track removal refused), F-31 (PoC messages gated).

    Deferred with a written reason and recommendation (follow-up report §3): F-10 (path allow-lists),
    F-15 (panic containment), F-16 (rotation/SAR), F-17 (streaming export audio), F-18 (oversized
    sources), F-22 (VFR bias), F-23 (colour-tag changes), F-27 (job/temp hygiene), F-30 (idempotency
    binding), F-32 (VBR probe duration), F-33 (performance debt).

11. **UNC asset paths are trusted.** A project file received from someone else can point at a UNC
    path (or a mapped network drive), and the in-process decoder will open it, which can trigger
    outbound SMB authentication (NTLM credential leak). Network-share media is intentionally
    supported, so this is bounded with the `--root`/`--output-root` allow-list work (audit F-10),
    not by rejecting UNC paths now.

## 7. Next steps (in roadmap order)

**P0 — before Phase C** (audit §10): **done** on `fix/p0-audit-88e0707` (F-01, F-04, F-05, F-02,
F-03, F-09, plus the four architect follow-ups: sidecar identity edge case, separate replay-rejection
evidence, UNC normalisation, device-namespace rejection).

**P1 — FFmpeg 9.1 upgrade** ([spec](docs/specs/ffmpeg-9.1-upgrade.md)): **done** on
`feat/ffmpeg-9.1-upgrade` (decision record and evidence in §9).

**P2 — before Phase C preview work depends on them:** **done** — F-06/F-07 (audio accuracy), F-08 (seek
accuracy), F-12, F-13, F-14 (see the follow-up report). Remaining P2: F-10 (`--root`/`--output-root`,
which also bounds the UNC/NTLM exposure noted in §6.11) and F-15 (panic containment with a
degraded-mode contract); CI workflow, dependency-rule check and `cargo-deny`.

**Phase B closure (batch 2, `2026-11-25`):** HEVC export, the malformed-media corpus with decode
error convergence and the CI workflow are done — see
[reports/phase-b-exit-2026-11-25.md](reports/phase-b-exit-2026-11-25.md). The only exit criterion
piece still open is a fixture that crashes or hangs the import check (the `Quarantined` branch).

**Phase C — the editor (largest remaining piece)** — plan: [docs/specs/phase-c-plan.md](docs/specs/phase-c-plan.md); C1 (preview pipeline) and C2 (scheduler) done.
1. Replace the PoC workload in `arc-app` with the editor shell: `egui_dock` layout, media bin, inspector
   generated from `EffectManifest`, viewer showing `arc_render::Renderer` output via an egui-wgpu user
   texture (`Renderer::with_device` shares egui's device).
2. Timeline canvas: paint, virtualization, LOD, hit testing, move/trim/split/snapping → command batches via
   `EngineHost::apply_async` (never block the UI thread).
3. Preview decode: one `VideoDecoder` per active asset on worker threads, frame cache keyed per docs/03 §3,
   two-level quality fallback.

**Phase D** — `cpal` output with an SPSC ring fed by `Mixer::process`; audio-master clock; parity test at the
parity tap (docs/05 §7).

**Phase E** — crash breadcrumbs + quarantine on relaunch, hang watchdog with the 2-thread limit and
decode-safe mode (docs/04 §1.2), relink, GUI export via `arcline-cli render --snapshot`.

**Phase F** — finish JSON-RPC (`history.list`, `inspect.frame`, `control.*`, notifications), MCP server
(`arcline-cli mcp`, 23 tools mapped in docs/06 §6.1) on top of `rpc::Session`, the named-pipe transport and
consent prompt, live attach.

**Cross-cutting**
- Add the CI workflow (§6.3).
- Text renderer for titles/subtitles (`cosmic-text` + `glyphon`), LUT asset parameter.
- Fold §5 into the docs; update `docs/schema/CHANGELOG.md` if the model changes (bump `SCHEMA_VERSION` and
  add a migration + fixture).

## 8. Development rules that the code already enforces

- All edits go through `Command` + `Engine`; never mutate a `Project` directly. New commands need
  resolution (IDs/times written back), validation, and property-test coverage.
- `arc-core` has no GPU/FFmpeg/UI dependencies; `arc-render` and `arc-audio` never depend on FFmpeg;
  `arc-media` never on UI.
- Rust never touches FFmpeg structs: extend `csrc/arc_av.c` with opaque functions instead.
- `unsafe` is denied workspace-wide; the only exceptions are `arc-media/src/libav.rs`, `arc-app/src/embed.rs`,
  the `with_parent_window` call in `arc-app/src/app.rs`, and the reference host — each with `SAFETY` comments.
- Child processes get argument vectors, never shell strings.
- No GPL encoders in shipped paths; keep `ARCLINE_FFMPEG_DIR` pointed at the pinned FFmpeg 9.1 LGPL
  package. Extend `third_party/ffmpeg/` and the manifest for any change to the media build.
- Tests use generated fixtures only (`arc_media::fixtures`), never third-party media.
- The shim must never trust stream parameters for buffer sizes or converter setup: frames can change size,
  format and channel layout mid-stream (audit F-02, F-03). Treat document asset paths as untrusted input
  (F-09): the shim opens `file:` URLs with `protocol_whitelist=file`, and the session layer rejects
  URL/protocol/device paths and runs the import check itself.

## 9. FFmpeg 9.1 upgrade record (P1)

**Decisions (spec §10, defaults taken):**

| # | Decision | Recorded as |
|---|---|---|
| D1 | LGPL version 2.1 or later; no `--enable-version3`; no version-3 libraries | profile + manifest `decisions.D1`; runtime accepts only `LGPL version 2.1 or later` |
| D2 | Media Foundation excluded; no `h264_mf`/`hevc_mf` | profile (`--disable-autodetect`, no `--enable-mediafoundation`) + manifest |
| D3 | In-repo source build from the verified official tarball; no prebuilt package | `third_party/ffmpeg/build.sh`, `manifest.json` source/signature fields |
| D4 | P0 audit fixes land before the version switch | branch history: `fix/p0-audit-88e0707` merged before the switch commit |

**Build:** FFmpeg 9.1 "Gauss" from `ffmpeg-9.1.tar.xz` (SHA-256 `1a4d394a…2433`, signed by the
FFmpeg release key `FCF986EA…58D8`, commit `74e4393…`). Host: MSYS2 UCRT64, GCC 16.2.0 rev7,
binutils 2.47, NASM 3.02, pkgconf 3.0.11; exact package lock in `third_party/ffmpeg/msys2-packages.txt`.
Ident `9.1-arcline-f414c1c54e`. The package includes the LGPL/third-party notices, a CycloneDX SBOM and
the manifest with a SHA-256 for all 225 packaged files (`third_party/ffmpeg/verify.sh` re-checks AC-4).

**Deviations:** native MSYS2 UCRT64 build omits the spec's `--cross-prefix` (UCRT64 ships only
unprefixed binutils; the compiler target is still x86_64-w64-mingw32/UCRT); the external libraries
are linked dynamically and their runtime DLLs are bundled in `bin/` with notices, rather than being
statically linked as in the BtbN reference; the `webm_dash_manifest` demuxer remains (it reads a
local manifest; network protocols are disabled), while `hls`/`dash`/`concat` are absent.

**Verification:** three consecutive green full test runs, clean clippy, the 8.1.3-vs-9.1 differential
(AAC block-pattern decode differences explained by F-25/F-06), performance within ±10 %, and the
rollback rehearsal are recorded in [reports/ffmpeg-9.1-differential.md](reports/ffmpeg-9.1-differential.md)
together with the AC-1…AC-15 table.
**Follow-up after the merge (2026-11-25):** F-06 (audio block pattern), F-07 (Opus alignment) and
F-08 (seek accuracy) are fixed on `main` and add `audio_blocks`, `audio_anchor` and `seek_accuracy`
to the suite; the audio differential's AAC block-pattern difference is now explained and gone in the
new tests (the 8.1.3 baseline itself is unchanged, it is history). The suite is **110 passed,
0 failed, 1 ignored** at the merge state, and the AC table in
[reports/ffmpeg-9.1-differential.md](reports/ffmpeg-9.1-differential.md) §8 carries the updated AC-6
row. No FFmpeg package change was needed for the follow-up: all three fixes are in the shim and its
tests.

**Merge record:** `feat/ffmpeg-9.1-upgrade` was merged into `main` with `--no-ff` (`e5d444f`), the
full suite was re-run on `main`, and the branch was pushed. The P0 branch `fix/p0-audit-88e0707` and
its worktree were left untouched.