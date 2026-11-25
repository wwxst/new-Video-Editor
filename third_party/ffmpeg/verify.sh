#!/usr/bin/env bash
# Verify the built FFmpeg 9.1 package against spec §6 (AC-4, AC-13): licence, configure profile,
# protocols, demuxers, encoders, security flags and the bundled runtime DLLs.
#
#   third_party/ffmpeg/verify.sh [package-dir]
set -uo pipefail

PKG=${1:-"$HOME/.local/toolchains/ffmpeg-9.1-lgpl"}
HERE=$(cd "$(dirname "$0")" && pwd)
BIN="$PKG/bin"
FF="$BIN/ffmpeg.exe"
FAIL=0
ok() { echo "ok   $*"; }
bad() { echo "FAIL $*"; FAIL=1; }

echo "== package: $PKG"
[ -x "$FF" ] || { echo "FAIL $FF missing"; exit 1; }

ver=$("$FF" -hide_banner -version | head -1)
echo "version: $ver"
case "$ver" in
    "ffmpeg version 9.1-arcline-"*) ok "ident is the pinned 9.1 build" ;;
    *) bad "unexpected version line: $ver" ;;
esac

if "$FF" -hide_banner -L | grep -qiE 'lesser general public license|lgpl version'; then
    ok "licence text is LGPL"
else
    bad "-L does not state the LGPL"
fi
if "$FF" -hide_banner -L | grep -qiE 'nonfree'; then bad "nonfree licence text"; else ok "no nonfree"; fi

if "$FF" -hide_banner -buildconf | grep -qE 'enable-gpl|enable-nonfree|enable-version3'; then
    bad "buildconf enables GPL/nonfree/version3"
else
    ok "buildconf: no GPL, nonfree or version3"
fi
if "$FF" -hide_banner -buildconf | grep -qE 'disable-network|disable-protocols'; then
    ok "buildconf: network/protocols disabled"
else
    bad "buildconf does not disable network/protocols"
fi

protos=$("$FF" -hide_banner -protocols | awk '/^  [a-z]/{print $1}' | sort -u | tr '\n' ' ')
echo "protocols: $protos"
if [ "$protos" = "file pipe " ]; then ok "only file and pipe"; else bad "unexpected protocols: $protos"; fi

for d in hls dash concat sdp rtsp rtp; do
    if "$FF" -hide_banner -demuxers | awk '{print $2}' | grep -qx "$d"; then bad "demuxer $d present"; else ok "demuxer $d absent"; fi
done
if "$FF" -hide_banner -encoders | grep -qE 'libopenh264|h264_mf|hevc_mf'; then
    bad "forbidden encoder present"
else
    ok "no libopenh264 / h264_mf / hevc_mf"
fi

echo "== external runtime DLLs"
for dll in libdav1d-7.dll libSvtAv1Enc-4.dll libvpl-2.dll libz-1.dll libgcc_s_seh-1.dll libwinpthread-1.dll libstdc++-6.dll; do
    [ -f "$BIN/$dll" ] && ok "bundled $dll" || bad "missing $dll"
done

echo "== PE security flags (DYNAMIC_BASE 0x40, HIGH_ENTROPY_VA 0x20, NX_COMPAT 0x100)"
OBJDUMP=${OBJDUMP:-/ucrt64/bin/objdump.exe}
for f in "$BIN"/*.dll "$BIN"/*.exe; do
    flags=$("$OBJDUMP" -p "$f" 2>/dev/null | awk '/DllCharacteristics/{print $2}')
    if [ -z "$flags" ]; then bad "$(basename "$f"): no DllCharacteristics"; continue; fi
    v=$((16#$flags))
    if (( (v & 0x40) && (v & 0x20) && (v & 0x100) )); then
        ok "$(basename "$f"): DllCharacteristics=0x$flags"
    else
        bad "$(basename "$f"): DllCharacteristics=0x$flags lacks DYNAMIC_BASE/HIGH_ENTROPY_VA/NX_COMPAT"
    fi
done

echo "== manifest / SBOM / licences"
for f in manifest.json sbom.cyclonedx.json licenses/NOTICE.md licenses/ffmpeg-COPYING.LGPLv2.1; do
    [ -f "$HERE/$f" ] && ok "$f" || bad "missing $f"
done

exit "$FAIL"
