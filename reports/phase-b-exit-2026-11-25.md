# Phase B exit criteria — closure batch 2 (2026-11-25)

**Branch:** `feat/phase-b-exit` (from `main` at `657f34a`). **Scope:** the three Phase B exit
criteria that the audit follow-up still listed as open — HEVC export, a malformed-media corpus with
error convergence, and CI. Documentation: `docs/10-roadmap.md` §4 and `HANDOVER.md`.

**State:** full workspace `cargo test` green (see §5 for the exact count), `cargo build
--workspace --all-targets` exit 0, the 8.1.3-vs-9.1 differential still 0 unexplained, and the CI
workflow is live on GitHub: the **core matrix is green** on Windows, Linux and macOS (run 2), while
the Windows `media` job is still building the pinned FFmpeg package (link and status in §3.2).

## 1. Goal A — HEVC export

### 1.1 What the pinned package contains

`ffmpeg -encoders` on the pinned package lists only hardware HEVC encoders:

| Encoder | Type | Available here |
|---|---|---|
| `hevc_nvenc` | NVIDIA NVENC | no NVIDIA GPU |
| `hevc_qsv` | Intel Quick Sync | **yes** (reference machine, Intel iGPU) |
| `hevc_amf` | AMD AMF | no AMD GPU |
| `hevc_d3d12va` | D3D12 Video Acceleration | not a candidate (VA-API-style wrapper, no encoder in this build) |
| `libx265` | software | **absent**: x265 is GPL and the package is built `--disable-gpl` (decision D1) |
| `hevc_mf` | Media Foundation | absent by decision D2 |

`ffmpeg -buildconf` shows `--enable-nvenc --enable-amf --enable-libvpl` (hardware wrappers only),
no `--enable-gpl`, no `--enable-libx265`. So software HEVC is **not available by design**, not by
accident: adding it would make the shipped package GPL and contradicts the documented licence
posture (docs/04 §9, docs/11 §2.1, spec D1).

### 1.2 What was implemented

`VideoCodec::Hevc` (`--codec hevc`) selects the first usable candidate from
`["hevc_nvenc", "hevc_qsv", "hevc_amf"]` through the existing capability probe (encoder listed by
`ffmpeg -encoders` **and** a one-frame trial encode). There is deliberately no software fallback: a
missing encoder returns the typed `EncodeError::NoEncoder(Hevc, candidates)` whose message names
every candidate, so a user sees exactly what was tried instead of a generic failure. Output is
`yuv420p`, BT.709-tagged through the existing export pipeline; audio follows the container rules
already in place (AAC for `.mp4`, PCM for `.mov`, FLAC for `.mkv`).

### 1.3 Evidence

`crates/arc-cli/tests/render_e2e.rs::hevc_exports_are_decodable_and_match_the_spec` renders a 1 s
range of the standard 320×180 / 29.97 fps project and validates it with the **pinned ffprobe**:

```
encoder probe: Hevc -> hevc_qsv
ffprobe -show_entries stream=codec_name,width,height,pix_fmt,r_frame_rate:
  codec_name=hevc  width=320  height=180  pix_fmt=yuv420p  r_frame_rate=30000/1001
```

The test then decodes the file back through `VideoDecoder` (`info().codec == "hevc"`, 320×180) and
asserts `frames == 10`, `codec == "hevc"`, `ffmpeg.override == false`. A unit test pins the
hardware-only candidate list, the pixel format and the error text
(`crates/arc-media/src/encode.rs::hevc_candidates_are_hardware_only_and_the_error_names_them`).

**Result:** HEVC export works on the reference machine through `hevc_qsv`; on a machine without any
hardware HEVC encoder the command fails with the typed error above. Software HEVC is not offered
and cannot be without a GPL build — if the owner ever wants it, that is a licensing decision for
the owner and counsel, not a code change.

## 2. Goal B — robustness corpus and error convergence

Full detail, fixture table and contract: [robustness-corpus-2026-11-25.md](robustness-corpus-2026-11-25.md).

Summary: `crates/arc-media/tests/malformed.rs` generates nine fixtures in the test (empty, text,
truncated 25 %/50 %, EBML magic zeroed, header overwritten, payload corrupted mid-file, MP4
`moov` overwritten, JPEG renamed `.mkv`), each below 2 MB. Every unreadable fixture must be refused
by the import check and must return a typed `MediaError` at the decode boundary; a panic or abort
fails the test. Two fixtures legitimately still decode (FFV1 concealment; a JPEG is a valid image)
and assert only the no-crash contract.

