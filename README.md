# i95: GTA 6 PC Launcher for every GTA VI Xbox Series runtime (VIctory, Kelvaroq, VItoriaBr and more)

![i95: GTA 6 Xbox Series launcher for Windows PC](assets/banner.svg)

Try them all from one window.

i95 is a Windows launcher for the projects trying to run the Xbox Series version of GTA VI on PC. It doesn't include or download the game, and there are no game files, keys or protection bypasses in it.

**It builds every runtime from source on your PC.** Each runtime's source code is pinned to one commit you can read on GitHub, and i95 compiles it locally with Rust, so you never run an .exe someone else built. That's safer than downloading prebuilt runtime exes, which is how fake "GTA 6 PC" malware spreads.

## What it does

- **One list of runtimes**, ranked like the race tracker, each with its author's own default launch settings.
- **Stays current without an update**: at start it reads the runtime list from [catalog.txt](catalog.txt) here. Every 15 minutes each runtime is moved to its author's newest commit (scripts/bump-upstreams.sh, run by hand, no bot), and any commit that changes the author's launch settings is checked by hand before it goes in. Every launcher picks up the list on its next start. No new release is needed. Offline it uses the list it shipped with.
- **Builds and rebuilds** a runtime only when its source changed, and keeps each runtime's shader cache when it does (a fork that keyed its cache on its own exe, KLBF Lab, is patched to key on its shader translator instead; `NEONBRIDGE_SHADER_KEY_PATCH=0` turns that off).
- **Prepares shaders** before the game starts, with a loading screen that shows real progress. Your own picture at `%LOCALAPPDATA%\6ixpack\splash.bmp` shows on the loading screen.
- **Checks your PC first**: graphics card features (D3D12, Shader Model 6.6, mesh shaders), RAM and free disk, and names what's missing.
- **Presets** per runtime, such as Series S, 8 GB cards, 16 GB laptops, DLSS to 4K, 16 GB of RAM (Vice-Tay's `4060` preset: 38 min at ~10 fps on an RTX 4060 here) and crash diagnostics, plus any extra setting a runtime reads.
- **In game**: one fps panel for every runtime, Vice-Tay's HUD (fps, frame-time graph, GPU and memory, CPU cores; F3 hides it), in place of each runtime's own; a controls overlay; the display kept awake.
- **After a run**: explains why it ended by reading the log, writes a short timeline, and can file an issue report.
- **Asks before downloading** a runtime's source or an optional extra. The one thing it fetches without asking is the small runtime list, catalog.txt.

**None of these runtimes is confirmed playable yet.** The tracker's bar for playable is 20 fps and 30 minutes without a crash. Only Vice-Tay reports clearing it (a 30-minute session at a 21.1 fps median on an RTX 4090), and nobody outside the project has checked it.

## Standings

Ranks come from vuyp's [GTA VI PC race tracker](https://vuyp.github.io/gta6-pc-race/) ([repo](https://github.com/vuyp/gta6-pc-race)). i95 has 38 runtimes, and the launcher lists them in one list in that order.

"Its author reports" is what each project says about itself, in its own words where we could. "Here" is our own PC: i5-14400F, RTX 4060 8 GB, 16 GB RAM, Windows 11. Most authors test on faster cards, so expect less on yours if it's like ours.

| Rank | Runtime | Based on | Its author reports | Here |
|---:|---|---|---|---|
| 1 | [Vice-Tay](https://github.com/TaynanGT/Vice-Tay) | VIctory | The tracker's leader and playable by its own report: a 21.1 fps median over a 30-minute session played by hand, Series S profile, RTX 4090 | ✅ The 4060 preset on an RTX 4060 8 GB with 16 GB of RAM: the latest run lasted 38 min in 3D at 10.3 fps on average (0.1 to 32; 30 in quiet rooms, under 1 in busy scenes), then the card ran out of memory and the device was lost. The picture looks rough (Series S plus low budgets). Plain Series S got 3 to 4 fps, because it used 17 GB of RAM and paged to disk; 8gb got 5 fps. Series X (its default) runs out of graphics memory on an 8 GB card in the first cutscene and crashes; use series_s or 8gb |
| 2 | [victory-fork](https://github.com/fxceless/victory-fork) | VIctory | Its ray-tracing builder is now its default: 0.2 to 3 fps with ray-traced shadows and light, 5 to 9 min runs ending in a crash in the game's script code, RTX 4070; without it about 11 fps at the first boat mission | ✅ Intro at 30 fps, 3D scene ~1 fps |
| 3 | [AnyXboxX](https://github.com/LooperSalty/AnyXboxX) | Vice-Tay (was Neptune) | "About 24 fps median in the world in Full HD", 30 (the game's cap) in lighter scenes, Series X, RTX 4070 SUPER | ⚠️ On its new Vice-Tay base: reaches 3D at 0.5 to 3 fps in two 2 to 3 min runs, no crash; RAM full (15.5 of 15.8 GB), so it pages. Its earlier Neptune build ran 10 min at ~14 fps |
| 4 | [KLBF Lab](https://github.com/PastiiJay/klbf-lab) | VIctory, victory-fork | 14.0 fps average over 37 minutes of play, player in control through the prologue (77 min on an earlier build), RX 9070 XT | Testing again. Its intro crash was i95 leaving TuxBox DLLs in the game folder (fixed). The intro video flickers |
| 5 | [VIctory-Belobug-AMD](https://github.com/Belobug/VIctory-Belobug-AMD) | VIctory | 5 to 8 fps over a 15-minute drive, less while driving, RX 7900 XTX | ❌ |
| 7 | [VItoriaBr](https://github.com/uiliannn/VItoriaBr) | VIctory, victory-fork | 4 to 6 fps on its current settings | ✅ 3D prison scene at 2 to 3 fps |
| 9 | [ScarlettPort](https://github.com/ZigZagBoi1/ScarlettPort) | victory-fork | 3.4 fps in the world on 0.5.0, RTX 3060 | ✅ 3D at 2 to 3 fps |
| 12 | CGBJ ([cool_fox](https://github.com/DoubleGhosty/cool_fox), a continuation) | VIctory | 25 fps average in gameplay on an NVIDIA card, relayed by PortWatch and unconfirmed; an outside build reports a 22.4 fps median on the boat, also unverified; zero-copy buffers now on by default; the original repo is no longer on GitHub | Not run yet |
| 13 | [victory-amd](https://github.com/myokex/victory-amd) | victory-fork | 2 to 3 fps in the world over 8-minute runs, RTX 4070 | Not run yet |
| 14 | [VIctory](https://github.com/Marvisak/VIctory) | (the original) | The opening mission at about 1 fps on an RX 6700 XT, "not playable" | Not run yet |
| 15 | [Flamingo](https://github.com/realblastex/flamingo) | victory-fork | 8.6 fps average over a 24-minute session, RTX 4070, unconfirmed | ⚠️ Runs at ~20 fps on average, but models are very dark and the screen flashes black and white |
| 16 | [BRVIctory-ka1qux](https://github.com/ka1qux/BRVIctory-ka1qux) | victory-fork | 10.5 fps on Series X with its patches on (no run length on this base; 4 to 4.7 fps over 25 min on the earlier one), RX 7800 XT | Not run yet |
| 17 | [DexNom RayTime](https://github.com/DexNom/VIctory) | VIctory | 1 fps with ray tracing on, 3.5 to 4 without, RX 7900 XTX | Not run yet |
| 18 | [Vilocity](https://github.com/voidopcode/vilocity) | cool_fox (was its own layer) | 24 to 29 fps over a scripted walk and run through the day scene (6,300 to 8,700 draws), Series S, RTX 5080; driver timeout about 7 minutes in, "not playable yet" | Not run yet |
| 19 | [CBL VIctory](https://github.com/cocainebunnylaboratories/cblvictoryvi) | VIctory, victory-fork | A 65 min session: 11.4 fps on the boat, 4.3 to 4.7 in the city, RTX 5080 | Not run yet |
| 21 | [Kelvaroq](https://github.com/stellarr-studio/kelvaroq) | VIctory, victory-fork | ~10 fps in typical views on an RTX 5080; a tester's 36 min session without a crash; shaders now translated once (heavy frames ~18 s to ~60 ms) | Testing |
| 22 | [victory-unified](https://github.com/Itsbatt/victory-unified) | VIctory | "A unified fork" tuned for an RTX 3070 8 GB: 2 to 3 fps, about 58 minutes without a crash | Not run yet |
| 23 | [VIniVIdiVIci](https://github.com/KomanderDG88/VIniVIdiVIci) | victory-fork | 6.0 fps median in the first room on its defaults, 38 minutes in the world, RTX 4060 Ti | Not run yet |
| 24 | [VI-BR-PORT](https://github.com/edsuuu/VI-BR-PORT) | victory-fork | 24 fps on a scripted prison walk and 21 fps in the opening cutscene, RTX 4060 Ti 8 GB; the author now calls it playable, with known picture problems | ❌ The game stops about 20 s in |
| 25 | [ruff-lab](https://github.com/LoverCatys/ruff-lab) | KLBF Lab | Built for 8 GB cards and 16 GB RAM: 6 to 7 fps by scene at 720p upscaled, 30 to 39 min runs without a crash, RX 6600 XT | Not run yet |
| 26 | [darkworldhm](https://github.com/darkworldhm/victory-fork-darkworldhm) | victory-fork | A snapshot of victory-fork with its own changes: 2 to 4 fps in the world, RTX 4070 | Not run yet |
| 27 | [VIctory-7900XTX](https://github.com/avatarmanz/VIctory-7900XTX) | VIctory | 13 to 17 fps in a city street at Series X 1440p, 16 to 23 with its patches off, RX 7900 XTX | ⚠️ Past the intro at ~15 fps, then out of graphics memory |
| 28 | [Vertex](https://github.com/proneonn/Vertex) | VIctory | "Not playable yet." 6.9 fps median in a 25-minute run, RTX 5070 | Not run yet |
| 29 | [VIctory-NVIDIA](https://github.com/trustno1-coder/VIctory-NVIDIA) | VIctory-7900XTX | CGBJ renderer with DLSS: 26 to 30 fps in the opening rooms and the first boat ride, RTX 5070, generated frames off and not counted (DLSS needs NVIDIA files i95 doesn't fetch; without them it runs Series X at native 1440p, uncapped) | Not run yet |
| 30 | [Victory-United](https://github.com/Fayberr/Victory-United) | VIctory | VIctory"s best open pull requests on Vice-Tay"s code: 8.5 fps median over 34.7 minutes played by hand on its earlier tree, RTX 3080 Ti | Not run yet |
| 31 | [VIctory-RDNA](https://github.com/XaviFortes/VIctory-RDNA) | VIctory | 19.1 to 23.1 fps medians at Series X 4K in one capture under 10 minutes, RX 7900 XT; about 40 minutes by hand on earlier builds | Not run yet |
| 32 | [Scarlett fork](https://github.com/op-lite/victory-scarlett) | CGBJ | cool_fox plus a local fork's work: 16 to 21 fps in hand-played heavy scenes (prison yard, open world), Series X at 1440p, ray tracing off, RTX 4080 SUPER | Not run yet |
| – | [Kitty6](https://github.com/kitty-core/Kitty6) | VIctory-7900XTX | 20.5 fps median in the opening scene (Series S) on an RTX 4070, ~5 min runs; not on the tracker | Not run yet |
| – | [VKarameloVI](https://github.com/badDevelopper/VKarameloVI) | VIctory-NVIDIA | 11 fps, or 21.8 with NVIDIA Smooth Motion (frame doubling), on its author's card; not on the tracker | Not run yet |
| – | [cool-fox-12gb](https://github.com/miaohaixiang1-netizen/cool-fox-12gb) | CGBJ | cool_fox with VIctory-NVIDIA's 8 GB patches; 7.7 fps at half render scale, 2–3 fps native; not on the tracker | Not run yet |
| – | [ApexVI](https://github.com/KevinPratap/ApexVI) | VIctory | Tuned for 6-8 GB laptop cards: a controllable character at 6.35 to 7.5 fps, 14-15 min runs without a crash, RTX 4050 6 GB; on the tracker's watch list | Not run yet |
| – | [Victory Grimkaaah](https://github.com/Grimkaaah/Victory-Grimkaaah) | VIctory | AMD only: 24.3 fps average in busy world scenes (ray tracing off), 13.9 with it on, 3-9 min sessions, RX 7900 XTX; on the tracker's watch list | Not run yet |
| – | [PiloteBox](https://github.com/PiloteCode/PiloteBox) | VIctory | For NVIDIA: 10.5 to 12.5 fps in the prologue (Series X), eleven 4-5 min runs without a crash, RTX 5090; on the tracker's watch list | Not run yet |
| 33 | [mavis-9070xt](https://github.com/mavis140/mavis-9070xt) | VIctory-7900XTX | 18.9 fps median over a 27-minute session, Series X at 1440p, ray tracing off, RX 9070 XT 16 GB | Not run yet |
| – | [PercyScarlett](https://github.com/rpatty-dev/PercyScarlett) | VIctory | Lit opening prison scene on Series S, 2-3 fps on an RTX 2060 SUPER; early, from PortWatch's watch list | Not run yet |
| – | [Five Star Recomp](https://github.com/ThatRecompDev/five-star-recomp) | own (C#) | Intro and opening cutscene at 30 fps, world surfaces still black; needs the .NET SDK; early | Not run yet |
| – | [GTA6-BootTest](https://github.com/OnlyMoisties/GTA6-BootTest) | own (C) | Boot harness: frame loop and first UI screen; needs Visual Studio 2022 C tools; early | Not run yet |
| – | i95-Victory (ours) | Kelvaroq | Our own fork: Kelvaroq plus the 8 GB and skin fixes from VI-BR-PORT, soldsawn and VItoriaBr | Not run yet |
| – | [xbox-emulator](https://github.com/omonkeyz/xbox-emulator) | – | Rust runtime that runs dumped Xbox Series builds as normal Windows processes, continuing the TomBox project (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [xcl](https://github.com/xcl-dev/xcl) | – | Wine-style layer running game code natively (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [ScarlettX](https://github.com/Freminet/ScarlettX) | – | Clean-room high-level emulator of the Scarlett OS and GDK runtime (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [TuxBox](https://github.com/brcly/TuxBox) | – | Native-first layer for unpacked Xbox Series games, tested mostly on Linux through Proton (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [OpenScarlett](https://github.com/californiaAlmonds/OpenScarlett) | – | MIT-licensed C++20 translation layer targeting D3D12 and Vulkan (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [scarlett-layer](https://github.com/jadeowaowa/scarlett-layer) | – | Loader plus injected hook DLL that redirects console-only calls to Windows shims, with its own d3d12_xs wrapper and shader lifter (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [Xbox2Windows](https://github.com/R0Y00/Xbox2Windows) | – | Static recompilation research turning Xbox Series code and shaders into Windows C++ and DXIL, with over 50 contributors (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [keylime](https://github.com/ethan00013/keylime) | – | A C++20 runtime written from scratch, with Vulkan graphics and its own RDNA2-to-SPIR-V translator (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [xvalkyrie](https://github.com/sammwyy/xvalkyrie) | – | Wine-style C++ layer for Xbox One and Series titles with native code and RDNA2-to-HLSL shaders (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [GTAVI-Recomp](https://github.com/JamesCicada/GTAVI-Recomp) | – | Targets the PS5 build through AnyPS5 and will not work from leaked files, so game research is blocked (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [anyxboxseries](https://github.com/edupode/anyxboxseries) | – | GTA VI-only translation layer with a PE mapper borrowed from Xbox2Windows, an early scalar shader lifter and a sample game that tests presentation and input (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [SCARAB](https://github.com/p-portela/SCARAB) | – | Found that the console d3d12_xs uses the same 28 interface IDs as PC D3D12, so standard calls can be forwarded and only the console extensions need building (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [AnyBox](https://github.com/Dark3581/AnyBox) | – | Relinks GDK executables against reimplemented system libraries, AnyPS5-style (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [GrandTheftXbox](https://github.com/zohaiblazuli/GrandTheftXbox) | – | Wine-style layer planned for Series and One titles (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [NativeX-Runtime](https://github.com/ProhorovIlja/NativeX-Runtime) | – | Clean-room test programs for how Game Core software meets Windows, covering loading, memory, presentation and input, plus a radar that watches other projects (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [DedSec](https://github.com/Hylanderry/DedSec) | – | Portuguese-language GTA VI port targeting DirectX 12 with an RDNA2-to-HLSL translator (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [Xbgtasix](https://github.com/ljm-233/Xbgtasix) | – | Chinese-language layer that places stand-in xgameruntime, xmem and d3d12_xs DLLs next to an executable, plus sourced research notes on the archive, shader and texture formats (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [SeriesX-Emulator](https://github.com/anrq0/SeriesX-Emulator) | – | C++ emulator that interprets x86-64 code instead of running it natively, with a launcher and seven XGameRuntime functions (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [ScarlettCompat](https://github.com/SSTGxf/ScarlettCompat) | – | C++ layer with its own PE loader and Xbox function registry (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [xlaunch-vi](https://github.com/JPTuba9/xlaunch-vi) | – | Multi-contributor C++ layer with a PE loader, a D3D12 backend and an RDNA2 shader translator that matches AMD-compiled test shaders bit for bit (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [OpenScarlett](https://github.com/OpenCommunityEdition/OpenScarlett) | – | AGPL C++ layer, separate from the other OpenScarlett, that swaps console DLLs for PC Direct3D 12 ones and translates console GPU code (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [port-of-the-americas](https://github.com/Steinkoloss/port-of-the-americas) | – | Linux layer that runs GTA VI's own exe under Proton with replacement console DLLs on a patched vkd3d-proton (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [G6Recomp](https://github.com/Kajcsi/G6Recomp) | – | Carefully tested C++ loader and console-runtime rewrite for GTA VI, with its own code throughout (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |
| – | [palmetto](https://github.com/luizwidmer/palmetto) | – | Open continuation of Leonida after its repo went private (PortWatch's summary, unverified) | Not in i95: doesn't reach the world yet |

**Not in i95:** ztrx (#6), victory-perf (#8) and the original CGBJ (#12, run here through DoubleGhosty's cool_fox) are gone from GitHub. omonkey (#10), suri (#11) and Doomsday (#20) are private builds with no public code. VIctory-macOS-halfwave50 is macOS only (2.9-4.5 fps on a MacBook Pro). Below #33 the projects are early-stage layers (Linux/Proton, macOS, recompilations or boot-only builds) that don't reach the game's world on Windows yet.

## Screenshots

**The launcher.** Pick a runtime and a preset, then press Play. **Advanced** sets extra options for a run.

![The launcher: runtimes, details, preset and the game](assets/picker.png)

**The loader.** Shows what's happening before the game starts.

![The loader preparing the shaders](assets/loader.png)

**On-screen keys and frame rate.**

![The controls overlay](assets/controls.png)

How each runtime is built, prepared and started is in [DEVELOPERS.md](DEVELOPERS.md#how-each-runtime-is-wired).

## FAQ

- **Does it work on Windows 10?** Not for every runtime. KLBF Lab needs Shader Model 6.6, and Windows 10's built-in DirectX 12 stops at 6.5; KLBF doesn't load Microsoft's newer DirectX 12 (the Agility SDK), so it crashes on Windows 10. Vilocity does load it.
- **Can I play GTA 6 on PC with this?** Not yet. Every runtime here is still being worked on.
- **Is this a download or a crack?** No. There are no game files, keys or protection bypasses.
- **Whose work is this?** Mostly the people who make the runtimes. i95 is the launcher around them. See [CREDITS.md](CREDITS.md).

## Credits

Every runtime and every project i95 borrows from is in [CREDITS.md](CREDITS.md), with its authors, licence and version. Thanks to all of them, and to [vuyp](https://github.com/vuyp) for the [GTA VI PC race tracker](https://vuyp.github.io/gta6-pc-race/) ([repo](https://github.com/vuyp/gta6-pc-race)), where the ranks come from.

## Licence

GPL-3.0-or-later ([LICENSE](LICENSE)). Each runtime keeps its own licence in `vendor/<name>/LICENSE`. The VIctory forks and ScarlettPort are GPL-3.0, and i95's own TuxBox-based code is Apache-2.0. Grand Theft Auto, GTA and Rockstar Games are trademarks of Take-Two Interactive. Xbox is a trademark of Microsoft. i95 isn't affiliated with or endorsed by them.

<sub>Keywords: GTA 6 PC, GTA VI PC, GTA 6 launcher, Grand Theft Auto VI Windows, Xbox Series X|S to PC, Scarlett, VIctory, victory-fork, AnyXboxX, Kelvaroq, KLBF Lab, Flamingo, Belobug, VItoriaBr, ScarlettPort.</sub>
