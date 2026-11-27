# `third-probe` — API notes for the `ffmpeg-the-third` path

Probe crate: `poc/media-validation/crates/third-probe` (binary `third-probe`).
Purpose: exercise FFmpeg 9.1 through the pure-Rust binding crate `ffmpeg-the-third = "6.0"`
so its cost/ergonomics can be compared with the project's own C shim (`arc-media` / `shim-probe`).

Host: Windows x86_64, rustc 1.100.0 `stable-x86_64-pc-windows-gnu`, MinGW-w64 binutils/gcc,
FFmpeg `9.1-arcline-f414c1c54e` (LGPL, shared, dev package at `%USERPROFILE%\.local\toolchains\ffmpeg-9.1-lgpl`).

---

## 1. Cargo features used and why

`crates/third-probe/Cargo.toml`:

```toml
[dependencies]
ffmpeg-the-third = "6.0"
serde_json.workspace = true
```

No cargo features are requested, so the crate's **default set** is used:

```
default = ["codec", "device", "filter", "format", "software-resampling", "software-scaling"]
```

(`ffmpeg-the-third` 6.0.0 Cargo.toml:118-125.)

| default feature | why it is needed here |
|---|---|
| `format` | `ffmpeg::format::input`, packets, streams, seeking, duration |
| `codec` | decoder contexts, `send_packet` / `receive_frame`, packets |
| `filter` | `filter::Graph` for `--mode filter` |
| `software-scaling` | `software::scaling::Context` (YUV420P → RGB24) |
| `software-resampling` | `software::resampling::Context` (44.1 kHz mono → 48 kHz stereo f32) |
| `device` | pulled in by default; not used directly (no `avdevice` call in the probe) |

Deliberately **not** enabled:

* `build` / `static` — those compile FFmpeg from source. We link the prebuilt shared FFmpeg 9.1
  (`FFMPEG_DIR`) instead, which is the same DLL set `arc-media` uses, so timings are comparable.
* `serialize`, `image`, `postprocessing`, `rpi` — not needed.

Resolved versions (`poc/media-validation/Cargo.lock`):

```
ffmpeg-the-third     6.0.0+ffmpeg-9.0
ffmpeg-sys-the-third 6.0.0+ffmpeg-9.0
bindgen              0.72.1          (build dependency, runtime libclang)
clang-sys            1.9.1
clang                2.1.0           (build dependency, used by check_features)
```

The crate's lib target is named `ffmpeg_the_third` (Cargo.toml:135-136), not `ffmpeg`, so
`src/main.rs:21` aliases it: `use ffmpeg_the_third as ffmpeg;`.
(First build failure was exactly this: `error[E0432]: unresolved import 'ffmpeg' … use of unresolved module or unlinked crate`.)

---

## 2. Build prerequisites, and the exact errors hit before it built

### 2.1 Final working invocations

```powershell
$env:PATH = "$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-gnu\bin;" +
            "$env:USERPROFILE\.local\toolchains\mingw64\bin;" +
            "$env:USERPROFILE\.local\toolchains\ffmpeg-9.1-lgpl\bin;" +
            "D:\Develop\Projects\arcline-validation\llvm\libclang;" + $env:PATH
$env:ARCLINE_FFMPEG_DIR = "$env:USERPROFILE\.local\toolchains\ffmpeg-9.1-lgpl"
$env:FFMPEG_DIR         = $env:ARCLINE_FFMPEG_DIR
$env:LIBCLANG_PATH      = "D:\Develop\Projects\arcline-validation\llvm\libclang"
$env:CPATH              = "D:\Develop\Projects\arcline-validation\llvm\libclang\lib\clang\18.1.1\include;" +
                          "D:\Develop\Projects\arcline-validation\llvm\llvm-mingw-20250910-ucrt-x86_64\include"
$env:BINDGEN_EXTRA_CLANG_ARGS = "--target=x86_64-w64-windows-gnu"

cargo build --release -p third-probe      # from poc\media-validation
# -> Finished `release` profile [optimized] target(s) in 9.48s (ffmpeg-sys force-rebuilt)
```

The same environment is captured in `poc\media-validation\build-third.ps1`.