Two convergence fixes came out of it: the import check now rejects a test decode that produces **no
frames** (a truncated container with an intact header used to be accepted because ffmpeg exits 0
when a seek lands past the data), and `VideoDecoder::open` refuses frame buffers above 2^28 pixels
before allocating, so a corrupt header cannot abort the process.

`crates/arc-cli/tests/render_e2e.rs::mixed_media_export_survives_an_unreadable_asset` proves the
engine keeps working with bad media: two assets imported while valid, one overwritten after
import, both in one project; the export exits 0 with all 60 frames, names the unavailable asset in
its warnings, renders black for that clip, keeps the intact clip frame-exact, and the project still
describes afterwards. No `catch_unwind` is used (see the robustness report §2: media errors are
already `Result`s, and a native FFmpeg fault is not recoverable in-process — the export child is
the containment boundary).

**Remaining gap:** the `Quarantined` branch needs media that crashes or hangs `ffprobe`/`ffmpeg` in
the import check. That requires a decoder-crash PoC and was not attempted, so the "crashing media
is quarantined and the parent survives" half of the criterion is still unproven.

### 2.1 The `Quarantined` branch (batch 3)

The `Quarantined` outcome (import check crashed or timed out) now has a test:
`crates/arc-media/tests/quarantine.rs` compiles a stub `ffmpeg.exe`/`ffprobe.exe` with `rustc` (the
tool lookup is a runtime `ARCLINE_FFMPEG_DIR/bin` read, so no production hook is needed), points the
process at that bundle, and the stub proxies to the real pinned tools unless an argument contains
`STUB_MATCH`. One process then exercises:

| Case | Stub behaviour | Expected outcome | Bound |
|---|---|---|---|
| crash | exits with NTSTATUS `0xC0000005` | `Quarantined { reason: "...Crashed..." }` | immediate |
| hang | sleeps 120 s | `Quarantined { reason: "...TimedOut..." }` | killed at `PROBE_TIMEOUT` (20 s), measured under 35 s |
| garbage | prints non-JSON, exits 0 | `Offline` (not `Quarantined`: "not media" vs "never decode in-process") | immediate |
| mixed | real media between the bad cases | `Ok` before and after, same process | — |

No panic and no abort: the test binary survives all four, and the timeout is asserted to be the
documented `PROBE_TIMEOUT` cap.

## 3. Goal C — CI

### 3.1 Workflow

`.github/workflows/ci.yml`, triggered on push to `main`, pull requests and manual dispatch:

| Job | Runner | Steps |
|---|---|---|
| `core` | `ubuntu-latest`, `windows-latest`, `macos-latest` | Rust 1.95.0 + clippy (declared MSRV), cargo cache, build `arc-core`/`arc-audio`, `cargo check -p arc-render`, `cargo test -p arc-core`, clippy on the three FFmpeg-free crates |
| `media` | `windows-latest` | MSYS2 UCRT64 (gcc/nasm/pkgconf/gpg), cache of the pinned FFmpeg 9.1 package keyed by manifest/profile/build-script hash, FFmpeg 9.1 built from the in-repo recipe on a cache miss (source fetched from ffmpeg.org and signature-verified by the recipe), Rust 1.95.0 `x86_64-pc-windows-gnu` host, full `cargo build --workspace --all-targets` + `cargo test --workspace` + clippy, `ARCLINE_FORCE_FALLBACK_ADAPTER=1` for the GPU tests |

Caching: `Swatinem/rust-cache` for cargo (per job), `actions/cache` for the FFmpeg package. The
warning strategy is the repository's existing one: clippy must exit 0, warnings are shown but not
denied.

The full workspace needs the media package, so the whole-suite job is Windows-only; the Linux and
macOS jobs cover the FFmpeg-free crates, which is exactly what HANDOVER §6.3 planned. Hardware
encoder tests now skip themselves when the capability probe finds no encoder, because a GPU-less
runner has no `h264_*`/`hevc_*` encoder in the shipped build (no software H.264/HEVC by posture).

### 3.2 First real runs on GitHub

The repository is public, so the Actions API is readable without a token (job logs are not: they
need repository admin rights). Runs, `https://github.com/wwxst/new-Video-Editor/actions`:

