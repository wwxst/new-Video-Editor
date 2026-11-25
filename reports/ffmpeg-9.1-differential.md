# FFmpeg 8.1.3 → 9.1 upgrade — verification report

**Date:** 2026-11-25. **Branches:** `fix/p0-audit-88e0707` (P0 + architect follow-ups) and
`feat/ffmpeg-9.1-upgrade` (spec steps 1–3 + this report). **Spec:**
[docs/specs/ffmpeg-9.1-upgrade.md](../docs/specs/ffmpeg-9.1-upgrade.md). **Package:**
[third_party/ffmpeg](../third_party/ffmpeg).

This is the acceptance evidence required by spec §9: AC-1…AC-15 with the command and result for
each, the 8.1.3-vs-9.1 differential with every difference explained, the performance measurement
(AC-12) and the rollback rehearsal (AC-15). The machine is the reference Windows 11 machine
(Intel integrated GPU, Rust 1.100.0 GNU, WinLibs GCC 16.2.0 UCRT for the workspace; MSYS2 UCRT64
GCC 16.2.0 rev7 for the FFmpeg build).

---

## 1. Package and source identity (AC-13)

| Item | Value |
|---|---|
| Release | FFmpeg 9.1 "Gauss", tag `n9.1`, commit `74e4393595cf2b5857f0da925c4e0dd74ea306e5` |
| Source | official `ffmpeg-9.1.tar.xz` from <https://ffmpeg.org/releases/> |
| SHA-256 | `1a4d394ae63d190a526cd052eb4415c156b5c5e2df37efab41a7de0b8fde2433` |
| Signature | **verified**, `GOODSIG`/`VALIDSIG` by key `FCF9 86EA 15E6 E293 A564 4F10 B432 2F04 D676 58D8` ("FFmpeg release signing key <ffmpeg-devel@ffmpeg.org>"), signature date 2026-11-20. Public key committed as [ffmpeg-release-signing-key.asc](../third_party/ffmpeg/ffmpeg-release-signing-key.asc); `build.sh` re-imports it into a temporary keyring and verifies every build. |
| Build host | MSYS2 UCRT64, user-local; package lock [msys2-packages.txt](../third_party/ffmpeg/msys2-packages.txt) (113 packages). GCC 16.2.0-7, binutils 2.47-5, NASM 3.02, pkgconf 3.0.11, dav1d 1.5.5, SVT-AV1 4.3.0, libvpl 2.17.0, ffnvcodec-headers 13.0.19.0, amf-headers 1.5.3 |
| Ident | `9.1-arcline-f414c1c54e` (`build_id` `arcline-f414c1c54e`) |
| Libraries | avutil 61.19.100, avcodec 63.24.100, avformat 63.16.100, avdevice 63.5.100, avfilter 12.8.100, swscale 10.5.100, swresample 7.6.100 |
| Manifest | [manifest.json](../third_party/ffmpeg/manifest.json): source/signature pins, configure line, profile hash, host toolchain, **SHA-256 of all 225 packaged files**, decisions D1–D4, licence set |
| SBOM | [sbom.cyclonedx.json](../third_party/ffmpeg/sbom.cyclonedx.json) (CycloneDX 1.5; FFmpeg + dav1d, SVT-AV1, oneVPL, zlib, nv-codec-headers, AMF headers, GCC runtime, libstdc++, winpthreads) |
| Notices | [licenses/](../third_party/ffmpeg/licenses/) — LGPL v2.1 text, FFmpeg `LICENSE.md`, every third-party notice, GCC Runtime Library Exception, MinGW-w64 notices, IJG credit note |

`third_party/ffmpeg/verify.sh` resolves `objdump` from `OBJDUMP`, then the PATH (any binutils
distribution, including Git's/WinLibs'), then the pinned MSYS2 location, and fails clearly when
none is found — so the AC-4 PE checks also run outside MSYS2. Both paths were exercised: MSYS2
exit 0 with `objdump: objdump`, and a dev-shell PATH (WinLibs objdump) exit 0 with 40 `ok` checks;
a missing/bogus `OBJDUMP` exits 1 with `FAIL objdump not found; set OBJDUMP=<path> …`.

