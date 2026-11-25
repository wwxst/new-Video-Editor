# FFmpeg 9.1 build pipeline (Arcline)

Pinned source, configure profile and build recipe for the FFmpeg build Arcline ships with
([spec](../../docs/specs/ffmpeg-9.1-upgrade.md) §5). Decisions D1–D4 are recorded in
[manifest.json](manifest.json) and in [HANDOVER.md](../../HANDOVER.md).

## Pins

| | |
|---|---|
| Version | FFmpeg 9.1 "Gauss" |
| Source | `ffmpeg-9.1.tar.xz` from <https://ffmpeg.org/releases/> |
| SHA-256 | `1a4d394ae63d190a526cd052eb4415c156b5c5e2df37efab41a7de0b8fde2433` |
| Signature | `.asc` verified against the FFmpeg release signing key `FCF986EA15E6E293A5644F10B4322F04D67658D8` (public key in [ffmpeg-release-signing-key.asc](ffmpeg-release-signing-key.asc)); the signature names commit `74e4393595cf2b5857f0da925c4e0dd74ea306e5` (tag `n9.1`, 2026-11-19) |
| Licence | LGPL version 2.1 or later (D1); no GPL, no nonfree, no version-3 libraries |
| Media Foundation | excluded (D2): no `h264_mf`/`hevc_mf` |
| Build | in-repo source build (D3), on MSYS2 UCRT64 (spec §5.2) |

## Host (MSYS2 UCRT64, user-local)

The exact package set and versions used for the reference build are in
[msys2-packages.txt](msys2-packages.txt). Toolchain: GCC 16.2.0 (MSYS2 rev7), binutils 2.47,
NASM 3.02, pkgconf 3.0.11; `mingw-w64-ucrt-x86_64-{dav1d,svt-av1,libvpl,ffnvcodec-headers,amf-headers,zlib}`.
No WSL, container or administrator rights are used.

## Build

```bash
# from the repository root, in the pinned MSYS2 UCRT64 shell:
MSYSTEM=UCRT64 "$HOME/.local/toolchains/msys64/usr/bin/bash.exe" \
    third_party/ffmpeg/build.sh
```

The script verifies the tarball hash and signature, extracts to
`~/.local/toolchains/build-ffmpeg-9.1`, configures with [configure-9.1.txt](configure-9.1.txt)
(plus `--prefix` and `--extra-version=arcline-<build id>`), runs `make`, installs to
`~/.local/toolchains/ffmpeg-9.1-lgpl` (`include/`, `lib/`, `bin/`), and writes
`ARCLINE-BUILD.txt`, `ffmpeg-buildconf.txt` and `ffmpeg-version.txt` there.

Unstripped DLL copies are kept in `<prefix>/.unstripped/` for crash symbolication and are **not**
part of the shipped package.

## Outputs

`avutil-61.dll`, `avcodec-63.dll`, `avformat-63.dll`, `swscale-10.dll`, `swresample-7.dll`,
`avfilter-12.dll`, `avdevice-63.dll`, `ffmpeg.exe`, `ffprobe.exe`, headers and GNU import
libraries. `avfilter`/`avdevice` are needed by the CLI tools only.

The package is described by [manifest.json](manifest.json) (hashes of every shipped file, build
facts, licences) and [sbom.cyclonedx.json](sbom.cyclonedx.json); licence texts and the third-party
notice set are in [licenses/](licenses/).

## Rollback

Keep the previous `~/.local/toolchains/ffmpeg-8.1-lgpl` package installed. Rollback means restoring
`ARCLINE_FFMPEG_DIR` and `scripts/dev-env.sh` to 8.1.3 and reverting the shim's version pin (spec
§11, AC-15).