| Run | Commit | Result |
|---|---|---|
| [#1](https://github.com/wwxst/new-Video-Editor/actions/runs/45643845772) | `a394f37` | `core (windows)` success; `core (ubuntu)`, `core (macos)` failed in `cargo test -p arc-core`; `media` failed in the FFmpeg build step |
| [#2](https://github.com/wwxst/new-Video-Editor/actions/runs/45645526473) | `5a1a3ac` | **`core` success on windows, ubuntu and macos**; `media`: MSYS2 setup, cargo cache, the **FFmpeg 9.1 build (step 6) and Rust 1.95 (step 7) succeeded**, then the combined workspace build/test/clippy step failed |
| [#3](https://github.com/wwxst/new-Video-Editor/actions/runs/45649281563) | `ef4528e` | **`core` success on all three OSes again**; `media`: FFmpeg build and Rust 1.95 succeeded, the workspace step failed (split steps: the Test/Clippy half) |
| [#5](https://github.com/wwxst/new-Video-Editor/actions/runs/45651118343) | `0eafb40` | core success 3/3; media failed at **Build the whole workspace**; the new annotation identified `arc-media`'s build script |
| [#6](https://github.com/wwxst/new-Video-Editor/actions/runs/45653879005) | `bb75198` | core success 3/3; the build-context annotation pinned the cause: the FFmpeg package was installed under the MSYS2 home, so `include/libavutil` was missing at the expected path |
| [#7](https://github.com/wwxst/new-Video-Editor/actions/runs/45656189098) | `4b4be80` | **all four jobs success** — the HOME fix restored the FFmpeg package to the cached path, the workspace build/test/clippy passed under Rust 1.95 + WARP |
| [#9](https://github.com/wwxst/new-Video-Editor/actions/runs/45661978288) | `1ea91e2` | **all four jobs success** again (the Phase C merge), which confirms the CI state on current `main` |

The media job's FFmpeg build succeeding in CI is the important half: the in-repo recipe (source
fetch + signature/SHA verification + configure profile) works on a clean `windows-latest` runner.

**Two root causes (batch 3).**

1. **The CI-built FFmpeg package was installed into the MSYS2 home.** Run 6's build-context
   annotation (added in `bb75198`) pinned it: `arc-media`'s build script panicked with
   `cannot read LIBAVUTIL_VERSION_* from C:\Users\runneradmin\.local\toolchains\ffmpeg-9.1-lgpl\include\libavutil`.
   In the MSYS2 shell `$HOME` is `/home/<user>` (inside the MSYS2 root), so `build.sh` installed the
   package under the MSYS2 home while the workflow cached and exported `C:\Users\<user>\.local` (the
   run-5 "Cache save failed." annotation is the same cause). Both MSYS2 steps now
   `export HOME="$(cygpath "$USERPROFILE")"` (`4b4be80`).
2. **The WARP access violation in the GPU tests.** With
`ARCLINE_FORCE_FALLBACK_ADAPTER=1` — exactly what the media job sets — wgpu selects
`Microsoft Basic Render Driver (Dx12)` (WARP). `arc-render/tests/render.rs` has five GPU tests that
each create their own wgpu device; with cargo's default parallelism the software driver crashes the
test binary with **`STATUS_ACCESS_VIOLATION` (0xC0000005)**, so the whole workspace step fails. It is
reproducible on the reference machine by exporting the same variable, and it is specific to the
software adapter: the same suite is green on the real GPU and green under WARP with
`--test-threads=1`.

**Fix:** the five GPU tests are serialized with a process-wide mutex in their test binary
(`GPU_SERIAL`), documented as a WARP workaround. Verified on the reference machine with
`ARCLINE_FORCE_FALLBACK_ADAPTER=1`: `arc-render` tests pass three runs in a row and the **whole
workspace suite is 116 passed / 0 failed / 1 ignored** under the software adapter.

The workflow also now annotates failures (`::error` check-run annotations with the failing lines
`FAILED|panicked at|STATUS_|error[` plus a step-summary tail), because job logs need repository admin
rights but annotations are readable through the public API.

Run 1 diagnosed two real problems, both fixed in `5a1a3ac`:

- the media job's MSYS2 package list was missing the external libraries the pinned profile enables
  (`dav1d`, `svt-av1`, `libvpl`, `ffnvcodec-headers`, `amf-headers`, `zlib`), so `configure` could
  not run;
- two `arc-core` lock tests assumed the Windows strong `SessionLock`. `second_open_is_locked` is now
  `#[cfg(windows)]`, and `create_over_locked_project_is_refused_without_touching_it` accepts
  `Locked` on Windows or `Exists` elsewhere while keeping its container-integrity assertions on all
  platforms.

Local verification of the same core-job steps (rustc 1.100.0, Windows GNU):

| Step | Result |
|---|---|
| `cargo build -p arc-core -p arc-audio` | exit 0 |
| `cargo check -p arc-render` | exit 0 |
| `cargo test -p arc-core` | 55 passed, 0 failed, 1 ignored |
| `cargo clippy -p arc-core -p arc-audio -p arc-render --all-targets` | exit 0 (pre-existing lint warnings only) |
| YAML | `.github/workflows/ci.yml` parses; 2 jobs, 3-OS matrix, 8 media steps |

The media job's only CI-specific pieces are the FFmpeg build (the recipe that produced the local 9.1
package, cached afterwards) and the WARP fallback; the suite it runs is the same full suite recorded
in §5. Its first cold build is the long pole; the result is recorded in the run link above.

### 3.3 How to see the real run

1. Install the CLI: `winget install GitHub.cli` (or `scoop install gh`), then `gh auth login`.
2. `gh run list --workflow=ci.yml` and `gh run watch <id>` — or open
   `https://github.com/wwxst/new-Video-Editor/actions`.
3. The first `media` run is expected to take ~30–60 min because it builds FFmpeg 9.1; later runs
   restore the cached package and finish in minutes.

## 4. Blockers and recommendations

- **Software HEVC is a licensing decision, not a gap.** If HEVC must work on machines without
  Intel/NVIDIA/AMD hardware encoders, the owner has to approve a GPL FFmpeg build (x265) and its
  distribution obligations; today the product is hardware-only by design.
- **Quarantine fixture.** The remaining Phase B gap is a crashing/hanging media sample. A fuzzed
  corpus (cargo-fuzz against the shim, `-fsanitize=address` where available) is the right way to
  produce one, and it also serves the "mutated-media corpus" wording of the criterion.
- **The `media` job had two independent failures, both fixed and now verified green in CI.** (1) The
  CI-built FFmpeg package was installed under the MSYS2 `$HOME` while the workflow
  cached/exported `C:\Users\<user>\.local`, so `arc-media`'s build script found no headers; the MSYS2
  steps now export `HOME="$(cygpath "$USERPROFILE")"`. (2) Five `arc-render` GPU tests created wgpu
  devices concurrently, which crashes under `Microsoft Basic Render Driver (Dx12)` with
  `0xC0000005`; the tests are serialized (three green runs under the fallback adapter, whole suite
  116/116 there). Both runs #7 (`4b4be80`) and #9 (`1ea91e2`, current `main`) finished with **all four
  jobs green**, so the media job is no longer a constraint. The workflow emits check-run annotations
  on failure, readable through the public API without a token.
- **Rust 1.95 locally is not installable in this environment.** `static.rust-lang.org` was throttled
  to ~7 KB/s (4 MB downloaded in ~10 minutes of a 297 MB bundle), so the 1.95 compile of the media
  crates could not be reproduced locally. The `core` matrix compiles and tests arc-core, arc-audio
  and arc-render with 1.95 in CI; if a media crate needed a newer compiler, the CI **Build** step
  (now a separate step with annotations) will name it. Fallback: pin the CI toolchain to 1.100 while
  keeping `rust-version = "1.95"` as the declared MSRV.

## 5. Verification at this branch

```text
cargo build --workspace --all-targets                 exit 0
cargo test --workspace                                115 passed, 0 failed, 1 ignored
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets   exit 0 (pre-existing warnings only)
ARCLINE_CORPUS_DIR=... cargo test -p arc-media --test differential   1 passed, 0 unexplained
```

The five new tests over the previous batch's 110 are `arc-media/tests/malformed.rs` (1 test,
9 fixtures), `render_e2e::hevc_exports_are_decodable_and_match_the_spec`,
`render_e2e::mixed_media_export_survives_an_unreadable_asset`, the HEVC candidate/error unit test
(`arc-media/src/encode.rs`) and the dimension-guard unit test (`arc-media/src/libav.rs`).

## 6. Commits

| Commit | Content |
|---|---|
| `05182d1` | HEVC export (`feat(media,cli)`), probe + typed error, ffprobe e2e |
| `5fcc28b` | malformed corpus, truncation + dimension convergence, mixed-media export |
| `ea4b0ce` | CI workflow and hardware-test skip policy |
| `a394f37` | MSYS2 archive/text tools for the FFmpeg recipe |
| `5a1a3ac` | FFmpeg build dependencies + the two platform lock-test fixes (run 1 → run 2) |