The export report's `ffmpeg.build_id` is the SHA-256 of `manifest.json`
(`6dfd2a28d285d600d6a99acaf5b9efcdfdf9d9622ff9f4f749ff05422294fa65` at the time of the build;
verified equal by an actual export). The manifest and SBOM are written as UTF-8 without BOM and
LF line endings so the hash is stable across checkouts.

**Deviations from the spec's recipe (recorded in the manifest and HANDOVER §9):**

1. The build is native MSYS2 UCRT64, so `--cross-prefix=x86_64-w64-mingw32-` is omitted (UCRT64
   installs prefixed GCC but only unprefixed binutils). Output is still x86_64-w64-mingw32/UCRT.
2. The profile links dav1d, SVT-AV1, oneVPL, zlib and the MinGW runtime dynamically; their runtime
   DLLs are bundled in `bin/` (BtbN statically links externals). All are in the manifest/SBOM and
   notices.
3. `webm_dash_manifest` (a local-manifest demuxer, not the network `dash` demuxer) remains; the
   literal AC-4 check for `hls`, `dash` and `concat` passes, and network protocols are disabled.
4. MP3 and hardware H.264 fixtures are committed (spec §8.2 allows those two); the rest of the
   differential corpus is generated and kept outside the repository because committing it is not
   allowed. The baseline records every file's BLAKE3 so the step-3 run cannot silently use a
   different corpus.

---

## 2. Differential results (AC-7, AC-9, AC-10)

**Harness:** [crates/arc-media/tests/differential.rs](../crates/arc-media/tests/differential.rs),
corpus generator and comparison in
[crates/arc-media/tests/support/mod.rs](../crates/arc-media/tests/support/mod.rs).
**Baseline:** [crates/arc-media/tests/baseline/ffmpeg-8.1.3.json](../crates/arc-media/tests/baseline/ffmpeg-8.1.3.json),
recorded on 8.1.3 against the corpus at `~/.local/toolchains/arcline-corpus-8.1.3` (22 entries,
generated media only). **Allow-list:** [expected-differences.json](../crates/arc-media/tests/baseline/expected-differences.json).
**Machine-readable result:** [ffmpeg-9.1-differential.json](ffmpeg-9.1-differential.json).

```bash
ARCLINE_CORPUS_DIR='C:\Users\ww\.local\toolchains\arcline-corpus-8.1.3' \
ARCLINE_DIFF_ALLOW='D:\Develop\Projects\Video Editing\crates\arc-media\tests\baseline\expected-differences.json' \
ARCLINE_DIFF_REPORT='reports\ffmpeg-9.1-differential.json' \
cargo test -p arc-media --test differential
# test result: ok. 1 passed   (22 entries; 4 differences, all allowed; unexplained = 0)
```

