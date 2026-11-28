# i95Loader: GTA 6 PC Launcher for every GTA VI Xbox Series runtime (VIctory, Kelvaroq, VItoriaBr and more)

Try them all from one window.

i95 is a Windows launcher for the projects trying to run the Xbox Series version of GTA VI on PC. It doesn't include or download the game, and there are no game files, keys or protection bypasses in it.

**It builds every runtime from source on your PC.** Each runtime's source code is pinned to one commit you can read on GitHub, and i95 compiles it locally with Rust, so you never run an .exe someone else built. That's safer than downloading prebuilt runtime exes, which is how fake "GTA 6 PC" malware spreads.

## Get started

1. Install [Rust](https://rustup.rs) and [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with **Desktop development with C++** (the x64 C++ tools and the Windows SDK). i95 needs them to build each runtime on your PC.
2. Download `i95-<version>.zip` from the [latest release](https://github.com/DesignWithTommy/i95-GTA6-PC-Launcher/releases/latest), unzip it anywhere and run `i95.exe`.
3. Point it at your own copy of the game's Xbox Series folder, pick a runtime and a preset, and press **Play**. The first start of a runtime builds it and prepares its shaders, which can take a while; later starts are quick.

On an 8 GB card, pick a runtime's `8gb` or `series_s` preset: most runtimes default to Series X and run out of graphics memory there.

## What it does

- **One list of runtimes**, ranked like the race tracker, each with its author's own default launch settings.
- **Stays current without an update**: at start it reads the runtime list from [catalog.txt](catalog.txt) here. Each runtime is moved to its author's newest commit several times a day (scripts/bump-upstreams.sh, no bot), and any commit that changes the author's launch settings is read before it goes in. Every launcher picks up the list on its next start. No new release is needed. Offline it uses the list it shipped with.
- **Builds and rebuilds** a runtime only when its source changed, and keeps each runtime's shader cache when it does. Runtimes run as their authors ship them: i95's own patches, shared saves and shared caches are off unless you turn them on.
- **Prepares shaders** before the game starts, with a loading screen that shows real progress. Your own `splash.bmp` in the output folder shows on the loading screen.
- **First launch** asks for the output folder (builds, caches, saves, logs, crash reports) and the game's folder before anything else.
- **Checks your PC first**: graphics card features (D3D12, Shader Model 6.6, mesh shaders), RAM and free disk, and names what's missing.
- **Presets** per runtime, such as Series S, 8 GB cards, 16 GB laptops, DLSS to 4K, 16 GB of RAM (Vice-Tay's `4060` preset: 38 min at ~10 fps on an RTX 4060 here) and crash diagnostics, plus any extra setting a runtime reads.
- **In game**: each runtime's own overlay as its author ships it, a controls overlay (F1), and the display kept awake. i95 adds no fps counter of its own.
- **Advanced** (next to Play) has on / off boxes, remembered between runs, so nothing needs a command: the runtime's own overlay, the controls overlay, one save folder for every victory runtime, saving a run log, extra runtime logging and diagnostics (the logs and diagnostics are off by default). It also takes any extra `KEY=VALUE` setting a runtime reads, with **Options list** showing them.
- **After a run**: explains why it ended by reading the log, writes a short timeline, and can file an issue report. A run that crashes leaves a crash report (exit code, settings and its last 300 lines, even with logging off) in `crashes` in the output folder; nothing is sent anywhere.
- **Updates itself** from the latest release at start (`NEONBRIDGE_UPDATE=0` skips it), and remembers your last runtime, preset, settings, Advanced boxes and output folder.
- **Asks before downloading** a runtime's source or an optional extra. The one thing it fetches without asking is the small runtime list, catalog.txt.

**None of these runtimes is confirmed playable yet.** The tracker's bar for playable is 20 fps and 30 minutes without a crash. Only Vice-Tay reports clearing it (a 30-minute session at a 21.1 fps median on an RTX 4090), and nobody outside the project has checked it.

## Standings

All 66 runtimes, ranked with [PortWatch VI](https://portwatch-vi.xyz/)'s scoring formula, are on the **[website](https://designwithtommy.github.io/i95-GTA6-PC-Launcher/#standings)** and in [STANDINGS.md](STANDINGS.md), with each author's own report and our results on an RTX 4060. The launcher lists them in the same order.

## FAQ

- **Does it work on Windows 10?** Not for every runtime. KLBF Lab needs Shader Model 6.6, and Windows 10's built-in DirectX 12 stops at 6.5; KLBF doesn't load Microsoft's newer DirectX 12 (the Agility SDK), so it crashes on Windows 10. Vilocity does load it.
- **Can I play GTA 6 on PC with this?** Not yet. Every runtime here is still being worked on.
- **Is this a download or a crack?** No. There are no game files, keys or protection bypasses.
- **Whose work is this?** Mostly the people who make the runtimes. i95 is the launcher around them. See [CREDITS.md](docs/CREDITS.md).

## Credits

Every runtime and every project i95 borrows from is in [CREDITS.md](docs/CREDITS.md), with its authors, licence and version. Thanks to all of them, and to [vuyp](https://github.com/vuyp) for the [GTA VI PC race tracker](https://vuyp.github.io/gta6-pc-race/) ([repo](https://github.com/vuyp/gta6-pc-race)) and [PortWatch VI](https://portwatch-vi.xyz/), whose scoring the ranks use.

## Licence

GPL-3.0-or-later ([LICENSE](LICENSE)). Each runtime keeps its own licence in `vendor/<name>/LICENSE`. The VIctory forks and ScarlettPort are GPL-3.0, and i95's own TuxBox-based code is Apache-2.0. Grand Theft Auto, GTA and Rockstar Games are trademarks of Take-Two Interactive. Xbox is a trademark of Microsoft. i95 isn't affiliated with or endorsed by them.

<sub>Keywords: GTA 6 PC, GTA VI PC, GTA 6 launcher, Grand Theft Auto VI Windows, Xbox Series X|S to PC, Scarlett, VIctory, victory-fork, AnyXboxX, Kelvaroq, KLBF Lab, Flamingo, Belobug, VItoriaBr, ScarlettPort, Vilocity, cool_fox, Mandy, XCL, ClaudeTheftAuto, CTA.</sub>