`FFMPEG_DIR` is enough to make `ffmpeg-sys-the-third` link the prebuilt DLLs; **pkg-config is not
required** (build.rs:726 takes the `FFMPEG_DIR` branch before the `pkg_config` fallback at
build.rs:755). `lib\pkgconfig\*.pc` exists in the FFmpeg package but no `pkg-config.exe` was needed.

### 2.2 libclang: the staged copy is 32-bit

The staged copy `D:\Develop\Projects\arcline-validation\libclang` cannot be loaded by a 64-bit
process. PE check:

```
libclang.dll          sig=0x00004550 machine=0x014C  (i386)   14,068,320 bytes
libLLVM_android.dll   sig=0x00004550 machine=0x014C  (i386)   25,287,264 bytes
```

A minimal `LoadLibraryA` test compiled with the MinGW gcc gave:

```
LoadLibrary failed err=193          # ERROR_BAD_EXE_FORMAT
```

Implication: bindgen and the `clang` crate (both load `libclang.dll` into the 64-bit rustc process)
can never use this copy, regardless of `LIBCLANG_PATH`. A 32-bit Rust toolchain is not installed,
and linking a 32-bit probe against the x64 FFmpeg DLLs would not work either.

### 2.3 Attempted alternatives

* **llvm-mingw 20250910 (clang 21)** — downloaded, but it ships only `libclang-cpp.dll` plus
  compiler-rt; there is **no C-API `libclang.dll`**, so bindgen cannot use it. It is still useful
  for its mingw sysroot C headers (see below).
* **PyPI `libclang` wheel** — contains a monolithic x86_64 Windows `libclang.dll`; this works.

Downloaded artifacts (all outside the workspace, under the sanctioned
`D:\Develop\Projects\arcline-validation\llvm`):

| artifact | URL | bytes | time |
|---|---|---|---|
| libclang 18.1.1 (win_amd64 wheel) | `https://files.pythonhosted.org/packages/0b/2d/3f480b1e1d31eb3d6de5e3ef641954e5c67430d5ac93b7fa7e07589576c7/libclang-18.1.1-py2.py3-none-win_amd64.whl` | 26,415,083 | 4.8 s |
| llvm-mingw 20250910 ucrt x86_64 | `https://github.com/mstorsjo/llvm-mingw/releases/download/20250910/llvm-mingw-20250910-ucrt-x86_64.zip` | 180,993,036 | 26.5 s |
| LLVM 18.1.8 sources (headers only) | `https://github.com/llvm/llvm-project/releases/download/llvmorg-18.1.8/llvm-project-18.1.8.src.tar.xz` | 132,067,260 | 21.6 s |

Placement used:

```
D:\Develop\Projects\arcline-validation\llvm\
  libclang\libclang.dll                                  (83,988,992 B, PE machine 0x8664)
  libclang\lib\clang\18.1.1\include\*                    (190 headers from clang/lib/Headers)
  llvm-mingw-20250910-ucrt-x86_64\include\*              (mingw-w64 C library headers + crt)
  llvm-mingw-20250910-ucrt-x86_64\lib\clang\21\include\* (clang 21 resource headers, unused in the end)
```

Verified loader/version check:

```
loaded ok handle=00007FF8A4F30000
createIndex=00007FF8A4F46F8B  getClangVersion=00007FF8A4F56FC1
version: clang version 18.1.1 (https://github.com/llvm/llvm-project.git dba2a75e9c7ef81fe84774ba5eee5e67e01d801a)
```

During the successful build the `ffmpeg-sys-the-third` build script printed:

```
[ffmpeg-sys-the-third 6.0.0+ffmpeg-9.0] loaded clang version: clang version 18.1.1 (...)
```

### 2.4 C headers were the second blocker

`libclang` needs its builtin headers to parse FFmpeg's `<stdint.h>` etc. Exact diagnostics collected
with a small `clang_parseTranslationUnit` harness:

1. No resource/system headers at all:
   ```
   C:\...\check.c:1: fatal error: 'stdint.h' file not found
   ```