Per entry the harness compares: BLAKE3 of every decoded RGBA frame sequential **and** random
access, BLAKE3 of the decoded audio (one long read and the mixer's 4096+1 block pattern), the
parsed `Probe`, the import-check status/kind, and the media file's own BLAKE3.

| Observation across all 22 entries | Result |
|---|---|
| Corpus media unchanged (file BLAKE3) | identical → the comparison is on the same inputs |
| Video frame hashes, sequential and random access (FFV1 29.97/25/30, MPEG-2 MKV/TS, MPEG-4 Part 2 MP4, H.264 MP4, PNG/JPEG stills, F-02 resolution change) | **identical** (0 differences) — swscale 9→10 output is bit-identical through the legacy API |
| Audio long read (FLAC 48 k/44.1 k, WAV, MP3, Opus Ogg/MKV, AAC sine/noise, F-03 channel change) | **identical** (0 differences) |
| Audio block pattern (4096+1 reads) for FLAC/WAV/MP3/Opus/MP2 | **identical** (0 differences) |
| Audio block pattern for AAC sine and AAC noise | **differs** — 2 entries / 4 flattened paths, explained below |
| `Probe` (all fields the model uses) | **identical** (AC-9) |
| Import-check outcome (status and kind) | **identical** (AC-10) |

### The 2 AAC differences, explained (AC-7)

Only `audio_aac_sine.m4a` and `audio_aac_noise.m4a`, field `audio.blocks` (the hash and its
per-block list), differ. Nothing else in either entry differs (the long continuous read is
bit-identical, the `Probe` is identical, the import outcome is identical).

**Mechanism:** the shim still re-seeks for each mixer block (`F-06`, deliberately P2; the 4096+1
pattern consumes one sample more than the block advance and the shim treats the next read as a
backward seek). FFmpeg 9.1 changed AAC decoder priming/seek behaviour (`APIchanges` lavc
63.23.101 removes `AVCodecContext.delay` assignment from `avcodec_parameters_to_context`, and
63.23.102 lets the AAC decoder raise `seek_preroll`), and AAC output depends on decode history
(F-25). The same bytes decoded continuously are identical; only the post-seek block windows differ.
An independent check with each version's `ffmpeg` CLI — the full first second and 24 × (seek +
4097 samples) covering the whole file — produced **0 differing samples** between 8.1.3 and 9.1,
which confirms that this is the shim's per-block re-seek interaction (F-06/F-25) and not a decoder
regression. Tracked as F-06 (P2); the tolerance is documented here and in the allow-list.

---

## 3. Build and package checks (AC-4, AC-13)

`third_party/ffmpeg/verify.sh` (also run by the build maintainer) exits 0:

```
ok   ident is the pinned 9.1 build
ok   licence text is LGPL
ok   no nonfree
ok   buildconf: no GPL, nonfree or version3
ok   buildconf: network/protocols disabled
protocols: file pipe
ok   only file and pipe
ok   demuxer hls absent / dash absent / concat absent / sdp absent / rtsp absent / rtp absent
ok   no libopenh264 / h264_mf / hevc_mf
ok   bundled libdav1d-7.dll, libSvtAv1Enc-4.dll, libvpl-2.dll, libz-1.dll,
     libgcc_s_seh-1.dll, libwinpthread-1.dll, libstdc++-6.dll
ok   all DLLs/EXEs: DllCharacteristics=0x00000160
     (DYNAMIC_BASE 0x40 | HIGH_ENTROPY_VA 0x20 | NX_COMPAT 0x100)
VERIFY_EXIT=0
```

`ffmpeg -buildconf` contains `--disable-gpl --disable-nonfree`, no `--enable-version3`, and the
profile's `--disable-network --disable-protocols --enable-protocol=file,pipe`; `ffmpeg -L` prints the
GNU Lesser General Public Licence version 2.1-or-later; `-encoders` contains `ffv1`, `libsvtav1`,
`prores_ks`, `h264_qsv`, `h264_nvenc`, `h264_amf`, `aac`, `flac`, and not `libopenh264`, `h264_mf`
or `hevc_mf` (D2). All shipped binaries carry the three required PE flags.

---

## 4. Code, runtime and tool checks (AC-1, AC-2, AC-3, AC-5)

- **AC-1:** `cargo build --workspace --all-targets` → exit 0. The shim compiles cleanly with the
  spec's warning flags against the 9.1 headers:
  `gcc -std=c11 -Wall -Wextra -Werror=deprecated-declarations -fsyntax-only -I$ARCLINE_FFMPEG_DIR/include crates/arc-media/csrc/arc_av.c`
  → exit 0, no diagnostics. `build.rs` also passes `-Werror=deprecated-declarations` normally.
- **AC-2:** `ARCLINE_FFMPEG_DIR=~/.local/toolchains/ffmpeg-8.1-lgpl cargo build -p arc-media` fails
  with `arc-media: avutil 60.26.103 is not FFmpeg 9.x (required major 61). Set ARCLINE_FFMPEG_DIR to
  the FFmpeg 9.1 package …` (exit 101). The raw shim against 8.1 headers emits both §4.3 `#error`
  lines. Building against 9.1 passes (AC-1).
