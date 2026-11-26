#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Builds RADV (Mesa's Vulkan driver for AMD) 26.2.7 with nir-lower-memory-model-image-intrinsics.patch,
# into a folder of your own, for the game only (tools/linux/play.sh loads it with VK_DRIVER_FILES).
# Your system's Mesa is not touched; nothing needs root. RADV only: no OpenGL, no LLVM, 64-bit.
#
# The fix: with vkd3d-proton's descriptor heaps (VKD3D_CONFIG=descriptor_heap), RADV lowers image
# accesses to image_heap_* intrinsics before nir_lower_memory_model runs, and that pass did not know
# them: MakeTexelVisible/Available were dropped without the loads being made coherent, so a wave on
# another compute unit read stale data (GTA VI's face programs: deformed mouths). Measured: a
# synthetic test went from 70,144 stale reads to 0, and the mouths are right in game (RX 9070 XT).
# Mesa main has the same code at the time of writing; the fix is not upstream yet.
#
# usage: tools/linux/mesa/build-radv.sh
#   MESA_PREFIX  where it is installed (default ~/.local/share/anyxboxx/mesa)
#   MESA_WORK    source and build folders (default ~/.cache/anyxboxx/mesa-build)
# Needs: git, meson, ninja, a C/C++ compiler, bison, flex, glslangValidator, python3 with mako and
# pyyaml, and the headers of libdrm, expat, xshmfence, wayland, xcb, libelf, zstd, libudev.
# On a Fedora 44 desktop that already had meson and glslang, three packages were missing:
#   sudo dnf install python3-mako libxshmfence-devel expat-devel
# On a bare system, `sudo dnf builddep mesa` installs everything.
set -euo pipefail

VERSION=26.2.7
HERE=$(cd "$(dirname "$0")" && pwd)
PATCH=$HERE/nir-lower-memory-model-image-intrinsics.patch
PREFIX=${MESA_PREFIX:-${XDG_DATA_HOME:-$HOME/.local/share}/anyxboxx/mesa}
WORK=${MESA_WORK:-${XDG_CACHE_HOME:-$HOME/.cache}/anyxboxx/mesa-build}
SRC=$WORK/mesa-$VERSION

missing=()
for tool in git meson ninja cc bison flex glslangValidator python3 pkg-config; do
  command -v "$tool" >/dev/null || missing+=("$tool")
done
python3 -c 'import mako, yaml' 2>/dev/null || missing+=("python3-mako/python3-pyyaml")
for pc in libdrm_amdgpu expat xshmfence wayland-client libelf; do
  pkg-config --exists "$pc" 2>/dev/null || missing+=("$pc (headers)")
done
if [ ${#missing[@]} -gt 0 ]; then
  echo "build-radv.sh: missing: ${missing[*]}" >&2
  echo "  Fedora: sudo dnf install python3-mako libxshmfence-devel expat-devel  (or: sudo dnf builddep mesa)" >&2
  exit 1
fi

mkdir -p "$WORK"
if [ ! -d "$SRC/.git" ]; then
  git clone --depth 1 --branch "mesa-$VERSION" https://gitlab.freedesktop.org/mesa/mesa.git "$SRC"
fi
cd "$SRC"
if git apply --reverse --check "$PATCH" 2>/dev/null; then
  echo "build-radv.sh: the fix is already applied"
else
  git apply "$PATCH"
fi
# The version Vulkan reports (vulkaninfo, VIctory's log) says which driver ran.
echo "$VERSION-anyxboxx" > VERSION

if [ ! -f "$WORK/build/build.ninja" ]; then
  meson setup "$WORK/build" --prefix "$PREFIX" --buildtype=release \
    -Dvulkan-drivers=amd -Dgallium-drivers= -Dplatforms=x11,wayland -Dllvm=disabled \
    -Dopengl=false -Dgles1=disabled -Dgles2=disabled -Degl=disabled -Dglx=disabled -Dgbm=disabled \
    -Dvalgrind=disabled -Dbuild-tests=false -Dvideo-codecs= -Dvulkan-layers= -Dtools= -Dhtml-docs=disabled
fi
meson compile -C "$WORK/build"
meson install -C "$WORK/build" --quiet

ICD=$(ls "$PREFIX"/share/vulkan/icd.d/radeon_icd.*.json)
echo "build-radv.sh: done. tools/linux/play.sh finds it there; by hand: VK_DRIVER_FILES=$ICD"
