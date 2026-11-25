# Neonbridge for developers

Everything a player doesn't need: building from source, the command line, the runtime trees and their settings. For playing, see the [README](README.md).

## Building from source

```
cargo build --release                      # target\release\neonbridge.exe
neonbridge                                 # the picker window (or, in a terminal, numbered choices)
neonbridge --runtime victory --game <dump dir> [--log run.log] [--timeout S]
```

You need Visual Studio Build Tools with the x64 C++ tools, the Windows SDK and Rust (stable, `x86_64-pc-windows-msvc`). Each runtime's own toolchain is pinned in its tree (`rust-toolchain.toml`), and rustup fetches it.

`neonbridge --package` builds every runtime and lays out a release. A runtime that can't be built here (a missing prerequisite or a failed build) is left out and listed under "Not bundled" in the package's README.

## Runtimes

Each runtime is a tree under `vendor/<name>`, copied unchanged from its upstream at the hash in its `UPSTREAM` file. Its `NEONBRIDGE.toml` tells the launcher how to build it, which exe and arguments to run, its environment, options, presets and setup steps. A runtime's own fixes, if any, are in `vendor/patches/rt-<name>.patch`, applied for the build and reverted after it.

In picker order: each toml's `rank = N` (best first, by the leader's own published results; unranked trees last, Mix always last).

| Rank | `--runtime` | What it is | Upstream |
|---|---|---|---|
| 1 | `klbf` | KLBF Lab, a VIctory/victory-fork build (AMD-tested; `--preset fsr` = its Series S + AMD FSR setting, needs `VICTORY_FFX_DIR`) | PastiiJay/klbf-lab |
| 2 | `flamingo` | Flamingo, a VIctory fork | realblastex/flamingo |
| 3 | `victory` | VIctory-fxceless, the victory emulator fork | fxceless/victory-fork |
| 4 | `vitoriabr` | VItoriaBr's VIctory fork | uiliannn/VItoriaBr |
| 5 | `xcl` | xcl | ivall/xcl |
| 6 | `belobug` | VIctory-Belobug-AMD, a VIctory fork tuned for AMD | Belobug/VIctory-Belobug-AMD |
| 7 | `scarlettport` (default for `--runtime`) | ScarlettPort's VIctory-based runtime and launcher scripts | ZigZagBoi1/ScarlettPort |
| - | `neonbridge-own` | Neonbridge (own build), hidden from the picker: TuxBox console-library providers staged into the game folder, with Neonbridge's fixes and shader translator | brcly/TuxBox |

Each runtime's default launch is exactly its author's (flags, environment, preset, cited in its toml); anything of Neonbridge's is an optional `preset_*`.

The picker selects the last pick, else `NEONBRIDGE_RUNTIME`, else rank 1.

The loader reads each step's own counts (`src/progress.rs`, one parser test per runtime): cargo's `Building [..] N/M` (asked for with `CARGO_TERM_PROGRESS_WHEN=always`), victory's `warm-up N shaders so far`, xcl's `N/M (t s)`, TuxBox's `label · N/M ready`, ScarlettPort's precompile-graphics records over the last full pass (`graphics-total.txt`), and Five Star's `[i/n] step` and `N/M (p%)` lines. A step with no count shows a pulse, not a percentage.