- **AC-3:** `objdump -p target/debug/arcline-cli.exe` lists `avcodec-63.dll`, `avformat-63.dll`,
  `avutil-61.dll`, `swscale-10.dll`, `swresample-7.dll` and **no** 8.x names. A fake bundle whose
  `ffmpeg`/`ffprobe` are the 8.1.3 binaries is refused:
  `probe` exits 2 with `{"kind":"ffmpeg_unavailable","message":"ffmpeg is n8.1.3-41-…, but this
  build is pinned to 9.1-arcline-f414c1c54e; …"}`. An actual export report contains the §4.5 block
  and its `build_id` equals the manifest SHA-256:
  ```json
  "ffmpeg": {"ident":"9.1-arcline-f414c1c54e",
             "libraries":{"avcodec":"63.24.100","avformat":"63.16.100","avutil":"61.19.100",
                          "swresample":"7.6.100","swscale":"10.5.100"},
             "license":"LGPL version 2.1 or later",
             "build_id":"6dfd2a28d285d600d6a99acaf5b9efcdfdf9d9622ff9f4f749ff05422294fa65",
             "override":false}
  ```
- **AC-5:** `cargo test -p arc-media --test paths` passes: `VideoDecoder`/`AudioDecoder` reject
  `http://`, `https://`, `handle:`, `concat:`, `udp://` and `file:`-prefixed strings with
  `MediaError::Path` before any I/O; spaces and non-ASCII filenames open correctly. CLI/RPC tests
  reject URL, `handle:`, `concat:` and `\\.\` / `\\?\` / `//./` / `//?/` asset paths with
  `invalid_value` before the import check, and `arcline-cli probe`/`import` reject scheme/device
  paths. The shim itself refuses any open string that does not start with `file:` and passes
  `protocol_whitelist=file`.

---

## 5. Test evidence (AC-6, AC-8, AC-11)

**AC-6 — three consecutive full runs and clippy:**

| Run | Command | Result |
|---|---|---|
| 1 | `cargo test --workspace` | **96 passed, 0 failed, 1 ignored** |
| 2 | `cargo test --workspace` | **96 passed, 0 failed, 1 ignored** |
| 3 | `cargo test --workspace` | **96 passed, 0 failed, 1 ignored** |
| clippy | `cargo clean -p … && CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets` | exit 0, **0 lints** (only the pre-existing unused-workspace-dependency manifest warning) |
| build | `cargo build --workspace --all-targets` | exit 0 |

All 65 original tests are still present and pass unchanged; the additional 31 tests are the P0
regressions and the upgrade's runtime/encoder/fixture tests.

- **AC-8:** on this machine `--codec h264` selects **`h264_qsv`** (10/10 frames, verified by an
  actual render); `libsvtav1` encodes and the output decodes (`libsvtav1_encodes_and_the_result_decodes`);
  `render_is_frame_exact_with_audio` verifies every FFV1 output frame and `render_range_and_h264`
  verifies the H.264 output frame; `av1_and_prores_exports_are_decodable` verifies AV1 and
  `prores_ks` outputs decode; `dnxhd_exports_are_decodable` renders a 1920×1080 DNxHD (SMPTE VC-3)
  `.mov` and decodes it back, as spec §8.4 requires. Encoder selection is unchanged from 8.1.3
  (`h264_qsv` first available).
- **AC-11:** `cargo test -p arc-media --test param_change` — 2 passed. The F-02 test checks both
  the byte-concatenated TS fixture (sequential, 120 frames) and the audited continuous-timestamp
  MKV fixture (sequential, ≥110 of 120 frames, plus fresh-decoder random access). The MKV fixture is
  built by muxing segment B with `-output_ts_offset 2.1`, byte-concatenating the two MPEG-TS files
  and remuxing with `-c copy`; the `mpegts` demuxer reports **one corrupt packet at the byte join**
  (the PES/continuity boundary) and drops at most one frame, so the test requires ≥110 frames
  instead of exactly 120. The remux is otherwise clean: the MKV carries the mid-stream resolution
  change, sequential decoding scales every frame into the reported 320×180 buffer, and random
  access works. Every decoded frame is scaled to the reported 320×180 with a 320×180×4 RGBA
  buffer. The F-03 test reads MP2 stereo→mono both as one long read and in the mixer's 4096+1
  pattern; no crash and both segments carry audio.

