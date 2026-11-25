# Source this file before building:  . scripts/dev-env.sh
#
# 1. Puts the Rust toolchain on PATH when rustup's proxies (~/.cargo/bin) are absent.
# 2. On Windows GNU targets, puts a MinGW-w64 toolchain on PATH. `windows-sys` >= 0.60 links with
#    raw-dylib, which needs `dlltool.exe`; MinGW also provides the C compiler some crates need.
#    Default location: ~/.local/toolchains/mingw64 (WinLibs GCC 16.2.0 UCRT, see HANDOVER.md).
#    Override with ARCLINE_MINGW=/path/to/mingw64.

if ! command -v cargo >/dev/null 2>&1; then
  for d in "$HOME/.cargo/bin" \
           "$HOME/.rustup/toolchains/stable-x86_64-pc-windows-msvc/bin" \
           "$HOME/.rustup/toolchains/stable-x86_64-pc-windows-gnu/bin"; do
    if [ -x "$d/cargo" ] || [ -x "$d/cargo.exe" ]; then
      export PATH="$d:$PATH"
      break
    fi
  done
fi
command -v cargo >/dev/null 2>&1 || echo "dev-env: cargo not found; install Rust via rustup" >&2

case "$(rustc -vV 2>/dev/null | sed -n 's/^host: //p')" in
  *windows-gnu*)
    MINGW="${ARCLINE_MINGW:-$HOME/.local/toolchains/mingw64}"
    if [ -x "$MINGW/bin/dlltool.exe" ]; then
      export PATH="$MINGW/bin:$PATH"
    elif ! command -v dlltool >/dev/null 2>&1; then
      echo "dev-env: dlltool not found; install MinGW-w64 (see HANDOVER.md)" >&2
    fi
    ;;
esac

# 3. FFmpeg (LGPL, shared) for arc-media: headers + import libs for the libav shim, and its
#    LGPL ffmpeg/ffprobe for subprocess jobs. The pinned 9.1 package is preferred; 8.1.3 stays
#    installed for rollback (spec §5.5/§11). ARCLINE_FFMPEG_DIR overrides the choice.
if [ -z "$ARCLINE_FFMPEG_DIR" ]; then
  for d in "$HOME/.local/toolchains/ffmpeg-9.1-lgpl" \
           "$HOME/.local/toolchains/ffmpeg-8.1-lgpl"; do
    if [ -d "$d" ]; then
      export ARCLINE_FFMPEG_DIR="$d"
      break
    fi
  done
fi
if [ -n "$ARCLINE_FFMPEG_DIR" ]; then
  export PATH="$ARCLINE_FFMPEG_DIR/bin:$PATH"
  echo "dev-env: FFmpeg = $(ffmpeg -version 2>/dev/null | head -1)" >&2
else
  echo "dev-env: ARCLINE_FFMPEG_DIR not set; arc-media will not build (see HANDOVER.md)" >&2
fi