2. libclang 18 + llvm-mingw's clang **21** resource headers (version mismatch):
   ```
   ...\lib\clang\21\include\stdint.h:822: error: function-like macro '__INT16_C' is not defined
   C:\Users\ww\.local\toolchains\ffmpeg-9.1-lgpl\include\libavutil\common.h:33: fatal error: 'errno.h' file not found
   ```
3. libclang 18 + **clang 18.1.8** resource headers + mingw sysroot headers: clean.
   (`check_all.c` covering the build.rs header set parses without diagnostics; only the deliberately
   over-broad test that also included `hwcontext_amf.h` needed the external AMF SDK, which
   `ffmpeg-sys-the-third` does not request.)

Why `CPATH`: `ffmpeg-sys-the-third` parses FFmpeg headers **twice** during a build:

* `check_features()` (build.rs:535) uses the `clang` crate to evaluate `FF_API_*` and
  `LIB*_VERSION_*` macros (build.rs:536 `clang::Clang::new()`). It passes only
  `-I<ffmpeg>/include` and **offers no hook** for extra include dirs.
* `bindgen` (build.rs:810) generates `bindings.rs`; this one honours
  `BINDGEN_EXTRA_CLANG_ARGS` (used for `--target=x86_64-w64-windows-gnu`).

Setting `CPATH` to the clang-18 resource headers + mingw sysroot fixes the first parse; without it
the `check_features` TU fails and the build panics on
`FFmpeg 5.1 or higher is required, but found avcodec version (0, 0)`. libclang does honour `CPATH`
(verified independently: `lc4.exe check.c` with only `CPATH` set returned `diagnostics=0`).

---

## 3. What the safe API made easy vs. what needed raw FFI / `unsafe`

### 3.1 Easy (typed, RAII, no `unsafe` at the call site)

* **Demux + stream selection**
  `format::input(path)` (src/format/mod.rs:40), `ictx.packets()` (src/format/context/input.rs:43),
  `ictx.streams().best(Type::Video|Audio)` (src/format/context/common.rs:223).
* **Decoder setup** — `codec::context::Context::from_parameters(stream.parameters())`
  (src/codec/context.rs:58) then `.decoder().video()` / `.audio()`
  (src/codec/decoder/opened.rs:15/23); no manual `avcodec_alloc_context3` + `avcodec_parameters_to_context`.
* **Send/receive loop** — `send_packet` (opened.rs:39), `receive_frame` (opened.rs:73),
  `send_eof` (opened.rs:50), `flush` (opened.rs:106). Frame/`Packet` are owned, no buffer juggling.
* **Colour conversion** — `software::scaling::Context::get(...)` +
  `run(&frame, &mut out)` (src/software/scaling/context.rs:36-165). `run` also re-validates
  `InputChanged`/`OutputChanged` and allocates the destination frame on first use.
* **Channel layouts** — `ChannelLayout`, `ChannelLayoutMask`, `ChannelLayout::STEREO`
  (src/util/channel_layout/layout.rs:17, 80, 340) replace raw `u64` masks.
* **Format enums** — `Pixel`, `Sample`, `Rational` are typed, `Debug`, `Copy`; `Rational` gives
  `numerator()/denominator()` for the buffer-source arguments.
* **Filter plumbing (partly)** — `Graph::new/add/validate` (src/filter/graph.rs:32/53/44),
  `Context::source()/sink()` (src/filter/context/context.rs:36/40), `Source::add`
  (src/filter/context/source.rs:22), `Sink::frame` (src/filter/context/sink.rs:17).
  Feeding decoded frames and pulling filtered frames needs no pointer work at the call site.
* `ffmpeg::init()` is essentially a no-op in this generation
  (`src/lib.rs`: only `init_device()`), because FFmpeg 9 no longer needs `av_register_all`.

### 3.2 Required raw FFI (`ffmpeg::ffi::`) or `unsafe`

1. **Numeric pixel format for the `buffer` source.**
   `format::Pixel` is a fieldless enum **without** `AVPixelFormat` discriminants
   (src/util/format/pixel/mod.rs:18), so `pix as i32` is meaningless. The probe uses the generated
   newtype instead: `ffmpeg::ffi::AVPixelFormat::from(pixel).0 as i32`
   (`src/main.rs:583`; conversion at src/util/format/pixel/mod.rs:940).
   Without this, `buffer=…:pix_fmt=…` cannot be expressed.