---

## 6. Performance (AC-12)

Method: release build of `arcline-cli` in both trees (`fix/p0-audit-88e0707` = 8.1.3,
`4ad2a54` = 9.1), same machine, same media, three runs each, median reported. Decode throughput is
a 60 s 1920×1080 FFV1 MKV decoded with each version's own `ffmpeg -benchmark … -f null -`
(rtime). The export is a 60-second FFV1 render of a 640×360 project (the 1080p export is dominated
by the pre-existing per-frame readback/compositor debt, audit F-33, and was not useful as a decoder
comparison).

| Measurement | 8.1.3 | 9.1 | Δ | Limit |
|---|---|---|---|---|
| 1080p decode, rtime per run | 0.813 / 0.816 / 0.793 s | 0.819 / 0.764 / 0.798 s | — | — |
| 1080p decode, median | **0.813 s** | **0.798 s** | **−1.8 %** | ±10 % ✅ |
| 60 s FFV1 export, runs | 1.783 / 1.665 / 1.673 s | 1.807 / 1.857 / 1.739 s | — | — |
| 60 s FFV1 export, median | **1.673 s** | **1.807 s** | **+8.0 %** | ±10 % ✅ |

Both measurements are inside the ±10 % budget. (Export times are short at 640×360; the absolute
numbers are dominated by process start, import and muxing overhead, which is why the three runs
are reported.)

---

## 7. Rollback rehearsal (AC-15)

Performed in a throw-away worktree, not by rewriting the branch:

```bash
git worktree add %TEMP%\arcline-rollback 4ad2a54
cd %TEMP%\arcline-rollback
# Revert the §4.3 pin / runtime check / build.rs / tools changes to the P0 state, keep step-1 corpus:
git checkout fix/p0-audit-88e0707 -- crates/arc-media crates/arc-cli scripts/dev-env.sh
rm crates/arc-media/tests/runtime.rs        # test added with the switch
unset ARCLINE_FFMPEG_DIR && . scripts/dev-env.sh   # P0 dev-env selects 8.1.3
ffmpeg -version | head -1                   # ffmpeg version n8.1.3-41-ga8029b6528-20261124
cargo test --workspace
```

Result: **87 passed, 0 failed, 1 ignored** against the still-installed 8.1.3 package. Project files
are unaffected (schema 7, no format change), and `scripts/dev-env.sh` keeps 8.1.3 as the rollback
default when the 9.1 directory is absent.

---

## 8. AC-1…AC-15 table

