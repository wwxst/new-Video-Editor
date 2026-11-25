# FFmpeg 9.1 Upgrade Specification

**Status:** Implemented 2026-11-25 (steps 0–3, plus the audit follow-ups). Acceptance evidence:
[HANDOVER.md](../../HANDOVER.md) §9 and the AC-1…AC-15 table in
[reports/ffmpeg-9.1-differential.md](../../reports/ffmpeg-9.1-differential.md). **Version:** 1.0,
2026-11-25.
**Audience:** DeepSeekHS, the implementer of this upgrade. ("DeepSeekHS" is not defined elsewhere in
the repository; this document treats it as the party doing the work and assumes no context beyond
the repository itself.)
**Inputs:** [audit of `88e0707`](../../reports/audit-2026-11-25-88e0707.md) (findings are cited as F-nn),
[ADR-0003](../adr/0003-ffmpeg-for-media-io.md), [04 Media Pipeline](../04-media-pipeline.md),
[11 Risks & Legal](../11-risks-and-legal.md), [HANDOVER.md](../../HANDOVER.md).

This is a dependency upgrade inside the approved architecture. It does not change the media design: the
narrow C shim, in-process preview decode, child-process import checks and exports, and dynamic LGPL
linking all stay. Anything beyond what is listed here is out of scope.

---

## 1. Goals and non-goals

### 1.1 Goals

1. Replace the pinned FFmpeg 8.1.3 with **FFmpeg 9.1 "Gauss"** for the libav shim and the bundled
   `ffmpeg`/`ffprobe` tools.
2. Make the FFmpeg dependency **reproducible and verifiable**: build recipe and source hashes in the
   repository, a compile-time version pin, and a run-time check of version and licence (F-21).
3. Bring the build in line with the documented licence posture: LGPL, no GPL, no nonfree parts, an
   explicit decision on LGPL v2.1 vs v3 (F-20), and no unlicensed software H.264 encoder (F-19).
4. **Not widen the attack surface.** 9.1 adds a Windows `handle:` protocol and a libcurl protocol; the
   upgrade restricts libav to local files (F-09).
5. Prove that the upgrade is **behaviour-preserving**, using a differential test against 8.1.3 on a
   generated media corpus.

### 1.2 Non-goals

- No architecture changes: no decode worker process, no hardware decode, no GPU YUV upload, no change to
  the export process model.
- No migration to the new swscale API (`sws_scale_frame`, `SwsContext.scaler`); the legacy API remains
  supported in 9.1 and is kept (§3.4).
- No new codecs, containers or features in Arcline, even where 9.1 adds them.
- No fixes for the accuracy defects F-06, F-07 and F-08 in this change; they are separate work items
  (§2.2). The upgrade must not make them worse (§9.2).
- No MSVC target work (optional, §5.8).
- No change to the project schema, journal format, JSON-RPC protocol or CLI output contracts, apart from
  the additive report fields in §4.5.

---

## 2. Preconditions and ordering

### 2.1 Facts as of 2026-11-25

| | Current (pinned) | Target |
|---|---|---|
| Release | 8.1.3 (BtbN `ffmpeg-n8.1.3-41-ga8029b6528-win64-lgpl-shared-8.1.zip`) | **9.1 "Gauss"**, tag `n9.1` |
| Source identity | BtbN build of branch `release/8.1` at `a8029b6528` | tag object `9d106a4c5a139fac2a62c349817017191217a9d8` → commit `74e4393595cf2b5857f0da925c4e0dd74ea306e5` (2026-11-19) |
| libavutil | 60.26.103 → `avutil-60.dll` | 61.19.100 → `avutil-61.dll` |
| libavcodec | 62.28.103 → `avcodec-62.dll` | 63.24.100 → `avcodec-63.dll` |
| libavformat | 62.12.103 → `avformat-62.dll` | 63.16.100 → `avformat-63.dll` |
| libswscale | 9.5.103 → `swscale-9.dll` | 10.5.100 → `swscale-10.dll` |
| libswresample | 6.3.103 → `swresample-6.dll` | 7.6.100 → `swresample-7.dll` |
| libavfilter / libavdevice (CLI tools only) | 11.x / 62.x | 12.8.x / 63.5.x → `avfilter-12.dll`, `avdevice-63.dll` |
| Prebuilt Windows LGPL shared build | available (BtbN) | **not yet published** by BtbN (their latest release, `autobuild-2026-11-24-12-41`, has n8.1 and n9.0 branches only) |

