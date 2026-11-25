#!/usr/bin/env bash
# Build the pinned FFmpeg 9.1 "Gauss" package for Arcline (spec §5).
#
# Run inside the pinned MSYS2 UCRT64 environment:
#   MSYSTEM=UCRT64 /c/Users/<you>/.local/toolchains/msys64/usr/bin/bash.exe \
#       third_party/ffmpeg/build.sh
#
# Inputs (override with the environment variables in braces):
#   {ARCLINE_FFMPEG_TARBALL}  official source tarball      (default: ~/.local/toolchains/dl-ffmpeg/ffmpeg-9.1.tar.xz)
#   {ARCLINE_FFMPEG_SIG}      detached signature           (default: <tarball>.asc)
#   {ARCLINE_FFMPEG_BUILD}    scratch build directory      (default: ~/.local/toolchains/build-ffmpeg-9.1)
#   {ARCLINE_FFMPEG_STAGE}    install prefix / dev package (default: ~/.local/toolchains/ffmpeg-9.1-lgpl)
#
# The recipe verifies the source signature with the release key committed next to this file and the
# pinned SHA-256, then configures with configure-9.1.txt, builds, installs, and writes the build
# facts used by manifest.json (configure line, `ffmpeg -buildconf`, tool versions).
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
TARBALL=${ARCLINE_FFMPEG_TARBALL:-"$HOME/.local/toolchains/dl-ffmpeg/ffmpeg-9.1.tar.xz"}
SIG=${ARCLINE_FFMPEG_SIG:-"$TARBALL.asc"}
BUILD=${ARCLINE_FFMPEG_BUILD:-"$HOME/.local/toolchains/build-ffmpeg-9.1"}
STAGE=${ARCLINE_FFMPEG_STAGE:-"$HOME/.local/toolchains/ffmpeg-9.1-lgpl"}
PINNED_SHA256=1a4d394ae63d190a526cd052eb4415c156b5c5e2df37efab41a7de0b8fde2433
KEY="$HERE/ffmpeg-release-signing-key.asc"
PROFILE="$HERE/configure-9.1.txt"

# The MSYS profile is not sourced when a script is invoked directly; put the UCRT64 toolchain on
# PATH ourselves so the recipe works from any MSYS2 shell.
if [ -d /ucrt64/bin ]; then
    PATH="/ucrt64/bin:$PATH"
    export PATH
fi

command -v gcc >/dev/null || {
    echo "build.sh: run inside MSYS2 (UCRT64 gcc not found)" >&2
    exit 1
}

echo "== verifying source"
actual=$(sha256sum "$TARBALL" | cut -d' ' -f1)
[ "$actual" = "$PINNED_SHA256" ] || {
    echo "build.sh: tarball SHA-256 $actual != pinned $PINNED_SHA256" >&2
    exit 1
}
GNUPGHOME="$BUILD/gnupg"
export GNUPGHOME
rm -rf "$GNUPGHOME"
mkdir -p "$GNUPGHOME"
chmod 700 "$GNUPGHOME"
gpg --batch --import "$KEY" >/dev/null 2>&1
gpg --batch --verify "$SIG" "$TARBALL"

# Build id: source + profile + pinned host packages, so rebuilds with a different host are traceable.
src_sha=$(sha256sum "$TARBALL" | cut -d' ' -f1)
prof_sha=$(sha256sum "$PROFILE" | cut -d' ' -f1)
pkgs_sha=$(sha256sum "$HERE/msys2-packages.txt" | cut -d' ' -f1)
BUILD_ID=$(printf '%s\n%s\n%s\n' "$src_sha" "$prof_sha" "$pkgs_sha" | sha256sum | cut -c1-10)
echo "== build id arcline-$BUILD_ID"

echo "== extracting"
rm -rf "$BUILD/src"
mkdir -p "$BUILD/src"
tar -xJf "$TARBALL" -C "$BUILD/src"
SRC="$BUILD/src/ffmpeg-9.1"

ARGS=()
while IFS= read -r line; do
    case "$line" in
        ''|'#'*) continue ;;
    esac
    ARGS+=("$line")
done < "$PROFILE"

echo "== configure"
(
    cd "$SRC"
    ./configure --prefix="$STAGE" \
        --extra-version="arcline-$BUILD_ID" \
        "${ARGS[@]}"
)

echo "== build"
make -C "$SRC" -j"$(nproc)"

echo "== install"
rm -rf "$STAGE"
make -C "$SRC" install

# This profile links dav1d, SVT-AV1, oneVPL, zlib and the MinGW runtime dynamically; ship their
# runtime DLLs next to the FFmpeg ones so the package is self-contained (manifest files[] covers
# them). The licences are in ../licenses/.
echo "== bundling external runtime DLLs"
for dll in libdav1d-7.dll libSvtAv1Enc-4.dll libvpl-2.dll libz-1.dll \
           libgcc_s_seh-1.dll libwinpthread-1.dll libstdc++-6.dll; do
    cp -f "/ucrt64/bin/$dll" "$STAGE/bin/$dll"
done

# Keep an unstripped copy of the DLLs for crash symbolication (spec §5.4); never shipped.
mkdir -p "$STAGE/.unstripped"
cp "$STAGE"/bin/*.dll "$STAGE/.unstripped/" 2>/dev/null || true

"$STAGE/bin/ffmpeg.exe" -hide_banner -buildconf >"$STAGE/ffmpeg-buildconf.txt"
"$STAGE/bin/ffmpeg.exe" -hide_banner -version >"$STAGE/ffmpeg-version.txt"

{
    echo "build_id=arcline-$BUILD_ID"
    echo "source_sha256=$src_sha"
    echo "profile_sha256=$prof_sha"
    echo "packages_sha256=$pkgs_sha"
    echo "built_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "host=$(uname -s -r)"
    echo "gcc=$(gcc --version | head -1)"
    echo "binutils=$(ld --version | head -1)"
    echo "nasm=$(nasm -v)"
    echo "pkgconf=$(pkg-config --version)"
} >"$STAGE/ARCLINE-BUILD.txt"

echo "== done: $STAGE"
cat "$STAGE/ffmpeg-version.txt"