| AC | Requirement | Command / evidence | Result |
|---|---|---|---|
| AC-1 | 9.1 build; shim with the warning flags, no warnings | `cargo build --workspace --all-targets`; `gcc … -Werror=deprecated-declarations -fsyntax-only csrc/arc_av.c` | ✅ exit 0 both; no shim diagnostics |
| AC-2 | 8.x build fails clearly; 9.1 passes | `ARCLINE_FFMPEG_DIR=…8.1 cargo build -p arc-media`; raw shim vs 8.1 headers | ✅ exit 101 with "not FFmpeg 9.x (required major 61)"; both `#error`s; 9.1 passes |
| AC-3 | 9.x imports only; mismatched DLLs/tools refused; report block | `objdump -p arcline-cli.exe`; fake 8.1 tool bundle + `probe`; export report | ✅ avcodec-63/format-63/util-61/sws-10/swr-7, no 8.x; exit 2 `ffmpeg_unavailable` with pinned ident; report block with manifest SHA |
| AC-4 | Shipped build: LGPL, no GPL/nonfree/version3, file+pipe only, no hls/dash/concat, no libopenh264/h264_mf, PE flags | `third_party/ffmpeg/verify.sh` | ✅ exit 0, 40 checks ok (see §3) |
| AC-5 | `http:`, `handle:`, `concat:` etc. fail before network/handle in shim and CLI | `cargo test -p arc-media --test paths`; CLI/RPC import rejection tests | ✅ all refused (`MediaError::Path` / `invalid_value`) |
| AC-6 | All existing tests pass 3×; clippy clean | 3 × `cargo test --workspace`; clippy | ✅ 96/96/96 passed, 0 failed, 1 ignored; 0 lints |
| AC-7 | No unexplained 8.1.3-vs-9.1 difference | differential harness + allow-list | ✅ 4 differences (2 AAC block patterns ×2 paths), 0 unexplained; see §2 |
| AC-8 | Encoder selection unchanged; AV1/libsvtav1; FFV1+H.264 frame-exact; ProRes/DNxHD decodable | `render_e2e`; manual `--codec h264`; `libsvtav1` test | ✅ h264_qsv; libsvtav1 encodes/decodes; FFV1/H.264 frame-exact; AV1/ProRes/DNxHD decode |
| AC-9 | `Probe` results equal baseline | differential report | ✅ identical for all 22 entries |
| AC-10 | Import-check outcomes equal baseline | differential report | ✅ identical for all 22 entries |
| AC-11 | Corpus J/K neither crash nor wrong data | `cargo test -p arc-media --test param_change` | ✅ 2 passed, both fixture shapes + both audio read patterns |
| AC-12 | Performance within ±10 % | benchmark §6 | ✅ decode −1.8 %, export +8.0 % |
| AC-13 | `third_party/ffmpeg/` recipe, profile, manifest hashes, licences, SBOM; signature verified and recorded | directory + `build.sh` + `manifest.json` + gpg output | ✅ all present; GOODSIG/VALIDSIG recorded |
| AC-14 | Docs updated | HANDOVER §1/§3/§5/§6/§7/§9, docs/04 §9, docs/11 §2.1, ADR-0003 note, README, spec status | ✅ committed with this report |
| AC-15 | Rollback rehearsed | worktree procedure §7 | ✅ 87 passed, 0 failed on 8.1.3 |

No acceptance criterion failed. The only differences expected by the spec that were observed are
the two AAC block-pattern hashes (§2), and they are attributed to the open F-06/F-25 work items,
not to this upgrade.

---

## 9. Files changed for the upgrade

- `third_party/ffmpeg/` — new: `README.md`, `build.sh`, `configure-9.1.txt`, `manifest.json`,
  `sbom.cyclonedx.json`, `licenses/`, `msys2-packages.txt`, `ffmpeg-release-signing-key.asc`,
  `verify.sh`.
- `crates/arc-media/csrc/arc_av.c` — 9.x compile-time pin, `arc_av_runtime`/`arc_av_abi_size`.
- `crates/arc-media/build.rs` — version-header parsing and pin, pkg-config ranges, rebuild triggers,
  `-Werror=deprecated-declarations`, build ident and manifest hash.
- `crates/arc-media/src/{libav,tools,probe,encode,paths}.rs` — runtime identity check, tool
  resolution/verification, `file:` URLs and protocol whitelists, `-of json`, runtime report.
- `crates/arc-cli/{src/export.rs,src/main.rs,tests/render_e2e.rs}` — report block and hard tool
  verification for probe/import/render.
- `crates/arc-media/tests/{runtime.rs,param_change.rs,paths.rs,support/mod.rs,differential.rs,baseline/,fixtures/}`
  — new tests, corpus, baseline, allow-list, committed MP3/H.264 fixtures.
- `scripts/dev-env.sh`, `.gitattributes`, `Cargo.lock`, `Cargo.toml` (build deps).
- Docs: `HANDOVER.md`, `README.md`, `docs/04`, `docs/11`, `docs/adr/0003`, spec status line.