2. **Connecting a filter that was added with `Graph::add`.**
   `Graph::add` (src/filter/graph.rs:53) only creates the `AVFilterContext`; nothing links it.
   The safe `Parser` (src/filter/graph.rs:130-202) only links filters that are part of the parsed
   *string*; with pre-added buffersrc/buffersink/filter it parsed `[in]scale=1280:720[out]` but left
   the pads unconnected. `graph.validate()` then failed with the exact libavfilter message:
   ```
   Output pad "default" with type video of the filter instance "in" of buffer
   not connected to any destination
   graph validate: Invalid argument
   ```
   Fix: link pad 0→0 explicitly with `ffmpeg::ffi::avfilter_link` (`src/main.rs:612-619`):
   buffer.out0 → filter.in0, filter.out0 → buffersink.in0. This is the only place the probe needs
   the C API for the graph. (It also means the probe supports only single-input/single-output
   filters such as `scale`; a general chain would need pad discovery via
   `AVFilterContext::input_pads`/`output_pads`.)

3. **Two live filter contexts.**
   `Graph::get<'b>(&'b mut self, …)` (src/filter/graph.rs:81) borrows the graph mutably for the
   lifetime of the returned `Context`, so
   ```rust
   let mut src_ctx = graph.get("in")?;
   let mut sink_ctx = graph.get("out")?;   // error[E0499]: cannot borrow `graph` as mutable more than once
   ```
   does not compile. `graph_pad()` (`src/main.rs:553`) takes the raw `*mut AVFilterContext` once,
   drops the borrow-carrying handle, and re-wraps it with the unsafe
   `filter::Context::wrap` (src/filter/context/context.rs:19). The graph still owns the contexts,
   so the pointer stays valid.

4. **Resampler output capacity — a real bug workaround (most important finding).**
   `software::resampling::Context::run` allocates the output frame as
   `output.alloc(self.output.format, input.samples(), self.output.channel_layout)`
   (src/software/resampling/context.rs:239-245) — i.e. *input* sample count, but at the *output*
   rate. `swr_convert_frame` then can only write `out->nb_samples` samples, so **upsampling silently
   truncates every frame** (~1.088× missing for 44.1 kHz → 48 kHz), and `flush`
   (context.rs:258-270) does not pre-allocate a frame either, so the tail is lost.
   Measured before the workaround: `samples = 242,705` (exactly the 44.1 kHz packet count) instead
   of the correct 48 kHz count. After pre-allocating the output frame at the resampled size
   (`out_frame()`, `src/main.rs:462`; `frame::Audio::new(F32(Packed), cap, STEREO)`),
   `samples = 264,169`, byte-identical to
   `ffmpeg -i city_street.mp4 -vn -f f32le -ac 2 -ar 48000` (2,113,352 bytes / 8).
   The safe `run()`/`flush()` themselves are still used; only the frame allocation moved to the
   caller.

5. **Padded frame memory.**
   `frame::Video::data(0)` returns `stride * plane_height` bytes (src/util/frame/video.rs:295-306),
   and `frame::Audio::data(0)` returns `linesize[0]` bytes (src/util/frame/audio.rs:203-214), not
   `w*h*3` / `samples*channels*4`. The probe hashes RGB24 **row by row** using `stride(0)`
   (`src/main.rs` video_seq/seek loops) and clamps audio to `samples*channels*4`, otherwise the
   digests would depend on swscale alignment padding.

6. **Backward seek semantics.**
   `Input::seek` takes `R: RangeBounds<i64>` (src/format/context/input.rs:65); the old
   `SeekRange::Backward` enum no longer exists. The probe passes `..=ts`, which maps to
   `avformat_seek_file(ctx, -1, i64::MIN, ts, ts, 0)` — a backward seek to the keyframe at/before
   `ts`. Additionally a backward seek lands on a keyframe, so the probe decodes forward until the
   first frame with `pts >= target` (`src/main.rs:344`) to compare with `shim-probe`'s
   frame-accurate `frame_at(t)`.

