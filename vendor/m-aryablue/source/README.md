# VIctoryBlue (work in progress)

A work-in-progress fork of [VIctory](https://github.com/Marvisak/VIctory), a
compatibility layer that runs the Xbox Series build of Grand Theft Auto VI
natively on Windows, from files the user supplies. This fork is built on
[fxceless/victory-fork](https://github.com/fxceless/victory-fork) and aims at
one goal: a playable frame rate on a mid-range NVIDIA PC.

**This repository contains no game files, code or assets.** You need your own
copy of the game. It is not affiliated with or endorsed by the game's
developer or publisher.

## Status (2026-11-25)

**In game and controllable, but not playable yet: about 7.5 frames per second.**

Tested on one machine: Windows 11, NVIDIA RTX 3060 12 GB, AMD 16-thread
CPU, 48 GB RAM.

| Stage | State |
|-------|-------|
| Boot, menus, loading | works |
| Opening cutscene | plays, lit |
| Gameplay (the prologue) | you can walk and look around as Lucia; world, characters and textures render |
| Series S profile (`--console series-s`) | reaches gameplay; the game's own upscaler path runs |
| Upscaling | the game's FSR calls are served on the PC: NVIDIA DLSS (Super Resolution) or AMD FSR 3.1, 1280x720 to 1920x1080 |
| Frame rate in the world | **7.4-7.6 fps** settled with Series S + DLSS and shadow cascades capped at three (131-135 ms a frame, median of four five-minute runs; the fastest stretches reach 8); **6.7-6.9 fps** with full shadow cascades. Measured in the opening cutscene, which unattended runs do not yet leave. It was 5.2-5.6 fps earlier the same day and about 6.2 before per-draw table reuse. The Series X profile has not been re-measured (4.5-4.6 fps at 1080p before) |
| Stability | full five-minute runs with no device loss |
| Pause menu, HUD layer | the game presents its interface as a second 4K layer; drawing it is fixed on a branch (the pause menu shows in full there) and not merged here yet |
| Light in shadow | still dark: the game's ray-traced lighting does not run yet |

### Where the frame goes

| Part | ms per frame |
|------|--------------|
| Our submit thread preparing the frame (tracking what the game changed in its memory, copying it to the card) | ~105-111 |
| The game's command lists on the GPU (Series S, DLSS) | ~75-79 |
| The card waiting for us | ~50-54 |
| Copying back what the GPU wrote, before the game's signals go out | ~55 |
| The game's recording threads inside our calls, summed over threads | ~250 |

The Xbox shares one memory between CPU and GPU; on a PC that has to be
emulated by copying, and most of that work still runs on one thread. Inside
the ~105 ms: merging what changed under the GPU's own writes ~32, deferred
refresh steps ~25, the last flush ~24, descriptor heap sync ~18, card checks
~13. The target is 15 fps (66 ms a frame), which needs each of the three
budgets (preparing, recording threads, GPU) under about 66 ms; 10 fps needs
100 ms frames.

What moved the number on 2026-11-25: reusing a descriptor table already seen
in the frame (`VICTORY_TABLE_REUSE`, 151 to 131 ms a frame), lock striping for
made views, batched heap publishing, and a held texture reserve
(`VICTORY_TEXTURE_HOLD`, 93 % fewer megabytes of textures made again).
Measured and found to give nothing here: switching off the driver's
background shader work, in-place (system memory) copies on this GPU, and a
GPU upload heap (this card reports no support).

### Running the fastest configuration

```
VICTORY_UPSCALER=dlss  VICTORY_DLSS=quality  VICTORY_DLSS_DLL_DIR=<folder with nvngx_dlss.dll>
VICTORY_TABLE_REUSE=1  VICTORY_TEXTURE_HOLD=1  VICTORY_TEXTURE_KEEP=1  VICTORY_TEXTURE_WORKER=1
VICTORY_TEXTURE_WORKER_NEWEST=1  VICTORY_TEXTURE_SHORT_HOLD=1
VICTORY_MADE_FAST=1  VICTORY_MADE_STRIPES=1  VICTORY_MADE_BATCH_PUBLISH=1  VICTORY_HEAP_SYNC_BATCH=1
VICTORY_HEAP_WALK_THREAD=1  VICTORY_LOOKUP_FAST=1  VICTORY_VIEW_MEMO=1  VICTORY_WINDOWS_MANY=1
VICTORY_MERGE_MASKED=1  VICTORY_MERGE_MASKED_INDEX=1  VICTORY_CARD_HASH_WAVE=1  VICTORY_CARD_REHASH_RUNS=1
VICTORY_WIDE_COPIES=1  VICTORY_WIDE_GUARDS=1  VICTORY_WIDE_ASKS=1  VICTORY_FLUSH_WIDE=0
VICTORY_FINE_PAGES=1  VICTORY_FAST_READ=1  VICTORY_GUARD_SEEN=1  VICTORY_GUARD_COUNT=1  VICTORY_ASK_LEAN=1
VICTORY_JOURNAL_FLAT=1  VICTORY_PIECES_FLAT=1  VICTORY_COMMIT_NOTICE=1  VICTORY_COPY_FLUSH_OVERLAP=1
VICTORY_JOIN_PARTS=1  VICTORY_BACK_OWN_VIEW=1  VICTORY_INDIRECT_GATHER=1  VICTORY_CLAMP_BATCH=1
VICTORY_LATE_WAIT_LISTS=512
victory run <path to game_scarlett_master.exe> --execute --console series-s
```

That is the set the 7.4-7.6 fps runs used, together with
`VICTORY_SHADOW_CASCADES=3`, which caps the shadow cascades. The cap is left
out above because it lowers picture quality, and it matters: the same set
without it measured 6.7-6.9 fps (142-150 ms a frame) in two runs, so the cap
is worth about 10 ms a frame here. All in-place (`VICTORY_INPLACE*`) switches
were off.

Build with the `dlss` cargo feature and `VICTORY_DLSS_SDK` pointing at
NVIDIA's [DLSS SDK](https://github.com/NVIDIA/DLSS); the DLSS runtime is not
included here. For AMD FSR 3.1 instead, set `VICTORY_FIDELITYFX_DIR` to a
folder holding AMD's signed `amd_fidelityfx_upscaler_dx12.dll` (FidelityFX SDK).
On the Series X profile, `VICTORY_FFX_FORCE=1` makes the game take its FSR
path too. `VICTORY_DISPLAY_MODE` sets the display size the game is told
(1920x1080 by default in this fork).

### What this fork adds

- Series S profile reaching gameplay, and the game's FidelityFX upscaler API
  served on the PC by NVIDIA DLSS or AMD FSR 3.1.
- Frame time: heap refresh 103 to 12 ms a frame, stale settles 75 to 0 ms,
  masked merge bookkeeping indexed, texture uploads moved to a worker with a
  backlog-aware time cap, write-watch and page-query cuts, copies spread over
  worker threads (`VICTORY_WIDE_COPIES`).
- Frame time, 2026-11-25: per-draw descriptor table reuse, striped and
  lock-free lookups of made views, batched heap publishing, a held texture
  reserve, stale heap slots made again when their copy changes, an indexed
  list of pending GPU-write ranges (`VICTORY_PENDING_INDEX`, with a checker),
  per-megabyte change counters for refresh steps (`VICTORY_PUSH_DIRTY`, with a
  checker), and switches that flip in the middle of a run for A/B timing
  (`VICTORY_ALT`).
- Input: keyboard Esc and Enter reach the first connected controller, so the
  pause opens with a pad plugged in.
- Diagnostics: per-frame frame log and card meter (from victory-fork), a
  driver-error watch, census of title memory for in-place use, step timers
  for the submit thread.
- Selective in-place memory (`VICTORY_INPLACE_PICK`, experimental): putting
  everything in system memory halved the frame rate on this GPU, so only
  buffers the card reads rarely are tried, and each trial is taken back if
  it costs time.

Most changes sit behind `VICTORY_*` environment switches, off by default;
each is described where it is read.

## Credits

- **[VIctory](https://github.com/Marvisak/VIctory)** by Marvisak and its
  contributors: the base of everything here. GPL-3.0-or-later; so is this fork.
- **[victory-fork](https://github.com/fxceless/victory-fork)** by **fxceless**:
  this fork's base, including the speed switches, frame log, card meter and
  driver watch, and the credits it carries (XCL by ivall as a reference;
  myokex's page-view guard after work by Spexivus, joroyt and tightfall;
  Joshua Odom's read-beside fix; Flyander's descriptor mirror, PR #305).
- **[ztrx](https://github.com/Zettrox/ztrx)** by **Zettrox**: the first build
  we saw reach gameplay, used as a reference.
- Fixes and ideas from **ZazaJr24**, **cxccxc**, **oddiums**, **Halfect**
  (VIctory PR #1059), **Jonirulah**, **triippywhale** and **asdaasdsaddf**.
- **AMD FidelityFX SDK** (FSR 3.1) and **NVIDIA DLSS SDK**, used through their
  official runtimes, which are not redistributed here.
- This fork's changes were developed with [Claude Code](https://claude.com/claude-code).

### Known problems

- About 7.5 fps; not playable yet.
- Long frames (over 400 ms) while the game streams new areas: its render
  thread waits on loading, and our disk-read emulation is the suspect.
- The Series S profile runs the game's lowest geometry settings
  (tessellation, terrain and trees at Low), so the world looks low-poly.
- The pause menu and HUD layer are not drawn in this branch yet.
- Everything in shadow is dark (ray-traced lighting not running).
- 2 shaders still don't translate, and 315 draws a frame are dropped
  (mostly the ray-tracing structure builder).
- Many changes are specific to this game and this kind of GPU.

Everything below this line is upstream VIctory's own README, unchanged; its
status section describes upstream, not this fork.

---

# VIctory

Runs the title (Xbox Series X|S) natively on Windows, from files
the user supplies.

The title's x86-64 code runs unchanged inside our process; VIctory supplies
the platform underneath it and converts the title's data, ahead of time where
it can and on first use where it must (shaders). See
[docs/architecture.md](docs/architecture.md).

## Status (2026-11-23)

**Not playable yet.** The title boots, runs its menus at its own 30 fps and
loads the world, where rendering fails.

| Stage | State |
|-------|-------|
| Boot, first-boot screens (accessibility, language, settings, legal) | works, 30 fps |
| Intro video with sound | works |
| Start screen, profile and saves | works |
| Loading into the world (about 50 s) | works |
| The world (first mission's intro) | reached and runs with indirect draws; its title card renders, the scene after it is still dark; 0.5-1.8 s per frame |

Tested on one machine: Windows 11, AMD RX 6700 XT.

> **Warning.** Entering the world on a real GPU can currently hang it, and
> Windows then resets the display driver (every monitor goes black for a
> moment). To look further, use a diagnostics build with
> `VICTORY_D3D12_WARP=1`, which renders on the software adapter.

What each part does today and what it still needs:

| Subsystem | Today | Needs |
|-----------|-------|-------|
| Loader | Runs the title's executable and its DLLs in our process; crash reports name the faulting code and are kept in `crashes.log` | – |
| Runtime services | One signed-in offline user, licences, packages, saves that survive a kill at any step, text entry dialog | Speech is silent; the dialog and relaunch are untested live |
| Network | Offline by design: the title cannot reach the network | – |
| Files and streaming | Archives and streamed data read, zlib and BCPack decoded in parallel | Loading is limited by writes into title memory |
| Input | XInput pads, keyboard, keyboard stand-in pad | Trigger rumble |
| Audio | Menu, video and loading sound; XMA voices and Opus streams decode | Reverb is silent; Opus results unverified against the title |
| GPU: menus | Draws, compute and presentation on host D3D12; only changed title memory is copied | Frame pacing drifts on one screen in release builds |
| GPU: world | Mesh pipelines, shader-written buffers, descriptor lifetimes, indirect execution (arguments clamped), pipelines made by worker threads and kept on disk | Most of the scene is still missing (untranslated shaders drop draws); frames take seconds; real queries, depth bounds, HDR and scaling are missing |
| Shaders | Console GPU code recompiled to DXIL at first use, cached on disk. Menus: all. World: mesh 8,191 of 8,506, vertex/pixel 10,617 of 12,346, compute 1,643 of 2,539 | 64-bit image atomics, merged geometry stages, the ray-tracing builder's shaders; some compute programs need a GPU with 64-lane waves |
| Textures | Console tiled layouts and detiling for the modes the title uses | Render-target metadata planes |
| Series S mode | Runs through the menus to the loading screen | The world, as above |

## Getting started

Requirements: Windows 10/11 x64, a CPU with AVX2, a Direct3D 12 GPU with
shader model 6.6 (and mesh shaders for the world), Rust (pinned in
`rust-toolchain.toml`).

The title also needs several GiB of available Windows commit at startup.
Check **Committed** under **Task Manager → Performance → Memory**. If
`crashes.log` reports Windows error 1455, close other applications or increase
the Windows paging file size, then retry.

```powershell
cargo xtask install-hooks                # asset-policy check on every commit
copy victory.example.toml victory.toml   # then set your paths (outside the repo)
cargo test --workspace                   # needs no game data
cargo run -p victory -- config           # check your paths
```

Run the title:

```powershell
cargo build --release -p victory
target\release\victory.exe run <game dir>\<the title's exe> --execute
```

The window caption shows the framerate. Saves, settings and `crashes.log`
are kept under the data folder (default `%LOCALAPPDATA%\victory\data`), derived
data under the cache folder.

Input: XInput pads. With none connected the keyboard is a pad while the
title's window has focus:

| Keys | Pad |
|------|-----|
| W A S D / I J K L | left / right stick |
| arrows | D-pad |
| Enter, Backspace, X, Y | A, B, X, Y |
| Q / E, Z / C | shoulders, triggers |
| F / H | stick clicks |
| Esc, Tab | Menu, View |

## Commands

```
victory inspect <exe|dll> [--imports] [--exports]
victory census [paths...] [--missing] [--strict]
victory lib-exports <import.lib> [--list]
victory shader-info [paths...] [--verbose] [--disasm]
victory translate [paths...] [--validate] [--pipelines] [--list] [--jobs N]
victory run <exe> [--execute] [--unimplemented trap|zero] [--strict]
            [--console series-x|series-s] [-- <title arguments>]
victory config
```

Logging: `VICTORY_LOG=victory_loader=trace,info` (a release build logs
info and above).

## Debugging

A release build has no debugging aids. Build them in with

```powershell
cargo build --profile diag -p victory --features diagnostics   # target\diag\victory.exe
```

| Variable | Effect |
|----------|--------|
| `VICTORY_LOG=<target>=debug,...` | debug and trace output |
| `VICTORY_STACKS=20,60` | every thread's stack at those seconds |
| `VICTORY_WATCH=<rva>,...` | logs a title u32 when it changes |
| `VICTORY_KEYS=30:Enter,35:X` | presses keys at those seconds, focus or not |
| `VICTORY_INPUT_LATCH=0` | turns off holding a press until the title has read it (on by default; at ~1 fps the title reads input once a frame) |
| `VICTORY_DISPLAY_MODE=WxH` | the display size reported to the title (1920x1080 by default; `3840x2160` for the old 4K mode); the world renders at two thirds of it |
| `VICTORY_CRASH_SAVE=<rva>:<len>,...` | saves title memory to the cache folder on a crash |
| `VICTORY_CAPTURE=<dir>` | saves presented frames (powers of two, or `VICTORY_CAPTURE_FRAMES=<list>`) |
| `VICTORY_DUMP_TARGETS=1` | with the two above, saves every render target of those frames |
| `VICTORY_D3D12_DEBUG=1` | host debug layer |
| `VICTORY_D3D12_GBV=1` | with the above, GPU-based validation (bad descriptor use reported, not faulted; slow) |
| `VICTORY_D3D12_DRED=1` | on a GPU failure, the allocation hit and the commands in flight |
| `VICTORY_PIPELINE_WAIT_MS=24` | reduce the per-frame wait for background pipeline creation from the default of 500 ms. A draw can be omitted until its pipeline is ready |
| `VICTORY_SKIP_PIPELINES=<hash>,...` | omit draws and dispatches, including indirect ones, whose pipeline contains a listed 16-digit shader hash |
| `VICTORY_GPU_TIMING=1` | with the above, each list of a submission run and timed on its own; slow lists logged with their slowest commands |
| `VICTORY_D3D12_WARP=1` | host device on the software adapter (slow; cannot reset the driver) |
| `VICTORY_SHADER_LOOP_LIMIT=<n>` | translated shader loops stop after `n` iterations per lane instead of 2^20 (own shader cache entries); if a lower `n` makes world frames much faster, loops hitting the limit are what makes them slow |
| `VICTORY_AUDIO_CAPTURE=<file.wav>` | records what the title plays (our process only) |
| `VICTORY_XMA_DUMP=<dir>` | saves the first 32 XMA voices' context and input |

Every build appends crashes and host GPU failures to `crashes.log` in the
title's data folder.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