A build is skipped when its build stamp (`<exe>.nbstamp`: the tree's git hash, its uncommitted changes, its patch and its build line) still matches.

## Commands

| Command | Does |
|---|---|
| `neonbridge` | No arguments: the picker. In a terminal with `NEONBRIDGE_PICKER=0`, nothing is asked. |
| `--runtime <name>` | The runtime to use (or `NEONBRIDGE_RUNTIME`; default `scarlettport`). |
| `--list` | The runtimes. |
| `--list <name>` | That runtime's options, their defaults and the values a run would use, plus its presets and setup steps. |
| `--set NAME=VALUE` | One option for this run: an environment variable, or `--set --flag=VALUE` for the runtime's own arguments. Repeatable. |
| `--preset <name>` | A preset from the runtime's `NEONBRIDGE.toml`, applied before `--set`. |
| `--mix <name>`, `--mix-part <part>=<runtime>` | Build one victory exe from crates of several trees. |
| `--setup <name>` | The runtime's setup steps. A step that downloads something or needs a file of yours asks first and runs only on a typed `yes`. |
| `--game <dir>`, `--exe <file>` | The dump folder, and another title exe in it. |
| `--log <file>`, `--timeout <s>` | Where the run's log goes (default `%LOCALAPPDATA%\6ixpack\logs\<runtime>_<time>.log`), and a time limit. |
| `--build-only`, `--prewarm` | Build the runtime only, or run its shader prewarm. |

Settings apply in this order, the later winning: the toml, your environment, the preset, then `--set`.

The `neonbridge-own` runtime keeps its own options (`--debug`, `--rev`, `--tuxbox`, `--pipelines`, `neonbridge ctl ...`); `neonbridge --runtime neonbridge-own --help` lists them.

## Environment variables

| Variable | Does |
|---|---|
| `NEONBRIDGE_LOG=0` | No run log (logging is on by default for every runtime; children get `RUST_LOG=info`, the victory family also `VICTORY_LOG=info`). |
| `NEONBRIDGE_DIAG=0` | No diagnostics build. On by default: each toml's `diag_build`/`diag_exe` (the author's own diagnostics build) is used, with its own build stamp. The picker's Diagnostics box sets it. |
| `NEONBRIDGE_FEED=1` | Stream the run over Tailscale (`tools/tsproxy.exe`, else `tools/cloudflared.exe`) behind a site password; needs ffmpeg. Shows the loader and the game window. |
| `NEONBRIDGE_FEED_WINDOWS=0` | Stream only neonbridge-own's frame tap, not window captures. |
| `NEONBRIDGE_AUTO_A=0` | Turns off auto-press A: by default the launcher focuses the game window and presses Enter (A) every 8 s for the whole run, alternating a tap (start screens) and a 6 s hold (cutscene skip), so unattended runs don't stall (`NEONBRIDGE_AUTO_A_EVERY`, `_FOR` in seconds, `_HOLD_MS`, `_LONG_MS`) |
| `NEONBRIDGE_CACHE_BACKUP=0` | Turns off the cache backup: at each launch and after each shader warm, the runtime's shader and pipeline cache is mirrored (new files only, never deleted) into `%LOCALAPPDATA%\6ixpack\cache-backup` at idle priority; `=<folder>` backs up there instead |
| `NEONBRIDGE_DISPLAY=borderless` `NEONBRIDGE_RESOLUTION=1920x1080` `NEONBRIDGE_FPS_CAP=30` `NEONBRIDGE_VSYNC=0` `NEONBRIDGE_MONITOR=1` | Display settings (ScarlettPort's display.rs, cloned): set as `XCL_WINDOW_*`/`XCL_FPS_CAP`/`XCL_VSYNC`/`XCL_MONITOR`, `VICTORY_DISPLAY_MODE`, `VICTORY_PACE_FPS` and `XCL_PRESENT_SYNC`; each runtime reads its own. Unset = author defaults |
| `NEONBRIDGE_BUILD=0` | Never build; use the exe as it is. |
| `NEONBRIDGE_BUILD_STAMP=0` | Use the mtime check instead of build stamps. |
| `NEONBRIDGE_PATCHES=0` | Build the trees without `vendor/patches`. |
| `NEONBRIDGE_WARM=0` | Skip the shader warm before a run. |
| `NEONBRIDGE_PRECOMPILE_GRAPHICS=0` | Skip ScarlettPort's graphics precompile (`NEONBRIDGE_GRAPHICS_IDLE_S`, default 60, is its idle limit per chunk). |
| `NEONBRIDGE_SHADER_PRUNE=0` | Keep every shader-cache toolchain folder (default: the 4 newest). |
| `NEONBRIDGE_SHADER_ROOT` | The shader cache folder (default `%LOCALAPPDATA%\6ixpack\cache\neonbridge-own`; `NEONBRIDGE_SHADER_ROOT_MOVE=0` keeps the old `%TEMP%` one). |
| `NEONBRIDGE_BUILD_VCVARS=0` | Run a Python build without the x64 Visual Studio environment. |
| `NEONBRIDGE_AGILITY_DIR` | The D3D12 Agility SDK DLLs (default `%LOCALAPPDATA%\6ixpack\agility-1.614.1`, from the official Microsoft.Direct3D.D3D12 1.614.1 NuGet package). |
| `NEONBRIDGE_FPS=1` | Start with the fps overlay shown. |
| `NEONBRIDGE_SPLASH=0` | No loader window. |
| `NEONBRIDGE_CACHE` | Another root for the runtimes' caches. |

## Files

| What | Where |
|---|---|
| Logs | `%LOCALAPPDATA%\6ixpack\logs\<runtime>_<time>.log`, the last one also in `%LOCALAPPDATA%\6ixpack\<runtime>\cache\last-run.log` |
| Saves | `%LOCALAPPDATA%\6ixpack\save` (the victory family shares it) |
| Caches | `%LOCALAPPDATA%\6ixpack\cache` (shared by the victory family), `%LOCALAPPDATA%\6ixpack\<runtime>\cache` (others) |
| Picker memory | `%LOCALAPPDATA%\6ixpack\last_game.txt`, `last_pick.txt` |
| Mixes | `%LOCALAPPDATA%\6ixpack\mix\<hash>` |

## Adding a runtime to Neonbridge

Each step is something an earlier runtime needed (named in brackets).

1. **Check it is a leader.** It must be in the [race tracker](https://github.com/vuyp/gta6-pc-race)'s top 10, and its repository must still exist (ztrx and victory-perf vanished). The licence must be GPL-3.0 or more permissive.
2. **Pin it.** Create `vendor/<name>/` with only `NEONBRIDGE.toml`, `LICENSE` and `UPSTREAM` (its full commit hash). The launcher downloads that commit from GitHub on first Play (after asking) into `%LOCALAPPDATA%ixpack\src\<name>-<commit>` and builds it there; the repo holds no runtime sources. Only a runtime we must change (ScarlettPort's xcl fix) keeps its tree in `vendor/`.
3. **Credit it.** Write `VENDORED.md` (source, commit, licence, author, what was removed) and add a row to `CREDITS.md`.
4. **Write `NEONBRIDGE.toml`, all values from the author.** Cite the README or script line each value came from. Its default launch is the author's own; anything of ours goes in a `preset_*` (tomls: author default).
   - `name`, `fork`, `rank`, `about`, `upstream`.
   - `build`, `watch`, `exe`. A runtime that lives in a subfolder needs `--manifest-path` and the exe path under it (AnyXboxX).
   - `diag_build`/`diag_exe` only if the author plays the diagnostics build, else `diag_build = []`. AnyXboxX's diagnostics build needed Agility 1.620.2 and had no device without it.
   - `run` with the author's console and flags; `prewarm` with the author's shader-prebuild command.
   - `game_env`, `cache_env`, `agility`, and `setup` steps if the author runs any before the first start.
   - `[env]` with the author's switches (from play.ps1 or the launcher script, e.g. victory-unified and 7900XTX). Add `VICTORY_DATA_DIR = "%LOCALAPPDATA%\\6ixpack\\save"`, written with doubled backslashes.
   - `[controls]`; `own_overlay = true` if it draws its own fps (Flamingo).
   - Prerequisites (GDK, Python, Agility) so the picker greys it out when one is missing (Five Star, Kelvaroq).
5. **Never touch its shader cache key:** `victory-shader`, the build kind, `shader_cache.rs` or the translator's switches. A key change cost a full 6-minute retranslation (rt-anyxboxx).
6. **Register it in `src/runtimes.rs`.** Add it to `cache_format` (VSH1 or VSH2) and the seed list. Add a `patch_root()` entry if it gets a patch.
7. **Patch only a proven crash.** Example: device slot `0x330` / method 102, which killed VI-BR-PORT, unified, AnyXboxX and Flamingo. Patches go in `vendor/patches/<fix>-<name>.patch`, never in `victory-shader`. The build stamp then rebuilds the runtime with the patch.
8. **Build and check.** `cargo check`, then rebuild `target/release/neonbridge.exe` (the desktop shortcut runs it). Don't build while a run is live.
9. **Test through the launcher only.** Prewarm first, then one run. Read the run with `triage.py` and the launcher's crash reason. Check that auto-A gets it past the start screens.
10. **Document it.** Add its row to the README table (author's claim beside our result). Add its build, cache format and family to "How each runtime is wired" below.
11. **Ship it.** Stage files by name and check the staged diff for private details. Commit on main and push with the hash guard.
12. **When it drops out of the top 10:** set `archived = true`. It stays in the launcher under Archived; don't delete it.

## How each runtime is wired

How Neonbridge builds, prepares and starts each runtime in `vendor/`. Every setting below is its author's own (the source is noted in its `vendor/<name>/NEONBRIDGE.toml`); the only things Neonbridge adds are listed under [What Neonbridge adds](#what-neonbridge-adds).

All nine are builds of [VIctory](https://github.com/Marvisak/VIctory) (Marvisak): the same `victory.exe` with the same commands (`run`, `translate`, `warm`). They differ in their fixes, their switches and their shader cache format.

### Family tree

```
VIctory (Marvisak)
├── victory-fork (fxceless)          → vendor/victory
│   ├── Flamingo (realblastex)       → vendor/flamingo
│   ├── VI-BR-PORT (edsuuu)          → vendor/vibrport
│   ├── victory-unified (Itsbatt)    → vendor/unified   (+ Belobug, DexNom)
│   └── KLBF Lab (PastiiJay)         → vendor/klbf   (VIctory + victory-fork)
├── AnyXboxX (LooperSalty)         → vendor/anyxboxx  (third_party/victory, Neptune base)
├── VIctory-Belobug-AMD (Belobug)    → vendor/belobug
├── VItoriaBr (uiliannn)             → vendor/vitoriabr
└── ScarlettPort (ZigZagBoi1)        → vendor/scarlettport/runtime/victory
```

### At a glance

1. **KLBF Lab** (`klbf`)
   - Build: `cargo build --release` → `target/release/victory.exe`
   - Prepare: `translate <game> --cache --pipelines`
   - Run: `run <game exe> --execute` (Series X profile). The `fsr` preset adds `--console=series-s` and AMD FidelityFX
   - Its switches: `SHADER_OPT=0`, `SKIP_PIPELINES=39f0e9aeb156a61c`, `LATE_WAIT=1`, `WIDE_BACK=1`, `PIPELINE_WAIT_MS=24`, `RELAY_COPY_QUEUE=0`
   - Shader cache: VSH1
2. **Flamingo** (`flamingo`)
   - Build: `cargo build --profile diag` → `target/diag/victory.exe`
   - Prepare: `translate <game> --warm all`
   - Run: `run <game exe> --execute --console series-s`
   - Its switches: ray tracing off, `NO_WATCHDOG`, `RELAY=1`, `LATE_WAIT=3`, 1 GB card buffers, `WIDE_MERGE`, its own on-screen display
   - Shader cache: VSH2
3. **victory-fork** (`victory`)
   - Build: `cargo build --release` (`--profile diag --features diagnostics` for diagnostics)
   - Prepare: `translate <game> --warm all`
   - Run: `run <game exe> --execute` (Series X profile)
   - Its switches: ray tracing off
   - Shader cache: VSH1
4. **VI-BR-PORT** (`vibrport`)
   - Build: `cargo build --profile diag --features diagnostics` → `target/diag/victory.exe`
   - Prepare: `translate <game> --write-cache --used-only`
   - Run: `run <game exe> --execute` (Series X profile)
   - Its switches, for 8 GB cards: `RELAY=1`, `WIDE_BACK`, `WIDE_REFRESH`, `DISPLAY_MODE=1920x1080`, `NO_WATCHDOG`
   - Shader cache: VSH1
5. **VIctory-Belobug-AMD** (`belobug`)
   - Build: `cargo build --release`
   - Prepare: `translate <game>`
   - Run: `run <game exe> --execute`
   - Its switches: none (its defaults are tuned for AMD Radeon)
   - Shader cache: VSH2
6. **VItoriaBr** (`vitoriabr`)
   - Build: `cargo build --release`
   - Prepare: `translate <game> --warm all`
   - Run: `run <game exe> --execute`
   - Its switches: ray tracing off, `RELAY=2`, `TEXTURE_NO_WAIT`, 6 GB card buffers, `WIDE_REFRESH`, `WIDE_MERGE`, `JOIN_COPIES`, `DISPLAY_MODE=1920x1080`
   - Shader cache: VSH1 / VSH3
7. **ScarlettPort** (`scarlettport`)
   - Build: `cargo rustc` of `runtime/victory` with diagnostics, as its `play-victory.ps1` does
   - Prepare: `warm <game>`
   - Run: `run <game exe> --execute --console series-s`, with the D3D12 Agility SDK 1.614.1 beside the exe
   - Its switches: its own upscaler, `NO_WATCHDOG`, `RELAY=2`, precise sync, precompiled graphics pipelines, the `NARROW_*` and `PREP_*` set, ray tracing off
   - Shader cache: VSH1
8. **victory-unified** (`unified`)
   - Build: `cargo build --release` → `target/release/victory.exe`
   - Prepare: `translate <game> --warm all` (its base victory-fork's warm; its README names none)
   - Run: `run <game exe> --execute` (Series X profile)
   - Its switches, from its `scripts/play.ps1` for an RTX 3070 8 GB: `ASYNC_SUBMIT`, `QUEUE2` (128 MB landing), `HOIST_CLAMPS=7`, `AMBIENT=1.25`, `MAX_FPS=30`, `LEVELS_ON_DEMAND`, `TEXTURE_NO_WAIT`, `TEXTURE_BUDGET_MB=1920`, 768 MB card buffers and 15 more
   - Shader cache: VSH1
9. **AnyXboxX** (`anyxboxx`)
   - Build: `cargo build --release` of `third_party/victory` → `third_party/victory/target/release/victory.exe`
   - Prepare: `warm` of `shaders.rpf`, the game and its two DLLs (its `lancer_victory.ps1 -Precompile`)
   - Run: `run <game exe> --execute` (Series X profile)
   - Its switches: Neptune's set from its `tools/lancer_victory.ps1` (37, tuned for a 16 GB card: `CARD_WINDOWS_MB=4096`, `CARD_BUFFERS_MB=1536`, `RELAY=2`, `LEVELS_ON_DEMAND`, `PIPELINE_WAIT_MS=2500` and more)
   - Shader cache: VSH1

Every switch above is a `VICTORY_` environment variable (`RELAY=1` means `VICTORY_RELAY=1`).

### What Neonbridge adds

The same for every runtime, and none of it changes how a runtime draws:

- **Folders:**
  - `VICTORY_GAME_DIR` points at your game.
  - `VICTORY_CACHE_DIR` points at the runtime's own cache, because the cache formats above differ.
  - `VICTORY_DATA_DIR` points at one save and settings folder for the whole family, `%LOCALAPPDATA%\6ixpack\save`.
- **Before the run:**
  - Checks the graphics card (`--check`) and the game files (`--verify`).
  - Lighter graphics settings in the game's `settings.xml` (`NEONBRIDGE_SAFE_SETTINGS=0` keeps yours).
  - The shader warm (the Prepare step above).
- **During the run:**
  - Auto-press A (`VICTORY_PRESS_FILE`, `NEONBRIDGE_AUTO_A=0` off).
  - The phone stream.
- **After the run:**
  - A frame-rate summary.
  - A cache backup.
- **Display settings, only when you set them:**
  - `NEONBRIDGE_RESOLUTION` sets `VICTORY_DISPLAY_MODE`.
  - `NEONBRIDGE_FPS_CAP` sets `VICTORY_PACE_FPS`.

### Mixing

The family shares one binding ABI, so a crate from one tree can usually replace the same crate in another. Mixing a VSH2 runtime (Flamingo, Belobug) with a VSH1 one rebuilds the shader cache. See "Mix and match" in the README: no mix has been tested yet.

### Known gaps seen here

- **VI-BR-PORT:** the game stops itself on a breakpoint about 20 s in, after the menus, right after calling a graphics-device method that VI-BR-PORT does not implement (`method not implemented object="device" slot=0x330`). Seen on an RTX 4060 8 GB, every run, also when run alone exactly as its README says (so not caused by Neonbridge); the other runtimes never call it.
