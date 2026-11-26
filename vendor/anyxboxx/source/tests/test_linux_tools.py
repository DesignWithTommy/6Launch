"""tools/linux: what Linux (Proton) + AMD needs around `anyxboxx run --profile linux-amd`.

play.sh is run against a fake Proton whose `wine` logs its arguments and the environment it got;
build-radv.sh and the Mesa patch are checked as text (building Mesa takes minutes and the network);
the anonless shim is compiled when a C compiler and libdrm's headers are there. Linux only.
"""
import os
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
LINUX = ROOT / "tools" / "linux"
PLAY = LINUX / "play.sh"
BUILD_RADV = LINUX / "mesa" / "build-radv.sh"
PATCH = LINUX / "mesa" / "nir-lower-memory-model-image-intrinsics.patch"
SHIM = LINUX / "anonless" / "anonless.c"

needs_bash = pytest.mark.skipif(sys.platform == "win32" or not shutil.which("bash"), reason="Linux scripts")

FAKE_WINE = """#!/bin/bash
{ printf 'ARGS'; printf ' [%s]' "$@"; echo
  for k in LD_PRELOAD VK_DRIVER_FILES VKD3D_CONFIG WINEDLLOVERRIDES VICTORY GAME_DIR ANYXBOXX; do
    echo "$k=${!k-<unset>}"; done; } > "$FAKE_LOG"
"""


def test_no_binaries_in_tools_linux():
    # Rule 1: the shim and the driver are built on the user's machine; d3dcompiler_47 is the user's copy.
    built = [p for p in LINUX.rglob("*") if p.suffix.lower() in (".so", ".dll", ".exe", ".json")]
    assert built == []


def test_the_mesa_patch_touches_only_the_memory_model_pass():
    text = PATCH.read_text(encoding="utf-8")
    changed = [line.split()[-1] for line in text.splitlines() if line.startswith("+++ ")]
    assert changed == ["b/src/compiler/nir/nir_lower_memory_model.c"]
    for intrinsic in ("nir_intrinsic_image_heap_load", "nir_intrinsic_image_heap_store",
                      "nir_intrinsic_bindless_image_load", "nir_intrinsic_image_atomic"):
        assert f"+   case {intrinsic}:" in text, intrinsic
    assert not any(line.startswith("-") and not line.startswith("---") for line in text.splitlines())


def test_build_radv_pins_the_release_and_installs_outside_the_system():
    text = BUILD_RADV.read_text(encoding="utf-8")
    assert "VERSION=26.2.7" in text and '--branch "mesa-$VERSION"' in text
    assert "-Dvulkan-drivers=amd" in text and "-Dllvm=disabled" in text
    assert "sudo" not in text.replace("sudo dnf", "")   # only the dnf hint names sudo
    assert "python3-mako libxshmfence-devel expat-devel" in text


@needs_bash
def test_scripts_parse():
    for script in (PLAY, BUILD_RADV):
        subprocess.run(["bash", "-n", str(script)], check=True)


def _play(tmp_path, *, dll=True, env_extra=None, args=()):
    game, bin_dir, proton = tmp_path / "game", tmp_path / "bin", tmp_path / "proton" / "bin"
    for folder in (game, bin_dir, proton):
        folder.mkdir(parents=True)
    (game / "game_scarlett_master.exe").write_bytes(b"")
    (bin_dir / "victory.exe").write_bytes(b"")
    (bin_dir / "anyxboxx.exe").write_bytes(b"")
    if dll:
        (bin_dir / "d3dcompiler_47.dll").write_bytes(b"")
    wine = proton / "wine"
    wine.write_text(FAKE_WINE, encoding="utf-8")
    wine.chmod(0o755)
    icd = tmp_path / "radeon_icd.x86_64.json"
    icd.write_text("{}", encoding="utf-8")
    log = tmp_path / "wine.log"
    env = {"PATH": os.environ["PATH"], "HOME": str(tmp_path), "FAKE_LOG": str(log),
           "GAME_DIR": str(game), "VICTORY": str(bin_dir / "victory.exe"), "ANYXBOXX": str(bin_dir / "anyxboxx.exe"),
           "PROTON": str(tmp_path / "proton"), "WINEPREFIX": str(tmp_path / "pfx"), "MESA_ICD": str(icd),
           "ANONLESS": "0", **(env_extra or {})}
    done = subprocess.run(["bash", str(PLAY), *args], env=env, capture_output=True, text=True, timeout=60)
    lines = log.read_text(encoding="utf-8").splitlines() if log.is_file() else []
    return done, lines, icd


@needs_bash
def test_play_runs_the_linux_profile_with_the_patched_driver(tmp_path):
    done, lines, icd = _play(tmp_path, args=("--auto-a",))
    assert done.returncode == 0, done.stderr
    call = lines[0]
    assert "[run] [--profile] [linux-amd] [--victory-exe] [Z:" in call and call.endswith("[--auto-a]")
    assert "game_scarlett_master" not in call   # the game folder, not the game, is passed
    assert f"VK_DRIVER_FILES={icd}" in lines
    assert "VKD3D_CONFIG=descriptor_heap" in lines
    assert any(line.startswith("WINEDLLOVERRIDES=d3d12,d3d12core,dxgi,d3d11,d3d10core,d3dcompiler_47=n,b") for line in lines)
    # The script's own variables do not reach VIctory (no VICTORY_ prefix, and unset before Wine).
    assert {"VICTORY=<unset>", "GAME_DIR=<unset>", "ANYXBOXX=<unset>"} <= set(lines)
    assert "LD_PRELOAD=<unset>" in lines   # ANONLESS=0


@needs_bash
def test_play_needs_microsofts_d3dcompiler_beside_victory(tmp_path):
    done, lines, _ = _play(tmp_path, dll=False)
    assert done.returncode != 0 and lines == []
    assert "d3dcompiler_47.dll" in done.stderr


@needs_bash
def test_play_refuses_without_the_patched_driver_unless_asked_for_the_system_one(tmp_path):
    missing = str(tmp_path / "nowhere.json")
    done, lines, _ = _play(tmp_path, env_extra={"MESA_ICD": missing})
    assert done.returncode != 0 and "build-radv.sh" in done.stderr and lines == []
    done, lines, _ = _play(tmp_path / "system", env_extra={"MESA_ICD": missing, "MESA": "system"})
    assert done.returncode == 0 and "VK_DRIVER_FILES=<unset>" in lines, done.stderr


@needs_bash
def test_play_builds_and_preloads_the_shim(tmp_path):
    if not shutil.which("gcc") or subprocess.run(["pkg-config", "--exists", "libdrm"]).returncode != 0:
        pytest.skip("needs gcc and libdrm's headers")
    done, lines, _ = _play(tmp_path, env_extra={"ANONLESS": "1", "XDG_CACHE_HOME": str(tmp_path / "cache")})
    shim = tmp_path / "cache" / "anyxboxx" / "anonless.so"
    assert done.returncode == 0, done.stderr
    assert shim.is_file() and f"LD_PRELOAD={shim}" in lines
    symbols = subprocess.run(["nm", "-D", "--defined-only", str(shim)], capture_output=True, text=True).stdout
    assert "drmCommandWriteRead" in symbols
