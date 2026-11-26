#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Plays the game through VIctory under Proton on Linux with an AMD card (RADV), the way it was
# measured on an RX 9070 XT (docs/running-gta6.md, "On Linux"). It sets what must be set on the Linux
# side, then runs the Windows launcher, `anyxboxx run --profile linux-amd`, inside Proton:
#   - LD_PRELOAD tools/linux/anonless (built here once, into ~/.cache/anyxboxx): the card then reads
#     the title's memory in place, and VICTORY_PORT_CONSTANTS engages;
#   - VK_DRIVER_FILES: the RADV built by tools/linux/mesa/build-radv.sh (mouths right; game only);
#   - VKD3D_CONFIG=descriptor_heap and d3dcompiler_47 native (Microsoft's own DLL, your copy).
#
# usage: GAME_DIR=<game folder> VICTORY=<victory.exe> ANYXBOXX=<anyxboxx.exe> \
#        PROTON=<Proton's files folder> WINEPREFIX=<prefix>/pfx tools/linux/play.sh [anyxboxx run options]
#   GAME_DIR      the folder holding game_scarlett_master.exe (your own copy)
#   VICTORY       victory.exe cross-built from third_party/victory (docs/running-gta6.md), with
#                 Microsoft's d3dcompiler_47.dll beside it (never in this repository)
#   ANYXBOXX      anyxboxx.exe cross-built from this repository's root (cargo build --release -p anyxboxx)
#   PROTON        e.g. ~/.local/share/Steam/compatibilitytools.d/GE-Proton11-12/files (has bin/wine)
#   WINEPREFIX    the prefix (STEAM_COMPAT_DATA_PATH too, if your Proton wants it)
#   MESA=system   the system's RADV instead of tools/linux/mesa's (mouths deform; for comparing)
#   MESA_ICD      the patched RADV's ICD file (default: build-radv.sh's install)
#   ANONLESS=0    without the shim (VICTORY_PORT_CONSTANTS then turns itself off)
# Options after the variables go to `anyxboxx run` (--warm, --diag, --auto-a, --override K=V...).
# (No VICTORY_ prefix on this script's own variables: VIctory reads VICTORY_*, and its shader cache
# key depends on them.)
# The 32-bit parts of Proton print "anonless.so ... wrong ELF class": harmless, the game is 64-bit.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
die() { echo "play.sh: $*" >&2; exit 1; }
winpath() { local p; p=$(realpath "$1"); echo "Z:${p//\//\\}"; }

for name in GAME_DIR VICTORY ANYXBOXX PROTON WINEPREFIX; do
  [ -n "${!name:-}" ] || die "set $name (see the header of this script)"
done
[ -f "$GAME_DIR/game_scarlett_master.exe" ] || die "no game_scarlett_master.exe in $GAME_DIR"
[ -f "$VICTORY" ] || die "no $VICTORY"
[ -f "$ANYXBOXX" ] || die "no $ANYXBOXX"
[ -x "$PROTON/bin/wine" ] || die "no $PROTON/bin/wine"
# Wine's own d3dcompiler cannot compile VIctory's helper shaders (black world): Microsoft's DLL,
# from your Windows or Windows SDK install (or `winetricks d3dcompiler_47`), goes beside victory.exe.
[ -f "$(dirname "$VICTORY")/d3dcompiler_47.dll" ] \
  || die "put Microsoft's d3dcompiler_47.dll beside $VICTORY (your own copy; never commit it)"

if [ "${ANONLESS:-1}" != 0 ]; then
  SHIM=${XDG_CACHE_HOME:-$HOME/.cache}/anyxboxx/anonless.so
  if [ ! -f "$SHIM" ] || [ "$HERE/anonless/anonless.c" -nt "$SHIM" ]; then
    mkdir -p "$(dirname "$SHIM")"
    # shellcheck disable=SC2046  # pkg-config prints several flags
    gcc -O2 -shared -fPIC -o "$SHIM" "$HERE/anonless/anonless.c" -ldl $(pkg-config --cflags libdrm) \
      || die "building the anonless shim failed (needs gcc and libdrm-devel)"
  fi
  export LD_PRELOAD="$SHIM${LD_PRELOAD:+:$LD_PRELOAD}"
fi

if [ "${MESA:-local}" != system ]; then
  ICD=${MESA_ICD:-${XDG_DATA_HOME:-$HOME/.local/share}/anyxboxx/mesa/share/vulkan/icd.d/radeon_icd.x86_64.json}
  [ -f "$ICD" ] || die "no patched RADV at $ICD: run tools/linux/mesa/build-radv.sh (or MESA=system)"
  export VK_DRIVER_FILES=$ICD
fi

export VKD3D_CONFIG=${VKD3D_CONFIG:-descriptor_heap}
# Proton's `wine` run directly (not its `proton` script) prefers Wine's own d3d12/dxgi: the prefix's
# vkd3d-proton and DXVK must be asked for, or VIctory gets no host D3D12 device (white window).
export WINEDLLOVERRIDES="d3d12,d3d12core,dxgi,d3d11,d3d10core,d3dcompiler_47=n,b${WINEDLLOVERRIDES:+;$WINEDLLOVERRIDES}"
echo "play.sh: shim=${ANONLESS:-1} driver=${VK_DRIVER_FILES:-system} VKD3D_CONFIG=$VKD3D_CONFIG" >&2
victory=$(winpath "$VICTORY") game=$(winpath "$GAME_DIR") anyxboxx=$ANYXBOXX
unset VICTORY ANYXBOXX GAME_DIR
exec "$PROTON/bin/wine" "$anyxboxx" run --profile linux-amd --victory-exe "$victory" --game-dir "$game" "$@"
