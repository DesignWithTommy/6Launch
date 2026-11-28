# i95Loader: GTA 6 PC Launcher for every GTA VI Xbox Series runtime (VIctory, Kelvaroq, VItoriaBr and more)

Try them all from one window.

i95 is a Windows launcher for the projects trying to run the Xbox Series version of GTA VI on PC. It doesn't include or download the game, and there are no game files, keys or protection bypasses in it.

**It builds every runtime from source on your PC.** Each runtime's source code is pinned to one commit you can read on GitHub, and i95 compiles it locally with Rust, so you never run an .exe someone else built (the one exception, RAZDA4A-XEN, ships only a prebuilt exe and is marked so). That's safer than downloading prebuilt runtime exes, which is how fake "GTA 6 PC" malware spreads.

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

i95 ranks every runtime itself, with [PortWatch VI](https://portwatch-vi.xyz/)'s scoring formula (how far it gets, then story, picture, fps against 30 and stability). Where PortWatch has assessed a runtime we use its numbers; the rest are scored from each author's own report by `scripts/rank.py`. The launcher lists them in this order. vuyp's [race tracker](https://vuyp.github.io/gta6-pc-race/) ranks fewer of them.

"Its author reports" is what each project says about itself, in its own words where we could, cut short here; **Full reports** below the table has every word. "On our RTX 4060" is our own PC: i5-14400F, RTX 4060 8 GB, 16 GB RAM, Windows 11. Most authors test on faster cards, so expect less on yours if it's like ours.

| # | Score | Runtime | GPU | Its author reports | On our RTX 4060 |
|---:|---:|---|---|---|---|
| 1 | 90 | [VIctory-NVIDIA](https://github.com/trustno1-coder/VIctory-NVIDIA) | NVIDIA | CGBJ renderer with DLSS: 26 to 30 fps in the opening rooms and the first boat ride, RTX… | — |
| 2 | 77.4 | [DexNom RayTime](https://github.com/DexNom/VIctory) | AMD | 1 fps with ray tracing on, 3.5 to 4 without, RX 7900 XTX | — |
| 3 | 73 | [Vice-Tay](https://github.com/TaynanGT/Vice-Tay) | NVIDIA | The tracker's leader and playable by its own report: a 21.1 fps median over a 30-minute… | ✅ The `4060` preset: 38 min in 3D at 10.3 fps on average |
| 4 | 68.8 | [Kelvaroq](https://github.com/stellarr-studio/kelvaroq) | NVIDIA | ~10 fps in typical views | Testing |
| 5 | 68.3 | [Vilocity](https://github.com/voidopcode/vilocity) | NVIDIA | 33 fps median through the opening and 25 to 29 in the open world, 12 minutes of each… | — |
| 6 | 68.1 | [VIctory-RDNA](https://github.com/XaviFortes/VIctory-RDNA) | AMD | 19.1 to 23.1 fps medians at Series X 4K in one capture under 10 minutes, RX 7900 XT | — |
| 7 | 67.5 | [KLBF Lab](https://github.com/PastiiJay/klbf-lab) | AMD | 14.0 fps average over 37 minutes of play, player in control through the prologue | Testing again. Its intro crash was i95 leaving TuxBox DLLs… |
| 8 | 67.5 | [victory-8gb](https://github.com/CuteBooty/victory-8gb) | NVIDIA 8 GB | First Chapter I mission played, a 5 h 45 min session without a crash, RTX 3070 8 GB | — |
| 9 | 67.3 | [VIctorious](https://github.com/randomguy12000/VIctorious) | AMD | ~26 fps driving in the world, Series S, RX 9070 XT | — |
| 10 | 66.8 | [El1xr](https://github.com/kylepelham/El1xr) | NVIDIA | 23.1-25.7 fps at 4K, RTX 5090 | — |
| 11 | 66.5 | [playablegames-peppa](https://github.com/Peppacodes/playablegames-peppa) | NVIDIA | 23.5 fps over 12 minutes in a house interior, RTX 5080 16 GB | — |
| 12 | 64.9 | [mavis-9070xt](https://github.com/mavis140/mavis-9070xt) | AMD | 18.9 fps median over a 27-minute session, Series X at 1440p, ray tracing off, RX 9070 XT… | — |
| 13 | 64 | [DanziLoader-Cpp](https://github.com/Veria70/DanziLoader-Cpp) | NVIDIA | ~11 fps in Chapter 1, prison to the boat pursuit, RTX 5080 | — |
| 14 | 63.7 | [Victory Grimkaaah](https://github.com/Grimkaaah/Victory-Grimkaaah) | AMD | AMD only: 24.3 fps average in busy world scenes | — |
| 15 | 63.3 | [Neptune](https://github.com/Dolberthi/neptune-dolberth-nvidia-8gb) | NVIDIA 8 GB | Tuned for 8 GB NVIDIA, 6-core Ryzen, 16 GB RAM | — |
| 16 | 63.3 | [ruff-lab](https://github.com/LoverCatys/ruff-lab) | AMD 8 GB | Built for 8 GB cards and 16 GB RAM: 6 to 7 fps by scene at 720p upscaled, 30 to 39 min… | — |
| 17 | 63.1 | [VIniVIdiVIci](https://github.com/KomanderDG88/VIniVIdiVIci) | NVIDIA | 6.0 fps median in the first room on its defaults, 38 minutes in the world, RTX 4060 Ti | — |
| 18 | 62.9 | [AnyXboxX](https://github.com/LooperSalty/AnyXboxX) | NVIDIA | "About 24 fps median in the world in Full HD", 30 | ⚠️ On its new Vice-Tay base: reaches 3D at 0.5 to 3 fps in… |
| 19 | 62.6 | [VIctory_sm86](https://github.com/whiteh4cker-tr/VIctory_sm86) | NVIDIA | 27-30 fps in the opening on an RTX 3090 | — |
| 20 | 62.5 | [Kitty6](https://github.com/kitty-core/Kitty6) | NVIDIA | 20.5 fps median in the opening scene | — |
| 21 | 62.1 | [BRVIctory-ka1qux](https://github.com/ka1qux/BRVIctory-ka1qux) | AMD | 10.5 fps on Series X with its patches on | — |
| 22 | 62 | [victory-unified](https://github.com/Itsbatt/victory-unified) | NVIDIA 8 GB | "A unified fork" tuned for an RTX 3070 8 GB: 2 to 3 fps, about 58 minutes without a crash | — |
| 23 | 61.5 | [CGBJ (cool_fox)](https://github.com/DoubleGhosty/cool_fox) | NVIDIA | 25 fps average in gameplay on an NVIDIA card, relayed by PortWatch and unconfirmed | — |
| 24 | 61.5 | [Flamingo](https://github.com/realblastex/flamingo) | NVIDIA | 8.6 fps average over a 24-minute session, RTX 4070, unconfirmed | ⚠️ Runs at ~20 fps on average, but models are very dark and… |
| 25 | 61 | [ApexVI](https://github.com/KevinPratap/ApexVI) | NVIDIA 6-8 GB | Tuned for 6-8 GB laptop cards: a controllable character at 6.35 to 7.5 fps, 14-15 min… | — |
| 26 | 60.5 | [SlaktarnVI](https://github.com/Slaktarn700/SlaktarnVI) | AMD | Linux/Proton on an RX 9070 XT: 19-25 fps | — |
| 27 | 60.5 | [VI-BR-PORT](https://github.com/edsuuu/VI-BR-PORT) | NVIDIA 8 GB | 24 fps on a scripted prison walk and 21 fps in the opening cutscene, RTX 4060 Ti 8 GB | ❌ The game stops about 20 s in |
| 28 | 60.4 | [VIctory-NVIDIA-RT](https://github.com/Kemerd/VIctory-NVIDIA-RT) | NVIDIA | A faster VIctory-NVIDIA with ultrawide, Reflex, HDR10, DLSS and frame generation: 21.9… | — |
| 29 | 60 | [victory-sley-fork](https://github.com/Spaghetih/victory-sley-fork) | NVIDIA | 20.6 fps median at about 6,000 draws, 11-15 in dense night scenes | — |
| 30 | 59.8 | [victory-fork](https://github.com/fxceless/victory-fork) | NVIDIA | Its ray-tracing builder is now its default: 0.2 to 3 fps with ray-traced shadows and… | ✅ Intro at 30 fps, 3D scene ~1 fps |
| 31 | 59.6 | [VItoriaBr](https://github.com/uiliannn/VItoriaBr) | NVIDIA 8 GB | 4 to 6 fps on its current settings | ✅ 3D prison scene at 2 to 3 fps |
| 32 | 59.5 | [VIctory-Hydra](https://github.com/tolimanl/victory-hydra) | NVIDIA | ~18-20 fps at 1080p, RTX 3060 | — |
| 33 | 59.3 | [foxhound](https://github.com/readmet3xt-lab/foxhound) | NVIDIA | DLSS and frame generation: 30 fps cap on a 4070 SUPER, 16-21 in heavier scenes | — |
| 34 | 59.3 | [victory-scarlett-3070](https://github.com/Yaekai/victory-scarlett-3070) | NVIDIA 8 GB | Tuned for 8 GB NVIDIA | — |
| 35 | 59.1 | [VIscosity](https://github.com/FrancescoGelosoEterea/VIscosity) | NVIDIA | 17.9 fps median on an RTX 5070 Ti | — |
| 36 | 59 | [Mandy](https://github.com/projectleonida/mandy) | NVIDIA | Leonida's continuation: playable in the world at 12 to 13 fps, with the HUD, map and pause menu | — |
| 37 | 59 | [Victory-United](https://github.com/Fayberr/Victory-United) | Any | VIctory's best open pull requests on Vice-Tay's code: 8 to 12 fps in the open world with… | — |
| 38 | 58.8 | [VIctory-BigCheese](https://github.com/computermakemegta6/VIctory-BigCheese) | NVIDIA | 17 fps held in the open world, 16 GB RAM | — |
| 39 | 58.5 | [VIctory-LITE](https://github.com/vix0e0e/VIctory-LITE) | NVIDIA 4 GB | 13-20 fps in the prologue, 19-22 on the boat mission | — |
| 40 | 58.5 | [Victory-naixue2233](https://github.com/naixue2233/Victory-naixue2233) | NVIDIA | About 11 fps at the first boat mission on an RTX 4070 | — |
| 41 | 58 | [VIctory-AMD-4K](https://github.com/elaboratecl/victory-amd-4k) | AMD | 13.0-16.1 fps at native 4K, RX 6800 | — |
| 42 | 57.6 | [CBL VIctory](https://github.com/cocainebunnylaboratories/cblvictoryvi) | NVIDIA | Now on Vice-Tay's renderer: about 12-15 fps at Series X 1440p, RTX 5080 | — |
| 43 | 56.8 | [VKarameloVI](https://github.com/badDevelopper/VKarameloVI) | NVIDIA | 11 fps, or 21.8 with NVIDIA Smooth Motion | — |
| 44 | 56.8 | [VIctory-NVIDIA (pepezin-ai)](https://github.com/pepezin-ai/VIctory-NVIDIA) | NVIDIA | DLSS frame generation fixed: 11 fps rendered, 44 shown at 4x, RTX 5070 12 GB in the… | — |
| 45 | 56.6 | [skornuta/victory-fork](https://github.com/skornuta/victory-fork) | NVIDIA | ~9-12 fps in the open world | — |
| 46 | 56.5 | [victory-amd](https://github.com/myokex/victory-amd) | AMD | 2 to 3 fps in the world over 8-minute runs, RTX 4070 | — |
| 47 | 56.5 | [CCS03](https://github.com/JozefDeTremmerie/CCS03) | NVIDIA 8 GB | Lucia's prologue played to chapter 1 | — |
| 48 | 56.5 | [CTA](https://github.com/DesignWithTommy/ClaudeTheftAuto) | NVIDIA 8 GB | i95's own runtime for the RTX 4060: rewrites the game's hottest stores live; steady frame 95 to 105 ms (~10 fps) | — |
| 49 | 56.5 | [Dark-Victory](https://github.com/darksidexc/Dark-Victory-RX6600-I714700KF-VI) | AMD 8 GB | About 10 fps on an RX 6600 | — |
| 50 | 56.5 | [Leonida](https://github.com/ProjectLeonidaL/leonida) | Any | About 10 fps in the first rooms with its newer switches | — |
| 51 | 56.5 | [mangrove](https://github.com/Joaco9020/mangrove) | NVIDIA 8 GB | 2.5 fps in the prologue, RTX 3070 | — |
| 52 | 56.1 | [victory-nvidia-8gb](https://github.com/soldsawn/victory-nvidia-8gb) | NVIDIA 8 GB | Tuned for 8 GB NVIDIA | — |
| 53 | 55.8 | [rorodev6/victory](https://github.com/rorodev6/victory) | NVIDIA | 7-9 fps in the prologue | — |
| 54 | 55.7 | [cool-fox-12gb](https://github.com/miaohaixiang1-netizen/cool-fox-12gb) | NVIDIA 8-12 GB | cool_fox with VIctory-NVIDIA's 8 GB patches | — |
| 55 | 55.6 | [ScarlettPort](https://github.com/ZigZagBoi1/ScarlettPort) | NVIDIA | About 7 to 8 fps in the world with a warm cache on 0.5.2, Series S, RTX 3060 | ✅ 3D at 2 to 3 fps |
| 56 | 55.5 | [mandy-port](https://github.com/bolado-dev/mandy-port) | NVIDIA 8 GB | ~7 fps in the world, aiming at 15 | — |
| 57 | 55.4 | [victory-4gb](https://github.com/Dathinel/victory-4gb) | NVIDIA 4 GB | 6.5 to 7 fps in open-world free play on a 4 GB RTX 3050 Laptop | — |
| 58 | 55 | [LNX-victory](https://github.com/Linox-IDK/LNX-victory) | NVIDIA 8 GB | Prison yard at about 4 fps | — |
| 59 | 54.5 | [victory-amd r13](https://github.com/tinygusie2/victory-amd) | AMD | About 3-5 fps in the open world on an RX 9060 XT | — |
| 60 | 54.5 | [M1LK](https://github.com/3cvil/M1LK) | NVIDIA 8 GB | Tuned for 8 GB VRAM and 16 GB RAM: playable at about 4 fps on an RTX 4060 Laptop 8 GB | — |
| 61 | 54.1 | [darkworldhm](https://github.com/darkworldhm/victory-fork-darkworldhm) | NVIDIA | A snapshot of victory-fork with its own changes: 2 to 4 fps in the world, RTX 4070 | — |
| 62 | 53.1 | [VIctory-NVIDIA-LowStutter](https://github.com/tiadiff/VIctory-NVIDIA-LowStutter) | NVIDIA | No fps reported; fewer pipeline stalls | — |
| 63 | 53.1 | [MiniVI](https://github.com/Androtim/MiniVI) | Any 8 GB | Built for 8 GB VRAM and 16-32 GB RAM, with an F3 FPS/VRAM overlay | — |
| 64 | 53.1 | [VIPort](https://github.com/DesertGamer/VIPort) | any | Boots and runs menus at 30 fps; world not yet playable | — |
| 65 | 50.3 | [VIctory](https://github.com/Marvisak/VIctory) | Any | The opening mission at about 1 fps on an RX 6700 XT, "not playable" | — |
| 66 | 35 | [XCL](https://github.com/xcl-dev/xcl) | Any | Compatibility layer: 30 fps in the menus, ~5 fps in the 3D world with its NVIDIA branch (RTX 5090), not playable yet | — |
| – | – | VIctory-Belobug-AMD (removed) | AMD | 5 to 8 fps over a 15-minute drive, RX 7900 XTX | ❌ Removed from the loader: the author's GitHub account was… |
| – | – | VIctory-7900XTX (removed) | AMD | 13 to 17 fps in a city street at Series X 1440p, RX 7900 XTX | ❌ Removed from the loader: the author's GitHub account was… |
| – | – | Vertex (removed) | NVIDIA | "Not playable yet." 6.9 fps median in a 25-minute run, RTX 5070 | ❌ Removed from the loader: the author deleted or hid the… |
| – | – | Scarlett fork (removed) | NVIDIA | cool_fox plus a local fork's work: 16 to 21 fps in hand-played heavy scenes | ❌ Removed from the loader: the author deleted or hid the… |
| – | – | POLSKAGUROM (removed) | NVIDIA | 19.4 fps average on an RTX 5060 Ti 16 GB | ❌ Removed from the loader: the author deleted or hid the… |

<details><summary><b>Full reports</b> (what each author says, and our runs in full)</summary>

| Runtime | Based on | Its author reports in full | Here in full |
|---|---|---|---|
| [VIctorious](https://github.com/randomguy12000/VIctorious) | VIctory-7900XTX | Driving in the world at about 38 ms a frame (~26 fps), Series S, RX 9070 XT, Ryzen 7 9800X3D, 32 GB RAM | Not run yet |
| [DanziLoader-Cpp](https://github.com/Veria70/DanziLoader-Cpp) | VIctory (C++20 port) | Prison, Chapter 1 and the boat pursuit at Series S and 50% render scale; 16+ minutes in one process; ~11.0 fps in a Chapter 1 sample; water and frame pacing faults; RTX 5080 | Not run yet |
| [mandy-port](https://github.com/bolado-dev/mandy-port) | VIctory | ~7 fps in the world (135-150 ms a frame median); target 15 fps; tuned for 8 GB cards | Not run yet |
| [VIctory-NVIDIA-LowStutter](https://github.com/tiadiff/VIctory-NVIDIA-LowStutter) | VIctory-NVIDIA | A low-stutter fork for RTX 20-50: slow pipelines finish in the background instead of repeating 500 ms frame stalls; no fps reported | Not run yet |
| [VIctory-AMD-4K](https://github.com/elaboratecl/victory-amd-4k) | VIctory | 13.0-16.1 fps at native 4K Series X, ray tracing off, Ryzen 5 5600, RX 6800 16 GB, 16 GB RAM | Not run yet |
| [VIctory-Hydra](https://github.com/tolimanl/victory-hydra) | VIctory | ~18-20 base fps at 1080p (37-40 with FSR 3 frame generation 2x), RTX 3060, Ryzen 5 5600X; "not ready for full playthroughs" | Not run yet |
| [El1xr](https://github.com/kylepelham/El1xr) | VIctory | 23.1-25.7 fps at 4K on an RTX 5090, 27-28 s from launch to the world; first two missions completed | Not run yet |
| [VIctory-BigCheese](https://github.com/computermakemegta6/VIctory-BigCheese) | VIctory | For 16 GB RAM: 17 fps held in the open world instead of ~10; prologue 13-20 fps | Not run yet |
| [VIctory-LITE](https://github.com/vix0e0e/VIctory-LITE) | VIctory | For 4 GB cards: prologue 13-20 fps, boat mission 19-22, street chase 6-10 | Not run yet |
| [skornuta/victory-fork](https://github.com/skornuta/victory-fork) | VIctory | Menus 30 fps, open world ~9-12 fps at ~4,900 draws | Not run yet |
| [mangrove](https://github.com/Joaco9020/mangrove) | VIctory | For 8 GB cards: player control in the prologue at 2.5 fps, RTX 3070; past the 8-minute crash | Not run yet |
| [rorodev6/victory](https://github.com/rorodev6/victory) | VIctory | Prologue 7-9 fps, up from ~3; fps counter overlay | Not run yet |
| [VIPort](https://github.com/DesertGamer/VIPort) | its own | Boots and runs menus at 30 fps; world not yet playable | Not run yet |
| [CGBJ (cool_fox)](https://github.com/DoubleGhosty/cool_fox) | VIctory | 25 fps average in gameplay on an NVIDIA card, relayed by PortWatch and unconfirmed; an outside build reports a 22.4 fps median on the boat, also unverified; zero-copy buffers now on by default; the original repo is no longer on GitHub | Not run yet |
| [VIctory-NVIDIA](https://github.com/trustno1-coder/VIctory-NVIDIA) | VIctory-7900XTX | CGBJ renderer with DLSS: 26 to 30 fps in the opening rooms and the first boat ride, RTX 5070, generated frames off and not counted (DLSS needs NVIDIA files i95 doesn't fetch; without them it runs Series X at native 1440p, uncapped) | Not run yet |
| [DexNom RayTime](https://github.com/DexNom/VIctory) | VIctory | 1 fps with ray tracing on, 3.5 to 4 without, RX 7900 XTX | Not run yet |
| [Vice-Tay](https://github.com/TaynanGT/Vice-Tay) | VIctory | The tracker's leader and playable by its own report: a 21.1 fps median over a 30-minute session played by hand, Series S profile, RTX 4090 | ✅ The `4060` preset: 38 min in 3D at 10.3 fps on average (30 in quiet rooms, under 1 in busy scenes), then out of graphics memory. Rough picture. Plain Series S: 3 to 4 fps (paged to disk); `8gb`: 5 fps; Series X (its default) crashes in the first cutscene on 8 GB |
| [Kelvaroq](https://github.com/stellarr-studio/kelvaroq) | VIctory, victory-fork | ~10 fps in typical views (7 to 8 in the heaviest) at 4K on an RTX 5080; a tester's 36 min session without a crash; its unreleased direct memory path needs 22 ms of GPU work an open-world frame and holds 30 fps on loading screens | Testing |
| [Vilocity](https://github.com/voidopcode/vilocity) | cool_fox (was its own layer) | 33 fps median through the opening and 25 to 29 in the open world, 12 minutes of each unattended without a crash or driver timeout, Series S, RTX 5080; "not playable yet" | Not run yet |
| [VIctory-RDNA](https://github.com/XaviFortes/VIctory-RDNA) | VIctory | 19.1 to 23.1 fps medians at Series X 4K in one capture under 10 minutes, RX 7900 XT; about 40 minutes by hand on earlier builds | Not run yet |
| [KLBF Lab](https://github.com/PastiiJay/klbf-lab) | VIctory, victory-fork | 14.0 fps average over 37 minutes of play, player in control through the prologue (77 min on an earlier build), RX 9070 XT | Testing again. Its intro crash was i95 leaving TuxBox DLLs in the game folder (fixed). The intro video flickers |
| [victory-8gb](https://github.com/CuteBooty/victory-8gb) | VIctory | First Chapter I mission played, a 5 h 45 min session without a crash, RTX 3070 8 GB | Not run yet |
| [playablegames-peppa](https://github.com/Peppacodes/playablegames-peppa) | VIctory | 23.5 fps over 12 minutes in a house interior, RTX 5080 16 GB | Not run yet |
| [mavis-9070xt](https://github.com/mavis140/mavis-9070xt) | VIctory-7900XTX | 18.9 fps median over a 27-minute session, Series X at 1440p, ray tracing off, RX 9070 XT 16 GB | Not run yet |
| [Victory Grimkaaah](https://github.com/Grimkaaah/Victory-Grimkaaah) | VIctory | AMD only: 24.3 fps average in busy world scenes (ray tracing off), 13.9 with it on, 3-9 min sessions, RX 7900 XTX; on the tracker's watch list | Not run yet |
| [Neptune](https://github.com/Dolberthi/neptune-dolberth-nvidia-8gb) | VIctory | Tuned for 8 GB NVIDIA, 6-core Ryzen, 16 GB RAM; no fps figure yet | Not run yet |
| [ruff-lab](https://github.com/LoverCatys/ruff-lab) | KLBF Lab | Built for 8 GB cards and 16 GB RAM: 6 to 7 fps by scene at 720p upscaled, 30 to 39 min runs without a crash, RX 6600 XT | Not run yet |
| [VIniVIdiVIci](https://github.com/KomanderDG88/VIniVIdiVIci) | victory-fork | 6.0 fps median in the first room on its defaults, 38 minutes in the world, RTX 4060 Ti | Not run yet |
| [AnyXboxX](https://github.com/LooperSalty/AnyXboxX) | Vice-Tay (was Neptune) | "About 24 fps median in the world in Full HD", 30 (the game's cap) in lighter scenes, Series X, RTX 4070 SUPER | ⚠️ On its new Vice-Tay base: reaches 3D at 0.5 to 3 fps in two 2 to 3 min runs, no crash; RAM full (15.5 of 15.8 GB), so it pages. Its earlier Neptune build ran 10 min at ~14 fps |
| [VIctory_sm86](https://github.com/whiteh4cker-tr/VIctory_sm86) | VIctory | 27-30 fps in the opening on an RTX 3090 (1440p, DLSS Quality) | Not run yet |
| [Kitty6](https://github.com/kitty-core/Kitty6) | VIctory-7900XTX | 20.5 fps median in the opening scene (Series S) on an RTX 4070, ~5 min runs; not on the tracker | Not run yet |
| [BRVIctory-ka1qux](https://github.com/ka1qux/BRVIctory-ka1qux) | victory-fork | 10.5 fps on Series X with its patches on (no run length on this base; 4 to 4.7 fps over 25 min on the earlier one), RX 7800 XT | Not run yet |
| [victory-unified](https://github.com/Itsbatt/victory-unified) | VIctory | "A unified fork" tuned for an RTX 3070 8 GB: 2 to 3 fps, about 58 minutes without a crash | Not run yet |
| [Flamingo](https://github.com/realblastex/flamingo) | victory-fork | 8.6 fps average over a 24-minute session, RTX 4070, unconfirmed | ⚠️ Runs at ~20 fps on average, but models are very dark and the screen flashes black and white |
| [ApexVI](https://github.com/KevinPratap/ApexVI) | VIctory | Tuned for 6-8 GB laptop cards: a controllable character at 6.35 to 7.5 fps, 14-15 min runs without a crash, RTX 4050 6 GB; on the tracker's watch list | Not run yet |
| [SlaktarnVI](https://github.com/Slaktarn700/SlaktarnVI) | VIctory | Linux/Proton on an RX 9070 XT: 19-25 fps | Not run yet |
| [VI-BR-PORT](https://github.com/edsuuu/VI-BR-PORT) | victory-fork | 24 fps on a scripted prison walk and 21 fps in the opening cutscene, RTX 4060 Ti 8 GB; the author now calls it playable, with known picture problems | ❌ The game stops about 20 s in |
| [VIctory-NVIDIA-RT](https://github.com/Kemerd/VIctory-NVIDIA-RT) | VIctory-NVIDIA | A faster VIctory-NVIDIA with ultrawide, Reflex, HDR10, DLSS and frame generation: 21.9 fps median, 31.9 best in the open world, RTX 5090 at 1440p with DLSS Quality, no frame generation; not on the tracker | Not run yet |
| [victory-sley-fork](https://github.com/Spaghetih/victory-sley-fork) | VIctory | 20.6 fps median at about 6,000 draws, 11-15 in dense night scenes; FSR 3.1 on NVIDIA | Not run yet |
| [victory-fork](https://github.com/fxceless/victory-fork) | VIctory | Its ray-tracing builder is now its default: 0.2 to 3 fps with ray-traced shadows and light, 5 to 9 min runs ending in a crash in the game's script code, RTX 4070; without it about 11 fps at the first boat mission | ✅ Intro at 30 fps, 3D scene ~1 fps |
| [VItoriaBr](https://github.com/uiliannn/VItoriaBr) | VIctory, victory-fork | 4 to 6 fps on its current settings | ✅ 3D prison scene at 2 to 3 fps |
| [foxhound](https://github.com/readmet3xt-lab/foxhound) | VIctory | DLSS and frame generation: 30 fps cap on a 4070 SUPER, 16-21 in heavier scenes | Not run yet |
| [victory-scarlett-3070](https://github.com/Yaekai/victory-scarlett-3070) | Scarlett fork | Tuned for 8 GB NVIDIA (RTX 3070): 16-21 fps in heavy scenes | Not run yet |
| [VIscosity](https://github.com/FrancescoGelosoEterea/VIscosity) | victory-fork | 17.9 fps median on an RTX 5070 Ti (Series S) | Not run yet |
| [Victory-United](https://github.com/Fayberr/Victory-United) | VIctory | VIctory's best open pull requests on Vice-Tay's code: 8 to 12 fps in the open world with scripted movement, three 7-minute runs, Series S; 'not playable yet' | Not run yet |
| [Victory-naixue2233](https://github.com/naixue2233/Victory-naixue2233) | victory-fork | About 11 fps at the first boat mission on an RTX 4070 (with its measured switches) | Not run yet |
| [CBL VIctory](https://github.com/cocainebunnylaboratories/cblvictoryvi) | Vice-Tay | Now on Vice-Tay's renderer: about 12-15 fps at Series X 1440p, RTX 5080 | Not run yet |
| [VKarameloVI](https://github.com/badDevelopper/VKarameloVI) | VIctory-NVIDIA | 11 fps, or 21.8 with NVIDIA Smooth Motion (frame doubling), on its author's card; not on the tracker | Not run yet |
| [VIctory-NVIDIA (pepezin-ai)](https://github.com/pepezin-ai/VIctory-NVIDIA) | VIctory-NVIDIA | DLSS frame generation fixed: 11 fps rendered, 44 shown at 4x, RTX 5070 12 GB in the prologue; not on the tracker | Not run yet |
| [victory-amd](https://github.com/myokex/victory-amd) | victory-fork | 2 to 3 fps in the world over 8-minute runs, RTX 4070 | Not run yet |
| [CCS03](https://github.com/JozefDeTremmerie/CCS03) | victory-fork | Lucia's prologue played to chapter 1; 3 to 7.8 fps in the prison and on the water, RTX 4060 Laptop 8 GB | Not run yet |
| [Dark-Victory](https://github.com/darksidexc/Dark-Victory-RX6600-I714700KF-VI) | VIctory | About 10 fps on an RX 6600 | Not run yet |
| [Leonida](https://github.com/ProjectLeonidaL/leonida) | own (static recompilation) | About 10 fps in the first rooms with its newer switches; not a VIctory fork | Not run yet |
| [XCL](https://github.com/xcl-dev/xcl) | own (compatibility layer) | Starts in ~4 s, a steady 30 fps in the menus; with its NVIDIA integration branch it reaches the 3D world at ~5 fps on an RTX 5090; the world's picture is incomplete and it does not keep running there long | Not run yet |
| [CTA](https://github.com/DesignWithTommy/ClaudeTheftAuto) | own (pieces) | Our own Rust runtime built from pieces of the others: page-fault stores rewritten live (299 of 318 sites), fills done on the card; steady frame 95 to 105 ms | Built from the local checkout |
| [Mandy](https://github.com/projectleonida/mandy) | Leonida | Its D3D12 runtime is playable in the world: 12 to 13 fps standing at the save point (75 to 82 ms median frame), with the HUD, map and pause menu; its Vulkan runtime is not playable yet | Not run yet |
| [victory-nvidia-8gb](https://github.com/soldsawn/victory-nvidia-8gb) | VIctory | Tuned for 8 GB NVIDIA (Wave32): 8 to 10 fps on an RTX 4060 Laptop 8 GB at 1080p; not on the tracker | Not run yet |
| [cool-fox-12gb](https://github.com/miaohaixiang1-netizen/cool-fox-12gb) | CGBJ | cool_fox with VIctory-NVIDIA's 8 GB patches; 7.7 fps at half render scale, 2–3 fps native; not on the tracker | Not run yet |
| [ScarlettPort](https://github.com/ZigZagBoi1/ScarlettPort) | victory-fork | About 7 to 8 fps in the world with a warm cache on 0.5.2, Series S, RTX 3060 | ✅ 3D at 2 to 3 fps |
| [victory-4gb](https://github.com/Dathinel/victory-4gb) | victory-fork | 6.5 to 7 fps in open-world free play on a 4 GB RTX 3050 Laptop | Not run yet |
| [LNX-victory](https://github.com/Linox-IDK/LNX-victory) | VIctory | Prison yard at about 4 fps (Series X), 7-8 fps with Series S + FSR, RTX 5060 Ti 8 GB | Not run yet |
| [victory-amd r13](https://github.com/tinygusie2/victory-amd) | victory-fork | About 3-5 fps in the open world on an RX 9060 XT | Not run yet |
| [M1LK](https://github.com/3cvil/M1LK) | VIctory | Tuned for 8 GB VRAM and 16 GB RAM: playable at about 4 fps on an RTX 4060 Laptop 8 GB | Not run yet |
| [darkworldhm](https://github.com/darkworldhm/victory-fork-darkworldhm) | victory-fork | A snapshot of victory-fork with its own changes: 2 to 4 fps in the world, RTX 4070 | Not run yet |
| [MiniVI](https://github.com/Androtim/MiniVI) | cool_fox | Built for 8 GB VRAM and 16-32 GB RAM, with an F3 FPS/VRAM overlay; 4 GB and AMD modes | Not run yet |
| [VIctory](https://github.com/Marvisak/VIctory) | (the original) | The opening mission at about 1 fps on an RX 6700 XT, "not playable" | Not run yet |
| VIctory-Belobug-AMD (removed) | VIctory | 5 to 8 fps over a 15-minute drive, RX 7900 XTX | ❌ Removed from the loader: the author's GitHub account was deleted, so the source can no longer be downloaded |
| VIctory-7900XTX (removed) | VIctory | 13 to 17 fps in a city street at Series X 1440p, RX 7900 XTX | ❌ Removed from the loader: the author's GitHub account was deleted, so the source can no longer be downloaded |
| Vertex (removed) | VIctory | "Not playable yet." 6.9 fps median in a 25-minute run, RTX 5070 | ❌ Removed from the loader: the author deleted or hid the repository, so the source can no longer be downloaded |
| Scarlett fork (removed) | CGBJ | cool_fox plus a local fork's work: 16 to 21 fps in hand-played heavy scenes (prison yard, open world), Series X at 1440p, ray tracing off, RTX 4080 SUPER | ❌ Removed from the loader: the author deleted or hid the repository, so the source can no longer be downloaded |
| POLSKAGUROM (removed) | VIctory | 19.4 fps average on an RTX 5060 Ti 16 GB (Series S) | ❌ Removed from the loader: the author deleted or hid the repository, so the source can no longer be downloaded |

</details>


**Not in i95:** ztrx (#6) and the original CGBJ (#12, run here through DoubleGhosty's cool_fox) are gone from GitHub, and so are PiloteBox, victory-perf and our own i95-Victory, so they were taken out (their last source is kept offline). RAZDA4A-XEN was taken out because it ships only a prebuilt exe, with no source to build. omonkey (#10), suri (#11) and Doomsday (#20) are private builds with no public code. VIctory-macOS-halfwave50 is macOS only (2.9-4.5 fps on a MacBook Pro).

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