Every major version changed in 9.0 (APIchanges: "the last version increases of all libraries were on
2026-06-23"). The 9.x DLLs are **not** binary-compatible with 8.x; the shim must be rebuilt, and all DLLs
must be replaced together.

### 2.2 Order of work

The upgrade lands in four steps. Steps 0 and 1 go in separate commits on 8.1.3, so that step 3 compares two
FFmpeg versions running *the same* shim.

| Step | Content | Why first |
|---|---|---|
| **0. Prerequisite fixes (on 8.1.3)** | F-02 (frame size vs destination buffer), F-03 (resampler rebuild on input change), F-09 (`file:` only in the shim, §4.4; import check for client `asset.import` in the session layer) | memory-safety defects in our own code; corpus tests in step 3 would crash otherwise |
| **1. Corpus and differential harness (on 8.1.3)** | §8.2 corpus generator and §8.3 harness; record 8.1.3 baseline hashes | provides the baseline the upgrade is compared against |
| **2. Build pipeline** | §5: recipe, source pin, configure profile, manifest; produce the 9.1 package | independent of Rust code |
| **3. Switch** | §4 (shim pin, runtime check, `build.rs`, CLI tool changes), dev-env, docs | the actual version change, checked by §9 |

Steps 0 and 1 are defect fixes and test infrastructure, not part of the version change; they are listed
here because the acceptance criteria depend on them. If the owner decides otherwise, record the decision in
HANDOVER.md and mark AC-7 and AC-11 as not applicable.

---

## 3. Compatibility analysis (shim and Rust FFI)

### 3.1 FFI surface

Rust never touches FFmpeg types. The surface between Rust and C is:

- five functions: `arc_av_open`, `arc_av_info`, `arc_av_video_at`, `arc_av_audio_read`, `arc_av_close`
  (`crates/arc-media/csrc/arc_av.c`, declared in `crates/arc-media/src/libav.rs`);
- one plain struct, `ArcAvInfo` / `RawInfo` (`#[repr(C)]`, fixed-size `char` arrays);
- an opaque handle `ArcAv*`.

None of this depends on FFmpeg's ABI, so **no Rust FFI declarations change for 9.1**. No bindgen and no
`ffmpeg-sys` crate are used, and none may be introduced.

### 3.2 libav symbols used by the shim

Checked against the 9.1 headers: all present, none deprecated. The shim compiles against 9.1 with
`-std=c11 -Wall -Wextra -Wdeprecated-declarations` and zero warnings. (That was a syntax-only check: the
configure-generated `avconfig.h`/`ffversion.h` came from 8.1.3. A real 9.1 build must repeat it; AC-1.)

| Group | Symbols | 9.1 status |
|---|---|---|
| lavf | `avformat_open_input`, `avformat_find_stream_info`, `av_find_best_stream`, `av_read_frame`, `av_seek_frame`, `avformat_close_input`; fields `nb_streams`, `streams[]`, `duration`; `AVStream.{discard,codecpar,time_base,avg_frame_rate,r_frame_rate,start_time,duration}` | unchanged (`r_frame_rate` stays: `FF_API_R_FRAME_RATE` is 1 and not tied to a major) |
| lavc | `avcodec_alloc_context3`, `avcodec_parameters_to_context`, `avcodec_open2`, `avcodec_send_packet`, `avcodec_receive_frame`, `avcodec_flush_buffers`, `avcodec_free_context`, `avcodec_get_name`, `av_packet_alloc/free/unref`; fields `thread_count`, `pkt_timebase` | unchanged; behaviour notes in §3.3 |
| lavu | `av_frame_alloc/free/unref/move_ref`, `av_mallocz`, `av_realloc`, `av_free`, `av_strerror`, `av_log_set_level`, `av_rescale_q`, `av_cmp_q`, `av_get_pix_fmt_name`, `av_get_sample_fmt_name`, `av_opt_set_int`, `av_channel_layout_copy/default/uninit`, `AV_CHANNEL_LAYOUT_STEREO`, `AV_TIME_BASE_Q`; `AVFrame.{best_effort_timestamp,pts,buf,width,height,format,colorspace,color_range,data,linesize,ch_layout,sample_rate,nb_samples,extended_data}` | unchanged (`AV_CHANNEL_LAYOUT_*_BACK` deprecations do not affect `STEREO`) |
| sws | `sws_getContext`, `sws_setColorspaceDetails`, `sws_getCoefficients`, `sws_scale`, `sws_freeContext`, flags `SWS_BICUBIC`, `SWS_ACCURATE_RND`, `SWS_FULL_CHR_H_INT`, `SWS_CS_ITU709` | present; scaler flags are documented as "deprecated in favor of `SwsContext.scaler`" but not marked `attribute_deprecated`; the stateful legacy API always uses `SWS_BACKEND_LEGACY` |
| swr | `swr_alloc_set_opts2`, `swr_init`, `swr_convert`, `swr_get_out_samples`, `swr_free`; options `filter_size`, `phase_shift` | unchanged (doc-only change about `AV_CHAN_UNUSED`) |

`libavutil/imgutils.h` is included but unused; the include may stay.

### 3.3 Behaviour changes between 8.1.3 and 9.1 that touch Arcline

From `doc/APIchanges`, `Changelog` and sources at `n9.1`:

| Change | Where | Effect on Arcline | Action |
|---|---|---|---|
| `avcodec_parameters_to_context()` no longer sets `AVCodecContext.delay` from `initial_padding` (lavc 63.23.101) | `codec_par.c` | none: the shim does not read `delay`; sample skipping uses lavc's internal `skip_samples` | none; covered by audio differential tests |
| AAC decoder may raise `AVCodecContext.seek_preroll` (lavc 63.23.102) | `aac/aacdec.c` | none today; this is the value the F-06 fix needs | note for the F-06 work item |
| AAC **encoder** extensively reworked (9.1 Changelog) | lavc | exports to `.mp4`/`.mov` with AAC audio produce different bitstreams | expected difference (§9.2); the end-to-end tests use FLAC in MKV and are unaffected |
| swscale 9 → 10: new backend selection, `SwsScaler` enum | lsws | the legacy API path is documented as unchanged; RGBA output should be bit-identical | proven by the differential harness (AC-7) |
| ID3v2 `COMM` tag naming (lavf 63.6) | lavf | `ffprobe` tag names differ for MP3 comments | none (Arcline does not read them) |
| `ffprobe -print_format` is a deprecated alias of `-output_format` (also in 8.1.3) | fftools | `probe.rs` uses `-print_format` | switch to `-of json` (§4.6) |
| New protocols `handle` (Windows handle) and `libcurl` | lavf | reachable from asset paths (F-09) | exclude from the build (§5.3) and block in the shim (§4.3) |
| TLS certificate verification on by default (deferred since 8.0, effective with the lavf 63 bump) | lavf | none (network is disabled in the shipped build) | none |
| NVENC: support for SDKs older than 11.1 removed (9.0) | lavc | old NVIDIA drivers lose `h264_nvenc`; the capability trial falls back automatically | release-note item |
| `asf_o` demuxer removed (9.1); CELT decoding removed (9.0, Opus unaffected) | lavf/lavc | none for the v1 ingest list (ASF is still served by `asf`) | none |
| Removed lavc fields such as `AVCodecContext.properties`, `AV_CODEC_ID_V308/V408/V410` | lavc | not used | none |

### 3.4 Decisions on deprecated paths

- Keep `sws_getContext` + `sws_scale`. Migration to `sws_scale_frame` is a later item and needs its own
  differential proof.
- Keep `av_seek_frame`; the F-08 fix will decide whether to move to `avformat_seek_file`.
- Do not use any `FF_API_*`-guarded field. Compile with deprecation warnings as errors (§4.3, §4.7).

### 3.5 C runtime and toolchain

- The Rust target (`x86_64-pc-windows-gnu` with WinLibs GCC 16.2 UCRT), the shim and the current FFmpeg
  DLLs all link the Universal CRT (`api-ms-win-crt-*`); verified with `objdump -p`. The 9.1 build must
  target **UCRT** too (§5.4).
- No CRT objects cross the boundary: FFmpeg memory is freed by FFmpeg (`av_free`, `*_free`), Rust memory by
  Rust, and no `FILE*` is shared. Keep it that way.
- The Rust toolchain stays 1.100.0. The Rust code is not affected by the FFmpeg version.

---

## 4. Code changes

### 4.1 Files that may change

| File | Change |
|---|---|
| `crates/arc-media/csrc/arc_av.c` | §4.3 compile-time pin; §4.4 open options; §4.5 `arc_av_runtime`, `arc_av_abi_size` |
| `crates/arc-media/src/libav.rs` | `extern` declarations for the two new functions; `runtime()` API (§4.5); URL construction (§4.4) |
| `crates/arc-media/build.rs` | §4.7 |
| `crates/arc-media/src/tools.rs` | §4.6 tool resolution and verification |
| `crates/arc-media/src/probe.rs` | §4.6 `-of json`, `-i`, `file:` URLs, `-protocol_whitelist` |
| `crates/arc-media/src/encode.rs` | §4.6 `file:` output URL and input whitelists; candidate list **unchanged** (F-19 decision is in the build, §5.3) |
| `crates/arc-media/src/fixtures.rs`, `crates/arc-media/tests/*` | corpus generator and tests (§8) |
| `crates/arc-cli/src/export.rs` | additive `ffmpeg` block in the export report (§4.5) |
| `scripts/dev-env.sh` | default to the 9.1 directory (§5.5) |
| new `third_party/ffmpeg/` | recipe, configure profile, manifest, licence texts (§5) |
| new `.github/workflows/` (optional in this change) | Windows job using the pinned package (§8.5) |
| `HANDOVER.md`, `docs/04` §9, `docs/11` §2.1, `ADR-0003` version note | documentation (AC-14) |

### 4.2 Frozen interfaces (must not change)

- Public API of `arc-media`: `VideoDecoder`, `AudioDecoder`, `FrameRef`, `StreamInfo`, `MediaError`, the
  `probe`, `encode`, `tools` and `fixtures` functions (additions are allowed).
- The five existing shim functions and the layout of `ArcAvInfo`.
- `arc-core`, `arc-render`, `arc-audio`, `arc-app`: no changes.
- CLI stdout contracts, JSON-RPC methods, the project schema (`SCHEMA_VERSION` 7) and the journal format.

### 4.3 Compile-time version pin (shim)

At the top of `arc_av.c`, after the includes:

```c
#if LIBAVUTIL_VERSION_MAJOR != 61 || LIBAVCODEC_VERSION_MAJOR != 63 || \
    LIBAVFORMAT_VERSION_MAJOR != 63 || LIBSWSCALE_VERSION_MAJOR != 10 || \
    LIBSWRESAMPLE_VERSION_MAJOR != 7
#error "arc_av requires FFmpeg 9.x headers (lavu 61, lavc 63, lavf 63, sws 10, swr 7); see docs/specs/ffmpeg-9.1-upgrade.md"
#endif
#if LIBAVCODEC_VERSION_INT < AV_VERSION_INT(63, 24, 100) || LIBAVFORMAT_VERSION_INT < AV_VERSION_INT(63, 16, 100)
#error "arc_av requires FFmpeg 9.1 or newer within the 9.x series"
#endif
```

`build.rs` compiles the shim with `-Werror=deprecated-declarations` (GCC/Clang) so that a later minor
version cannot silently move the shim onto a deprecated path.

### 4.4 Local files only (shim and Rust)

- **Rust** (`libav.rs`): build the open string as `file:` + the canonical absolute path
  (`std::fs::canonicalize`, then strip the `\\?\` prefix; FFmpeg adds the extended-length prefix itself for
  long paths, `get_extended_win32_path` in `libavutil/wchar_filename.h`). Reject inputs that are not
  existing local files with `MediaError::Path`.
- **Shim** (`arc_av_open`): pass an options dictionary to `avformat_open_input` with
  `protocol_whitelist=file`, and refuse any `path` that does not start with `file:`. The whitelist also applies
  to nested opens by demuxers (playlists, references), because lavf inherits it.
- The open options otherwise keep FFmpeg defaults (`probesize`, `analyzeduration`) so that behaviour is
  unchanged. Resource limits (`max_pixels`, interrupt deadlines) belong to the hang-watchdog work (docs/04
  §1.2 step 4), not to this change.
- The session-layer part of F-09 (import check for client-supplied `asset.import`, rejecting URL schemes in
  asset paths) is step 0 and lives in `arc-cli`; this spec only requires that it is done before AC-5 is
  evaluated.

### 4.5 Run-time identity check

New shim functions (signatures are part of the FFI from now on):

```c
typedef struct ArcAvRuntime {
    unsigned build[5];    /* LIB*_VERSION_INT at compile time: avutil, avcodec, avformat, swscale, swresample */
    unsigned runtime[5];  /* avutil_version(), avcodec_version(), ... at run time */
    char license[5][48];  /* avutil_license(), avcodec_license(), ... */
    char ident[64];       /* av_version_info(), e.g. "n9.1" */
} ArcAvRuntime;

int arc_av_runtime(ArcAvRuntime *out);  /* fills *out; 0 = compatible, <0 = refused (reason in err) */
int arc_av_abi_size(void);              /* sizeof(ArcAvInfo) as compiled */
```

Rules applied by `arc_media::libav::runtime()` (computed once per process, `OnceLock`):

1. `arc_av_abi_size() == size_of::<RawInfo>()`, otherwise refuse (protects the struct mirror).
2. For each library: the run-time major equals the build major, and the run-time minor is at least the build
   minor (FFmpeg's ABI rule within a major).
3. Every licence string equals the approved one: "LGPL version 2.1 or later", or "LGPL version 3 or later"
   if decision D1 chooses v3 (§10). A licence containing "GPL version" without "LGPL", or "nonfree", is
   refused.
4. On refusal every `VideoDecoder::open` / `AudioDecoder::open` returns `MediaError::Decode("ffmpeg runtime
   refused: …")`. A developer override `ARCLINE_ALLOW_ANY_FFMPEG=1` downgrades the refusal to a warning,
   which is recorded in every export report. The override exists because LGPL users may replace the
   libraries; it must never be set in packaged builds.

The export report gains an additive block:

```json
"ffmpeg": {"ident": "n9.1", "libraries": {"avcodec": "63.24.100", "...": "..."},
           "license": "LGPL version 2.1 or later", "build_id": "<manifest sha256>", "override": false}
```

### 4.6 CLI tools (`tools.rs`, `probe.rs`, `encode.rs`)

- **Resolution order:** (1) `ARCLINE_FFMPEG_DIR/bin`, (2) the directory of the current executable, (3) `PATH`
  only when `ARCLINE_ALLOW_PATH_FFMPEG=1`. In release builds (3) is not available. (On this development machine a
  GPL "essentials" build is on `PATH`, F-21.)
- **Verification:** once per process, run `ffmpeg -hide_banner -version` and `ffprobe -hide_banner
  -version`, and compare the first line with the manifest `ident` (§5.6). A mismatch is a hard error for
  `probe`, `import` and `render`, with the same override as §4.5.
- **Inputs:** pass media as `-i file:<absolute path>` (also for `ffprobe`, which today takes the path as a
  positional argument), preceded by `-protocol_whitelist file`. For the export encoder: `-protocol_whitelist
  pipe -f rawvideo … -i pipe:0`, the audio WAV as `-protocol_whitelist file -i file:<wav>`, and the output as
  `file:<absolute output path>`.
- **ffprobe:** `-of json` instead of the deprecated `-print_format json`. The JSON fields used by
  `parse_probe` (`streams[].{codec_type,codec_name,width,height,pix_fmt,avg_frame_rate,r_frame_rate,
  time_base,duration_ts,nb_frames,sample_rate,channels,side_data_list[].rotation,tags.rotate,
  disposition.attached_pic}`, `format.{format_name,duration}`) exist unchanged in 9.1's `ffprobe.c`.
- **Encoder listing:** the `-encoders` table keeps its six flag characters followed by the name
  (`fftools/opt_common.c` at `n9.1`), so the parser in `tools::encoders()` stays valid; add a test that
  parses the 9.1 output.
- **Options used by the exporter** (`-f rawvideo`, `-pix_fmt`, `-s`, `-framerate`, `-map`, `-shortest`,
  `-c:v`, `-c:a`, `-vf scale=out_color_matrix=bt709:out_range=tv`, `-colorspace`, `-color_primaries`,
  `-color_trc`, `-level`, `-g`, `-profile:v`, `-y`, `-nostdin`): all present at `n9.1`
  (`fftools/ffmpeg_opt.c`, `libavfilter/vf_scale.c`).

### 4.7 `build.rs`

- Parse `LIB*_VERSION_MAJOR/MINOR/MICRO` from `include/lib*/version_major.h` and `version.h`; fail with a
  clear message unless they satisfy §4.3. (The `#error` is the backstop; the build script gives the better
  message.)
- `cargo:rerun-if-changed` for those header files and for each `lib/lib*.dll.a`, so replacing FFmpeg in place
  rebuilds the shim (today only `csrc/arc_av.c` and the variable trigger a rebuild).
- `pkg-config` fallback: require `libavcodec >= 63.24.100` and `< 64`, and the matching ranges for the other
  four libraries, instead of `atleast_version("0")`.
- Emit `cargo:rustc-env=ARC_FFMPEG_BUILD_IDENT=<value>` from `include/libavutil/ffversion.h` for reports.
- Add `-Werror=deprecated-declarations` (non-MSVC) and keep `-std=c11`.

---

## 5. Build requirements

### 5.1 Source pin

- Source: the release tarball `ffmpeg-9.1.tar.xz` from ffmpeg.org, verified with its detached signature
  (`.asc`) against the FFmpeg release signing key; record the key fingerprint and the tarball SHA-256 in the
  manifest. If the tarball is not used, build from git tag `n9.1` and verify that the tag resolves to
  commit `74e4393595cf2b5857f0da925c4e0dd74ea306e5`.
- Patches: none. Any patch requires a manifest entry and must be published with the source (§6).

### 5.2 Build host

- Recommended: cross-compile in a pinned Linux container (image referenced by digest) with
  `x86_64-w64-mingw32` GCC, UCRT runtime, NASM (FFmpeg dropped yasm support in 8.0; use a version that
  `configure` accepts and record it), `pkg-config`, `make`. This is BtbN's method and is reproducible.
- Acceptable: MSYS2 UCRT64 on Windows with pinned package versions.
- Record the GCC, binutils, mingw-w64 and NASM versions in the manifest.

### 5.3 Configure profile (`third_party/ffmpeg/configure-9.1.txt`)

Start from this profile. A component that turns out to be missing shows up in the corpus and CLI tests
(§8); add it with a one-line justification in the profile file.

```
--prefix=<staging> --target-os=mingw32 --arch=x86_64 --cross-prefix=x86_64-w64-mingw32-
--enable-shared --disable-static --disable-debug --disable-doc --disable-ffplay
--extra-version=arcline-<manifest-short-hash>
--extra-cflags=-fstack-protector-strong --extra-ldflags=-fstack-protector-strong   # see S7; not --toolchain=hardened (ELF-only flags)
--disable-gpl --disable-nonfree                              # D1: add --enable-version3 only if approved
--disable-autodetect                                         # nothing enters the build implicitly
--enable-w32threads --enable-zlib
--enable-d3d11va --enable-d3d12va --enable-dxva2             # system APIs (hardware decode comes later, docs/04 §3)
--enable-ffnvcodec --enable-nvenc --enable-amf --enable-libvpl   # NVENC / AMF / QSV encoders (MIT headers/dispatcher)
--enable-libdav1d --enable-libsvtav1                         # AV1 decode and software encode (BSD-style)
--disable-network --disable-protocols --enable-protocol=file,pipe
--disable-demuxer=hls,dash,concat,sdp,rtsp,rtp               # demuxers that open further URLs
--disable-muxer=tee,segment,hls,dash
--disable-indevs --enable-indev=lavfi --disable-outdevs      # lavfi: encoder trial and fixtures use -f lavfi
```

Decoders, encoders, parsers, filters, bitstream filters and the remaining demuxers and muxers keep their
defaults. Ingest breadth is a product requirement (docs/04 §4); the CLI needs filters for exports and later
for proxies, thumbnails and waveforms; and the licence-relevant encoders are controlled by what the profile
enables, not by an allow-list. Results of this profile:

- **Excluded:** all GPL and nonfree libraries; the version-3 libraries (gmp, libaribb24, libastcenc,
  opencore-amr, vo-amrwbenc, mbedtls, rkmpp) unless D1 approves them; `libopenh264` (F-19); `libcurl`;
  every network protocol; `handle` and `fd` (Windows handle and file-descriptor protocols).
- **Media Foundation** is not enabled, so `h264_mf`/`hevc_mf` do not exist (D2). If D2 approves them, add
  `--enable-mediafoundation`.
- **Opus, MP3, Vorbis and VP9** are decoded by FFmpeg's native decoders; no external library is needed.

The 9.1 `configure` lists autodetected components (threads, AMF, ffnvcodec, D3D12VA, …) as
`--disable-<name> [autodetect]`; with `--disable-autodetect` they must be enabled explicitly, as above.

### 5.4 Outputs

- DLLs: `avutil-61.dll`, `avcodec-63.dll`, `avformat-63.dll`, `swscale-10.dll`, `swresample-7.dll`,
  `avfilter-12.dll`, `avdevice-63.dll`.
- Tools: `ffmpeg.exe`, `ffprobe.exe`.
- Development files: `include/`, `lib/lib*.dll.a` (GNU import libraries); optionally `.lib`/`.def` for MSVC.
- Unstripped copies of the DLLs, kept for crash symbolication and **not** shipped.
- `LICENSE.txt`/`COPYING.LGPLv2.1` (or v3), the configure line (`ffmpeg -buildconf`), and the manifest.

### 5.5 Developer setup

- Package directory: `~/.local/toolchains/ffmpeg-9.1-lgpl` (`include/`, `lib/`, `bin/`).
- `scripts/dev-env.sh`: prefer `ffmpeg-9.1-lgpl`, keep `ARCLINE_FFMPEG_DIR` as the override, print the
  selected version. During the transition, 8.1.3 stays installed for rollback (§11).
- After switching directories: `cargo clean -p arc-media` (until the §4.7 rebuild triggers exist).

### 5.6 Manifest (`third_party/ffmpeg/manifest.json`)

Required fields: version, tag, commit, tarball SHA-256, signature key fingerprint, configure line, host
toolchain versions, build date, SHA-256 of every shipped file, licence decision (D1), `ident` string, and
the SBOM file name. The `build_id` in export reports is the manifest's SHA-256.

### 5.7 Prebuilt fallback

If an in-repo build is not ready, a BtbN `ffmpeg-n9.1-*-win64-lgpl-shared-9.1.zip` may be used **for
development only** once published: verify it against `checksums.sha256` and the GitHub asset digest, record
both in the manifest, and note that it is an LGPL v3 build with about 60 bundled external libraries and
network protocols (as 8.1.3 is today). Shipping it requires decision D1 = v3 plus a complete third-party
notice set.

### 5.8 MSVC (optional)

Not required. If attempted: compile the shim with `/std:c11` (compound literals) and link the `.lib`
import libraries; the compile-time pin and run-time check apply unchanged.

---

## 6. LGPL compliance requirements

Engineering checklist; counsel signs off before release (docs/11 §6).

1. **Dynamic linking only.** FFmpeg ships as separate DLLs next to the executables. No static linking of
   any FFmpeg library into Arcline binaries.
2. **Replaceability.** The user can replace the DLLs with a compatible build: the shim uses only public
   API, and the run-time check allows an override (§4.5). No signature pinning of the FFmpeg DLLs is enforced
   against the user.
3. **Source.** Publish, for each release, the exact FFmpeg source (tarball or tag), the build recipe, the
   configure profile, any patches and the manifest, at a stable URL or alongside the release (a written
   offer is the fallback).
4. **Notices.** Ship the LGPL text and the FFmpeg copyright notice; show them in the About / legal screen
   and in the installer. Ship the licence notices of every external library compiled into the DLLs (for the
   §5.3 profile: dav1d, SVT-AV1 with its AOMedia patent licence, zlib, libvpl, nv-codec-headers, AMF
   headers; take each licence text from the pinned source tree and record it in the SBOM), and of the
   MinGW runtime DLLs if shipped (winpthreads; GCC Runtime Library Exception). FFmpeg's `LICENSE.md` also
   requires crediting the Independent JPEG Group in the documentation of distributed executables.
   The LGPL also requires that Arcline's licence terms permit modification of the combined work and
   reverse engineering to debug such modifications; counsel reviews the EULA for this.
5. **No GPL or nonfree.** `ffmpeg -buildconf` of the shipped build contains none of `--enable-gpl` and
   `--enable-nonfree`, and contains `--enable-version3` only if D1 approved it. `ffmpeg -L` states LGPL.
6. **Patents.** Unchanged posture (docs/11 §2.2): hardware H.264/HEVC encoders; SVT-AV1 for software AV1;
   no `libopenh264` (a build from source has no Cisco patent coverage); `h264_mf` only if D2 approves it.
7. **Version 3 (only if D1 = v3).** The LGPLv3 installation-information clause applies to "User Products";
   counsel confirms that it does not apply to desktop software distribution, and the third-party list in
   item 4 grows by the version-3 libraries actually enabled.
8. **SBOM.** An SPDX or CycloneDX SBOM of the FFmpeg package (FFmpeg plus each external library with
   version and licence) is produced by the build and published with the release.

---

## 7. Security requirements

| # | Requirement | Verified by |
|---|---|---|
| S1 | libav opens only `file:` URLs with `protocol_whitelist=file`; asset paths with a URL scheme are rejected in the session layer | AC-5 |
| S2 | The shipped build has no network protocols and no `handle`, `fd` or `libcurl` protocols; `ffmpeg -protocols` lists only `file` and `pipe` | AC-4 |
| S3 | Demuxers that open further URLs (`hls`, `dash`, `concat`) are absent from the shipped build | AC-4 |
| S4 | The shim never writes outside the buffer it was given and rebuilds converters on any parameter change (F-02, F-03 fixed in step 0) | AC-11 |
| S5 | CLI tools are resolved from the bundle only and verified against the manifest (§4.6) | AC-3 |
| S6 | Supply chain: source signature and SHA-256 verified; every shipped file hashed in the manifest | AC-13 |
| S7 | All shipped DLLs and EXEs have `DYNAMIC_BASE`, `HIGH_ENTROPY_VA` and `NX_COMPAT` (true for 8.1.3 today, checked with `objdump -p`). Stack protection with `-fstack-protector-strong` if the MinGW toolchain links it (needs `libssp`); FFmpeg's `--toolchain=hardened` is not used because it adds ELF-only linker flags (`-z relro`, `-z now`, `-pie`) | AC-4 |
| S8 | DLLs are installed next to the executables; FFmpeg's own optional loads (`nvcuda`, AMF runtime, the libvpl dispatcher) already use `LOAD_LIBRARY_SEARCH_APPLICATION_DIR \| SYSTEM32` (`compat/w32dlfcn.h` at `n9.1`) | review |
| S9 | Vulnerability process: watch FFmpeg security announcements and CVE feeds weekly (docs/11 §2.1); adopt a 9.1.x point release with decoder or demuxer security fixes within 14 days, re-running AC-6 to AC-10 | process |

---

## 8. Testing

### 8.1 Existing tests

All 65 existing tests must pass unchanged on 9.1, in three consecutive full runs (`cargo test
--workspace`), plus clippy clean (`CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets`).

### 8.2 Generated media corpus (step 1)

Generated by `arc_media::fixtures` (or a test helper calling the bundled `ffmpeg`). No third-party media
(docs/08 §6). Each entry has an expected outcome; entries for open defects are expected failures, marked with
the finding id until fixed.

Formats the shipped build cannot encode (MP3; H.264 on machines without a hardware encoder) are generated
once by a documented command and committed as small binary fixtures (≤ 200 KB each). Their provenance
(tool, version, command) goes in `crates/arc-media/tests/fixtures/README.md`, and `.gitattributes` marks
`*.mkv`, `*.mp3`, `*.ts`, `*.m4a` and `*.ogg` as binary. Opus fixtures use the native encoder
(`-c:a opus -strict experimental`).

| Entry | Expected |
|---|---|
| Counter video FFV1/MKV at 29.97, 25, 30 (existing) | frame-exact, sequential and random |
| Counter MPEG-2 in MKV, open GOP, B-frames | frame-exact |
| Counter MPEG-2 in MPEG-TS with non-zero start time | F-08 (expected failure until fixed) |
| Counter MPEG-4 Part 2 in MP4 with B-frames | F-08 (random access before keyframes) |
| Counter H.264 in MP4 via the available hardware encoder, B-frames | frame-exact |
| PNG and JPEG stills | exact pixels |
| MPEG-2 with a resolution change (experiment J) | no crash; scaled to the stream size or typed error (F-02) |
| MP2 stereo → mono (experiment K) | no crash; correct downmix or typed error (F-03) |
| Audio: FLAC 48 kHz, WAV, FLAC 44.1 kHz, MP3, Opus (Ogg and MKV), AAC (sine and white noise) | FLAC/WAV exact; others per F-06/F-07/F-25 |
| Asset path `http://127.0.0.1:9/x.mp4`, `handle:1`, `concat:a\|b` | refused before any I/O (S1) |
| Truncated file, zero-byte file, text file renamed `.mp4` | import check `offline`; no crash |

### 8.3 Differential harness (step 1, then step 3)

A test binary decodes every corpus entry through the shim and writes, per entry: BLAKE3 of each decoded RGBA
frame (sequential and random access), BLAKE3 of the decoded audio (one long read, and the mixer's block
pattern), the parsed `Probe`, and the import-check outcome. Step 1 stores the 8.1.3 results as a baseline
file; step 3 runs the same binary against 9.1 and compares:

- video frame hashes: **identical**, or a documented and reviewed tolerance (≤ 1 LSB per channel, with a
  written reason), entry by entry;
- audio from FLAC, WAV and PCM: identical; from lossy codecs: identical or within the documented tolerance
  (AAC: see F-25);
- `Probe`: identical except documented fields;
- import-check outcome: identical.

### 8.4 Other tests

- **Runtime check:** the ABI-size check; refusal with a stubbed licence string; refusal on a major or minor
  mismatch (a unit-testable comparison function); the override path.
- **Tools:** parse the 9.1 `-encoders` table; the `ffprobe -of json` mapping test runs on real 9.1 output;
  `file:` URLs with spaces, non-ASCII characters and long paths on Windows.
- **Encoders:** end-to-end frame-exact export with `ffv1` and the available hardware H.264 encoder (as
  today); AV1 with `libsvtav1` decodes and passes the counter check; `prores_ks` and `dnxhd` produce
  decodable files.
- **Performance smoke:** 1080p decode throughput and a 60-second FFV1 export within ±10 % of 8.1.3 on the
  reference machine.

### 8.5 CI

If the workflow is added in this change (ADR-0011): Windows job with the pinned package from the manifest
(cached, hash-verified), `ARCLINE_FORCE_FALLBACK_ADAPTER=1` for GPU tests, the full test suite, the
differential harness, and the AC-4 build checks.

---

## 9. Acceptance criteria

### 9.1 Criteria

| # | Criterion |
|---|---|
| AC-1 | `cargo build --workspace` succeeds against the 9.1 package; the shim compiles with `-Wall -Wextra -Werror=deprecated-declarations` and no warnings |
| AC-2 | Building against 8.x headers fails with the §4.3/§4.7 message; building against 9.1 passes |
| AC-3 | `objdump -p arcline-cli.exe` lists `avcodec-63.dll`, `avformat-63.dll`, `avutil-61.dll`, `swscale-10.dll`, `swresample-7.dll` and none of the 8.x names; mismatched DLLs or tools are refused at run time with a clear message; the export report contains the §4.5 `ffmpeg` block |
| AC-4 | Shipped build: `ffmpeg -L` states LGPL (v2.1+, or v3+ per D1); `-buildconf` has no `--enable-gpl` or `--enable-nonfree`; `-protocols` lists only `file` and `pipe`; `-demuxers` has no `hls`, `dash`, `concat`; `-encoders` has no `libopenh264` (and no `h264_mf` unless D2); all DLLs/EXEs have `DYNAMIC_BASE`, `HIGH_ENTROPY_VA`, `NX_COMPAT` |
| AC-5 | Opening `http://…`, `handle:…`, `concat:…` or a path with any other scheme fails before any network or handle access, in both the shim and the CLI tools |
| AC-6 | All 65 existing tests pass, three consecutive runs; clippy clean |
| AC-7 | The differential harness shows no unexplained difference between 8.1.3 and 9.1 (§8.3) |
| AC-8 | Encoder selection on the reference machine is unchanged (`h264_qsv` today); AV1 via `libsvtav1` works; frame-exact FFV1 and H.264 exports pass |
| AC-9 | `Probe` results for the corpus equal the 8.1.3 baseline, except documented fields |
| AC-10 | Import-check outcomes for the corpus equal the baseline |
| AC-11 | Corpus entries J and K neither crash nor return wrong data (step 0 fixes) |
| AC-12 | Performance within ±10 % of 8.1.3 (§8.4) |
| AC-13 | `third_party/ffmpeg/` contains the recipe, configure profile, manifest with hashes, licence texts and SBOM; the source signature was verified and recorded |
| AC-14 | Docs updated: HANDOVER §1 (toolchain table), docs/04 §9, docs/11 §2.1, ADR-0003 version note, this spec's status line |
| AC-15 | Rollback rehearsed: pointing `ARCLINE_FFMPEG_DIR` back to 8.1.3 and reverting the §4.3 pin restores a green build and test run |

### 9.2 Expected differences (not failures)

- AAC-encoded audio in `.mp4`/`.mov` exports (encoder rework).
- `ffprobe` tag names for ID3v2 comments.
- Anything else must be explained in the differential report before acceptance.

---

## 10. Decisions required from the owner

| # | Decision | Default if undecided |
|---|---|---|
| D1 | Licence version of the shipped FFmpeg: LGPL v2.1+ (minimal build, §5.3) or LGPL v3+ (allows the version-3 libraries and the BtbN builds) | **v2.1+** |
| D2 | Software H.264 fallback: `h264_mf` (Windows Media Foundation) allowed? (`libopenh264` is excluded either way, F-19) | **excluded** |
| D3 | In-repo build (§5.2) or BtbN prebuilt for development until the in-repo build exists (§5.7) | in-repo build; BtbN for development only |
| D4 | Whether step 0 (F-02, F-03, F-09) lands before the upgrade as specified | **yes** |

Record each decision in HANDOVER.md and in `third_party/ffmpeg/manifest.json`.

---

## 11. Rollout and rollback

- **Branch:** one feature branch for steps 2–3; steps 0–1 merge first.
- **Transition:** keep the 8.1.3 package installed; `scripts/dev-env.sh` prefers 9.1 when present.
- **Rollback:** set `ARCLINE_FFMPEG_DIR` to the 8.1.3 package, revert the §4.3 pin commit and the manifest
  switch, then `cargo clean -p arc-media` and rebuild. Project files are not affected: no schema change.
- **Release notes:** FFmpeg 9.1 (LGPL), the NVENC driver minimum (SDK ≥ 11.1), and where to find the source
  and licence package.

---

## 12. Work breakdown (estimates for one developer)

| Item | Estimate |
|---|---|
| Step 0: F-02, F-03 fixes; shim `file:` whitelist; session-layer import check (F-09) | 1.5–2 d |
| Step 1: corpus generator, differential harness, 8.1.3 baseline | 2 d |
| Step 2: build recipe, configure profile, container, manifest, SBOM | 2–3 d |
| Step 3: pin, runtime check, `build.rs`, tools changes, report block | 1 d |
| Verification (§8–§9), performance smoke, fixes found by the harness | 1–2 d |
| Compliance package, docs, release notes | 1 d |
| **Total** | **8.5–11 d** |

---

## Appendix A — How the compatibility facts were established

- Headers: `diff --strip-trailing-cr` of the installed 8.1.3 headers against `n9.1` for `avformat.h`,
  `avcodec.h`, `codec_par.h`, `packet.h`, `frame.h`, `channel_layout.h`, `opt.h`, `codec.h`, `codec_id.h`,
  `defs.h`, `avio.h`, `pixfmt.h`, `samplefmt.h`, `mem.h`, `swscale.h`, `swresample.h`
  (`imgutils.h`, `pixdesc.h`, `error.h`, `log.h`, `mathematics.h`, `rational.h`, `codec_desc.h` unchanged).
- Version headers: `libavutil/version.h` (61.19.100), `libavcodec/version.h` (63.24.100),
  `libavformat/version.h` (63.16.100), `libswscale/version.h` (10.5.100), `libswresample/version.h`
  (7.6.100), `libavfilter/version.h` (12.8), `libavdevice/version.h` (63.5).
- Behaviour: `doc/APIchanges` from the 9.1 cut back to 8.1; `Changelog` for 9.0 and 9.1;
  `libavcodec/codec_par.c` (8.1.3 vs 9.1: `delay` no longer assigned); `libavcodec/aac/aacdec.c`
  (`seek_preroll`); `libavformat/file.c` (`handle` protocol); `fftools/ffprobe.c` (`-print_format` alias);
  `fftools/opt_common.c` (encoder table format); `libavfilter/vf_scale.c` (`out_color_matrix`,
  `out_range`); `configure` (version-3 and nonfree library lists, `--toolchain=hardened`).
- Shim compile check: `gcc -fsyntax-only -std=c11 -Wall -Wextra -Wdeprecated-declarations` against both
  header sets (0 warnings each).
