# Build third-probe (FFmpeg 9.1 through ffmpeg-the-third) on Windows/GNU.
#
#   pwsh -File .\build-third.ps1
#
# Requires:
#   * rustup toolchain stable-x86_64-pc-windows-gnu
#   * MinGW binutils/gcc in %USERPROFILE%\.local\toolchains\mingw64\bin
#   * FFmpeg 9.1 LGPL shared dev package (include/, lib/, bin/)
#   * an x86_64 libclang (see API-NOTES-THIRD.md) + mingw sysroot C headers
$ErrorActionPreference = "Stop"

$root = $PSScriptRoot
$ffmpeg = if ($env:ARCLINE_FFMPEG_DIR) {
    $env:ARCLINE_FFMPEG_DIR
} else {
    "$env:USERPROFILE\.local\toolchains\ffmpeg-9.1-lgpl"
}
$llvm = "D:\Develop\Projects\arcline-validation\llvm"
$libclang = "$llvm\libclang"
$clangHeaders = "$libclang\lib\clang\18.1.1\include"
$mingwHeaders = "$llvm\llvm-mingw-20250910-ucrt-x86_64\include"

$env:PATH = @(
    "$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-gnu\bin",
    "$env:USERPROFILE\.local\toolchains\mingw64\bin",
    "$ffmpeg\bin",
    $libclang,
    $env:PATH
) -join ';'

$env:ARCLINE_FFMPEG_DIR = $ffmpeg
# ffmpeg-sys-the-third: prebuilt (shared) FFmpeg, no pkg-config needed.
$env:FFMPEG_DIR = $ffmpeg
# bindgen + the crate's `clang` feature probe both load libclang at build time.
$env:LIBCLANG_PATH = $libclang
# The FF_API_*/version probe parses FFmpeg headers with only -I<ffmpeg>/include,
# so the C library headers must come from CPATH (bindgen itself gets the target
# and headers from BINDGEN_EXTRA_CLANG_ARGS below).
$env:CPATH = "$clangHeaders;$mingwHeaders"
$env:BINDGEN_EXTRA_CLANG_ARGS = "--target=x86_64-w64-windows-gnu"

Push-Location $root
try {
    cargo build --release -p third-probe
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Host "built: $root\target\release\third-probe.exe"
}
finally {
    Pop-Location
}
