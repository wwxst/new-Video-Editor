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
