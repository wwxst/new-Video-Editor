# HANDOVER — foundation checkpoint (Phases A, P, B complete; F started)

Checkpoint date: 2026-11-25. Supersedes the earlier WIP handover of `720a99a`.

**State: the workspace builds, clippy is clean, and all 65 tests pass** (3 consecutive full runs on Windows 11,
Intel integrated GPU). Every milestone below is committed and pushed to `main`. Architecture docs under
`docs/` remain the source of truth; this file records implementation status, deviations, and how to continue.

---

## 1. Getting a working environment (Windows)

Everything is user-local; no admin rights or system PATH changes were made.

| Component | Location | Why |
|---|---|---|
| Rust 1.100.0, `x86_64-pc-windows-gnu` | `~/.rustup/toolchains/stable-x86_64-pc-windows-gnu/bin` | Only toolchain on this machine. `~/.cargo/bin` proxies do not exist. |
| MinGW-w64 GCC 16.2.0 UCRT (WinLibs r3) | `~/.local/toolchains/mingw64` | `windows-sys` 0.61 links via raw-dylib and **needs `dlltool.exe`**; also the C compiler for the libav shim. Download SHA-256 `3c69305081e4a647144339a936244fcf7d3d9e558319e1ad7be2cd2886d4b6a1` (matched the published checksum). |
| FFmpeg **8.1.3 LGPL shared** (BtbN autobuild `ffmpeg-n8.1.3-41-ga8029b6528-win64-lgpl-shared-8.1.zip`) | `~/.local/toolchains/ffmpeg-8.1-lgpl` | Headers + GNU import libs for the shim; LGPL `ffmpeg`/`ffprobe` for subprocess jobs. Built with the same GCC 16.2; `--disable-libx264 --disable-libx265`. Size matched the release API (81,544,256 bytes), zip CRCs OK, SHA-256 `dbed3e64d68de76472bf2cd0b0f3250384fb96bac14b635a708e998f6e1d6996` (GitHub reset the connection for the separate checksum file). |

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
scripts/dev-env.sh
```

### Phase status (docs/10)

| Phase | Status | Evidence |
|---|---|---|
| **A Spine** | **Done** | Exit criteria met: 10,000-case property test, timecode boundary tests, `new` + `apply` produce valid containers |
| **P Embedding PoC** | **Done — GO** | All automated criteria pass (docs/poc, ADR-0014 outcome) |
| **B Headless pipeline** | **Done** | Frame-exact `import → edit → render` through FFV1 and H.264 (hardware `h264_qsv` here) |
| **C Compositor + shell** | Compositor done; **GUI editor not started** | `arc-app` currently shows the PoC workload, not an editor |
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
arcline-cli render <p> -o out --codec ffv1|h264|av1|prores [--from TC --to TC] [--snapshot doc.json]
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

## 3. Test results (all green, 65 tests)

| Crate / suite | Tests | What is proven |
|---|---|---|
| arc-core unit (time, ids, model) | 15 | Exact time; all 8 rates; NDF/DF vectors and rejections; **exhaustive 24 h DF round trip**; sample mapping; canonical JSON |
| arc-core `tests/engine.rs` | 11 | Commands, overlap/off-boundary/unknown errors with paths, atomic batches, stale revision, dry run, idempotency, `internal.restore` ban, trim/split/ripple/transitions, manifest validation, undo/redo hashes, control + auto-takeover, rollback of unconfirmed batches, journal replay |
| arc-core `tests/property.rs` | 1 (10,000 cases) | **Invariant 7**: apply → undo → identical canonical hash; redo → identical; every committed state validates |
| arc-core `tests/journal.rs` | 9 | Torn tail at every byte offset, CRC/magic/length corruption, short writes, mid-record failure truncation, failed truncate, durable `fsync` failure, group-flush failure, CRC32C vectors |
| arc-core `tests/store.rs` | 7 (+1 helper) | Container round trip, future schema refused, save/reopen/clean close, lock, torn-tail recovery with `.rejected`, ack + notifications, **forced kill via child-process `abort()` at 1, 29, 30, 31, 75 edits** |
| arc-media unit + `tests/decode.rs` | 3 + 3 | ffprobe mapping; **frame-accurate in-process decode** (sequential, random, repeated, backward) at 29.97 and 25 fps; sample-addressed audio; import check ok/offline |
| arc-render unit + `tests/plan.rs` + `tests/render.rs` | 1 + 1 + 5 | Manifests; frame planning incl. dissolve weights and handles; GPU bit-exact determinism, 1-LSB pass-through, color correct within 2 LSB of the CPU reference, transforms, linear-light opacity/dissolve, every effect runs or is reported |
| arc-audio unit | 3 | **Block-size parity** (1…4096 identical), positions/gain/pan/fades/varispeed, WAV header |
| arc-cli `tests/cli.rs`, `render_e2e.rs`, `rpc.rs` | 2 + 2 + 2 | CLI contract; **end-to-end: every output frame verified exact** (FFV1, H.264, ranges) with audio; RPC semantics and an out-of-process render job |

Not part of `cargo test`: the embedding PoC (`--example embed_host`). Run it manually; result recorded in
`docs/poc/embedding-poc-2026-11-25.md`.

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
| CLI undo | Removed: history is session-scoped; use `arcline-cli rpc` |
| Decode | In-process libav through `csrc/arc_av.c` (opaque handle; no FFmpeg structs in Rust). YUV→RGB happens in swscale on the CPU (BT.709/601 by stream tags) — **docs/03 §10 GPU YUV upload is a later optimization** |
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

## 6. Known issues and unverified areas

1. **rustc 1.100 incremental ICE under clippy** for `arc-core` (`evaluate_obligation`). Workaround:
   `CARGO_INCREMENTAL=0 cargo clippy …`, or `cargo clean -p arc-core`. Builds and tests are unaffected.
2. **MSVC target untested.** The shim should compile with `cl /std:c11` (compound literals are used); `build.rs`
   passes `-std=c11` only where supported — add `/std:c11` for MSVC if needed.
3. **No CI workflow exists yet** (`.github/workflows`). Planned per ADR-0011: Windows build + test with an LGPL
   FFmpeg download and `ARCLINE_FORCE_FALLBACK_ADAPTER=1` (WARP); macOS/Linux `cargo check` of the FFmpeg-free
   crates (`arc-core`, `arc-render`, `arc-audio`) plus `arc-core` tests on Linux.
4. **GPU tests need an adapter.** They ran on Intel integrated graphics here; on CI use the fallback adapter.
5. **Embedding PoC manual items open:** keyboard focus round-trip, IME placement, DPI change across monitors,
   Electron host (Electron not installed).
6. **macOS embedding:** winit 0.30 `with_parent_window` supports Windows and X11 only (risk R21).
7. `cargo` warns that workspace dependency `arc-cli` is unused (nothing depends on it yet; `arc-app` will).
8. The export copies every decoded frame (`to_vec`) per item per frame; fine for the foundation, a known
   optimization target.
9. Performance budgets (docs/08 §2) are **not yet measured**, except the PoC frame pacing.

## 7. Next steps (in roadmap order)

**Phase C — the editor (largest remaining piece)**
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
- No GPL encoders in shipped paths; keep `ARCLINE_FFMPEG_DIR` pointed at an LGPL build.
- Tests use generated fixtures only (`arc_media::fixtures`), never third-party media.