7. **Drain/EOF loops.**
   `receive_frame`/`send_packet` report EAGAIN and EOF through the same `Result<(), Error>`
   (src/util/error.rs has no `EAGAIN` variant; `Error::Other { errno }` is possible but the probe
   does not need to distinguish), so the drain loops simply stop on `Err` and an explicit `send_eof()`
   is required after the last packet. Same for the resampler `flush()` loop.

### 3.3 `unsafe` inventory in `src/main.rs`

Only three sites (plus the deliberately non-unsafe state):

| site | why |
|---|---|
| `graph_pad()` (`src/main.rs:553`) | `Context::as_ptr()` + `Context::wrap()` to own two filter contexts |
| `avfilter_link` calls (`src/main.rs:612-619`) | connect buffer → filter → buffersink |
| `AVPixelFormat::from(...).0` (`src/main.rs:583`) | numeric pix_fmt for the buffer source (safe conversion, no `unsafe` block, but `ffi` type) |

All other demux/decode/scale/resample/filter data flow is safe Rust.

---

## 4. Results (FFmpeg 9.1, `city_street.mp4`, 640×360 H.264 24 fps + AAC 44.1 kHz mono, 5.54 s)

Build: `cargo build --release -p third-probe` → OK, `target\release\third-probe.exe`
(1,929,352 bytes). Binary links the shared FFmpeg DLLs (`FFMPEG_DIR\lib`, DLLs from
`FFMPEG_DIR\bin` on `PATH`). JSON saved in `results/`.

| mode | invocation | key metrics |
|---|---|---|
| video-seq | `--mode video-seq --frames 100` | `frames=100`, `decode_ms=112.1`, `fps=892.0`, `bytes=69,120,000` (=100·640·360·3), `first_pts=0`, `last_pts=4,125,000` µs, `codec=h264`, 640×360 |
| video-seek | `--mode video-seek --frames 10` | `frames=10`, `misses=0`, `decode_ms=138.2`, `fps=72.4`, `duration_us=5,541,667`, `first_pts=291,666`, `last_pts=5,291,666` |
| audio | `--mode audio` | `samples=264,169`, `decode_ms=4.27`, `samples_per_sec=61,905,420`, `in 44100 Hz mono F32(Planar)` → `out 48000 Hz stereo F32(Packed)`, `resampled=true` |
| filter | `--mode filter --filter "scale=1280:720" --limit 100` | `in_frames=100`, `out_frames=100`, `filter_ms=157.9`, output 1280×720 |

Checksums are deterministic across runs (e.g. video-seq always
`12879406621699811512`, audio always `18139543067963421453`), which is the useful
self-consistency check. They are **not** byte-comparable with `shim-probe`: the shim converts to
RGBA and hashes only the first 4 KiB of each frame, while this probe follows the requested RGB24
path and hashes the full frame (row-wise, padding excluded), XOR-accumulating per-frame FNV-1a.

Timing caveat: `decode_ms` is wall clock over the whole mode; video-seq was 103-112 ms over three
runs (892-973 fps) once the file was in the OS cache — the very first run (cold) was 213 ms.

---

## 5. LOC

`crates/third-probe/src/main.rs`: **701 lines total, 650 non-blank** (`Get-Content | Measure-Object`
at the time of writing), including the long `api_note` string and doc comments.

---

## 6. Caveats / open items

* Filter mode links pad 0↔0 only; it demonstrates `scale` (1-in/1-out) but not multi-input filters
  (`overlay`) or multi-output graphs.
* libavfilter logs one warning per filter run because the `buffer` source args do not carry
  `chroma_loc`, so it compares `unspecified` vs the frame's `left`:
  `[in @ ...] Changing video frame properties on the fly is not supported by all filters.`
  Output is unaffected (in_frames == out_frames == limit, 640×360 → 1280×720).
* `--frames` defaults to 300 (video modes), `--limit` defaults to 100 (filter mode), `--mode`
  defaults to `video-seq`; audio mode decodes the whole stream and ignores `--frames`.
* `fixtures/` in `poc/media-validation` is empty, so all runs above use
  `assets/demo/city_street.mp4` (the only demo file with an audio stream besides the WAVs).
