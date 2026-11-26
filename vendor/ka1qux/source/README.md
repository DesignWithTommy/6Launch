# BRVIctory-ka1qux

A fork of [VIctory](https://github.com/Marvisak/VIctory), a compatibility layer
that runs an Xbox Series title natively on Windows from files the user supplies.

Since 26/11/2026 this repository is based on
**[avatarmanz/VIctory-7900XTX](https://github.com/avatarmanz/VIctory-7900XTX)**
(GPL-3.0), which on this PC runs about 2-3 times faster than our previous base
(itself built on [fxceless/victory-fork](https://github.com/fxceless/victory-fork)).
Our own fixes from the previous base are being brought over on top, one at a time,
each measured in the game.

**This repository contains no game files, code or assets.** You need your own
copy of the game. It is not affiliated with or endorsed by the game's developer
or publisher.

## Where it stands (26/11/2026)

Measured on one PC: Windows 11, Intel i5-13600KF, **32 GB RAM**, Radeon
RX 7800 XT 16 GB. Open-world street, renderer patches 1, 2, 5, 6 and 7 on,
`VICTORY_RESIDENCY=1 VICTORY_BURY_LAG=8`, median of 26 or more intervals.

| | Frame (median) | About |
|---|---|---|
| Series X profile, ray tracing off | 95 ms | 10.5 fps |
| Series X profile, ray tracing on | 148 ms (card: 79 ms a frame) | 6.7 fps |
| Series S profile, ray tracing off | 68 ms | 15 fps |
| Previous base (Series X, ray tracing on) | 211-250 ms | 4-4.7 fps |

With ray tracing the frame is limited by the card; without it, by the thread
that submits the title's work. Ray tracing changes the picture a lot: glass
buildings that are black without it show their reflections.

Known problems:

- **Memory grows during a long session** (private memory 25 → 30-32 GB in about
  10 minutes of play): on a 32 GB PC Windows then pages and frames reach
  150-300 ms with stalls. Being worked on (a table of placed-view shapes that
  never stops growing is the first suspect).
- Textures of what just came into view appear late while the title streams.
- Mouths of characters do not move yet (a fix exists in the fork; next in line).
- The first run of a new build stutters while the driver makes pipelines.

## What changed here on top of the base

- Code that came from XCL (GPL-2.0-only) is removed and replaced with our own:
  the instruction emulation in `crash.rs` is VIctory's own, and the jump stubs of
  the patched sites are written anew (`victory-loader/src/stubs.rs`).
- **The renderer patches need a profile of your own copy.** The places in the
  title's code that patches 2 and the frame rate / resolution settings change
  are not in this repository: they are read at startup from a profile file of
  your own (`VICTORY_TITLE_PROFILE`, or `title_profile` under `[paths]` in
  `victory.toml`; the format is described at the top of
  `crates/victory-loader/src/title_profile.rs`). Without it those patches stay
  off and the log says so.
- BCPack decoding fixes (upstream #2648/#2649, fork #374, BC7 rotation only in
  modes 4 and 5).
- Release build with thin LTO and `x86-64-v3`.
- A dropped view heap forgets its slots (`memory::forget_heap`); bound render
  targets are kept until the list is reset (upstream #3128).
- Memory: the shapes of placed views are keyed without the description's filler
  words; idle snapshot buffers and per-thread read buffers are let go (after
  upstream #3230); the pools of idle host lists are capped and trimmed
  (`VICTORY_LIST_POOL_CAP`, `VICTORY_LIST_POOL_TRIM`).

## How to run it

Requirements, build and the launcher are those of the base: see
[docs/README-base.md](docs/README-base.md). In short:

```powershell
copy victory.example.toml victory.toml      # then set your paths, outside this folder
cargo build --locked --profile diag -p victory --features diagnostics
scripts\play.ps1 -Exe <path of your copy's executable>
```

`-RayTracing` turns the title's ray-traced indirect light on. Set
`VICTORY_TITLE_PROFILE` to your profile for renderer patch 2 and the frame
rate / output size settings.

This snapshot (the base with the cleanup above) passed the checks below but was
not run in the game yet; the numbers above are from the same base just before
the cleanup, which moved the title-specific places out of the code without
changing what is written.

## Credits

This is a tree of other people's work with more on top.

- **[VIctory](https://github.com/Marvisak/VIctory)** by Marvisak and its
  contributors.
- **[avatarmanz/VIctory-7900XTX](https://github.com/avatarmanz/VIctory-7900XTX)**:
  the base of this repository (renderer patches 1-7, recording on our own
  threads, card steps and much more). Its history is not repeated here; see
  that repository.
- **[fxceless/victory-fork](https://github.com/fxceless/victory-fork)** and the
  people whose pull requests it carries; **[DexNom/VIctory](https://github.com/DexNom/VIctory)**;
  **[Belobug/VIctory-Belobug-AMD](https://github.com/Belobug/VIctory-Belobug-AMD)**;
  **[myokex/victory-amd](https://github.com/myokex/victory-amd)**; and everyone
  credited in [docs/README-base.md](docs/README-base.md).
- XCL (GPL-2.0-only) was read for ideas only; no code of it is here.
- This fork's changes were made with [Claude Code](https://claude.com/claude-code).

## License

GPL-3.0-or-later, as VIctory. See [LICENSE](LICENSE).
