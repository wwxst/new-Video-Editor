# FFmpeg 9.1 third-party notices (Arcline)

This package is FFmpeg 9.1 "Gauss", built from the official release tarball for Arcline under
**LGPL version 2.1 or later** (decision D1). It is dynamically linked: the user may replace the
DLLs with a compatible build (see the run-time override in the spec §4.5). Source, build recipe and
configure profile are in this directory; the full SHA-256 pin and every shipped file hash are in
[manifest.json](../manifest.json), and the component list is in
[sbom.cyclonedx.json](../sbom.cyclonedx.json).

| Component | Version | Licence | Notice file |
|---|---|---|---|
| FFmpeg (libav*, ffmpeg/ffprobe) | 9.1 | LGPL-2.1-or-later | `ffmpeg-COPYING.LGPLv2.1`, `ffmpeg-LICENSE.md` |
| dav1d (AV1 decoder) | 1.5.5 | BSD-2-Clause | `dav1d-COPYING`, `dav1d-PATENTS` |
| SVT-AV1 (AV1 encoder) | 4.3.0 | BSD-2-Clause-Patent | `svt-av1-LICENSE`, `svt-av1-PATENTS.md` |
| oneVPL / libvpl (QSV dispatcher) | 2.17.0 | MIT | `libvpl-LICENSE` |
| zlib | 1.4 | Zlib | `zlib-LICENSE` |
| nv-codec-headers (NVENC headers) | 13.0.19.0 | MIT | `ffnvcodec-headers-LICENSE` |
| AMF headers (AMD encoder headers) | 1.5.3 | MIT | `amf-headers-LICENSE` |
| GCC runtime (`libgcc_s_seh-1.dll`) | GCC 16.2.0 | GPL-3.0-or-later WITH GCC-exception-3.1 | `gcc-COPYING.RUNTIME`, `gcc-COPYING3` |
| libstdc++ (`libstdc++-6.dll`) | GCC 16.2.0 | GPL-3.0-or-later WITH GCC-exception-3.1 | `gcc-COPYING.RUNTIME`, `gcc-COPYING3` |
| winpthreads (`libwinpthread-1.dll`) | 16.2.0 | MIT | `libwinpthread-COPYING` |
| MinGW-w64 runtime notices | 14.0.0 | public domain / BSD-style | `mingw-w64-COPYING.MinGW-w64-runtime.txt`, `mingw-w64-COPYING.MinGW-w64.txt` |

No GPL or nonfree components are enabled (`--disable-gpl --disable-nonfree`, no
`--enable-version3`); `ffmpeg -L` reports the LGPL. `libopenh264`, `libcurl` and every network
protocol are excluded; the only protocols compiled in are `file` and `pipe`. Media Foundation is
excluded (D2), so `h264_mf`/`hevc_mf` do not exist.

FFmpeg's `LICENSE.md` also requires crediting the **Independent JPEG Group** in the documentation
of distributed executables that include its JPEG code; Arcline's about/legal screen and release
notes carry that credit.

The GCC runtime DLLs are shipped under the GCC Runtime Library Exception; the MinGW-w64 CRT headers'
notices are included for completeness even though only system UCRT DLLs are used at run time.
