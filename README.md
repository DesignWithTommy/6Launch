# Neonbridge: one launcher for every GTA VI (GTA 6) Xbox Series runtime on PC

![Neonbridge: GTA 6 Xbox Series launcher for Windows PC](assets/banner.svg)

Test them all in one convenient launcher.

Neonbridge is a Windows app that runs every project trying to bring the Xbox Series version of GTA VI to PC, from one window, on a copy of the game you dumped from your own console. **None of them is playable yet.** No game files are included.

## Standings

The race's top 10 from the [GTA VI PC race tracker](https://github.com/vuyp/gta6-pc-race) ([live page](https://vuyp.github.io/gta6-pc-race/)), all in Neonbridge. **Nothing is playable yet** (the tracker's bar: 20 fps and 30 minutes without a crash). The launcher lists them grouped by the project each forks, best first.

"Here" = our PC: i5-14400F, RTX 4060 8 GB, 16 GB RAM, Windows 11. Most authors test on faster cards, so expect less here.

| Race | Runtime | Forks | Its author reports | Here |
|---:|---|---|---|---|
| 1 | [VIctory-7900XTX](https://github.com/avatarmanz/VIctory-7900XTX) | VIctory | 13–22 fps, 41 min no crash, RX 7900 XTX | Testing |
| 2 | [AnyXboxX](https://github.com/LooperSalty/AnyXboxX) | Neptune | 14–17 fps in the world, RTX 4070 SUPER | ✅ 10 min, no crash, ~14 fps (earlier build); **try this one first on NVIDIA** |
| 3 | [Kelvaroq](https://github.com/fablinoo/kelvaroq) | victory-fork | ~10 fps, 36 min no crash, RTX 5080 | Testing |
| 4 | [KLBF Lab](https://github.com/PastiiJay/klbf-lab) | victory-fork | 5–9 fps, 77 min, RX 9070 XT | ❌ RTX 4060, game 1.0.2.1: the game stops itself at the intro; klbf lacks device method 102 (slot 0x330). Its bug, not the launcher's |
| 5 | [Flamingo](https://github.com/realblastex/flamingo) | victory-fork | 7.8–14 fps, 24 min | ⚠️ crashed since fe12291 |
| 6 | [victory-fork](https://github.com/fxceless/victory-fork) | VIctory | 4.5–7.6 fps, 30 min | ✅ intro 30 fps, 3D scene ~1 fps |
| 7 | [Vice-Tay](https://github.com/TaynanGT/Vice-Tay) | VIctory | 30 min in the world, RTX 4090 | Testing |
| 8 | [VI-BR-PORT](https://github.com/edsuuu/VI-BR-PORT) | victory-fork | 4–7 fps, hour-long, RTX 4060 Ti | ❌ game stops ~20 s in (method 102) |
| 9 | [VIctory-Belobug-AMD](https://github.com/Belobug/VIctory-Belobug-AMD) | VIctory | 5–8 fps, 15 min drive | ❌ |
| 10 | [VItoriaBr](https://github.com/uiliannn/VItoriaBr) | VIctory | 8 GB preset in its README | Testing |

**Archived** (in the launcher under Archived, below Mix): [victory-amd](https://github.com/myokex/victory-amd), [victory-unified](https://github.com/Itsbatt/victory-unified), [VItoriaBr](https://github.com/uiliannn/VItoriaBr) (✅ 3D prison scene at 2–3 fps, best verified here) and [ScarlettPort](https://github.com/ZigZagBoi1/ScarlettPort) (✅ 3D at 2–3 fps). xcl and ztrx-victory are out of the tree: `git checkout 5a8260ee -- vendor/xcl` or `git checkout 2210a250 -- vendor/ztrx-victory` brings them back.

## Get started

1. Download the **v0.3.0-alpha** zip from [Releases](../../releases).
2. Unzip it anywhere, for example `C:\Games\Neonbridge`.
3. Run `neonbridge.exe`.
4. If it doesn't find your game folder by itself, click **Change** and choose it. Then pick a runtime and press **Play**.

The first start of a runtime takes a while: it builds the runtime and prepares the game's shaders. The loading screen shows real counts for each step (for example "translating shaders: 1,000 / 26,503"), and a moving pulse where a runtime gives no count. Later starts are quicker: a runtime that hasn't changed isn't built again.

## What you need

- **Your own copy of the game:** the Xbox Series X|S version of GTA VI, dumped from a console you own. It's the folder with `game_scarlett_master.exe` in it, about 210 GB.
- **Windows 10 or 11**, 64-bit, with a processor that supports AVX2.
- **A graphics card** that supports DirectX 12 with mesh shaders (Shader Model 6.6), with an up-to-date driver.
- **Memory:** 32 GB of RAM is recommended.
- **Disk space:** the game, plus up to about 20 GB per runtime for its files and shaders.


## Screenshots

**The launcher.** Pick a runtime and a preset, then press Play.

![The launcher: runtimes, details, preset and the game](assets/picker.png)

**Advanced.** Mix parts of different runtimes, or set options for a run.

![Advanced: mix mode and the run options](assets/mix.png)

**The loader.** Shows what's happening before the game starts.

![The loader preparing the shaders](assets/loader.png)

**On-screen keys and frame rate.**

![The controls overlay](assets/controls.png)

How each runtime is built, prepared and started: [DEVELOPERS.md](DEVELOPERS.md#how-each-runtime-is-wired).

## Mix and match (help wanted)

**Advanced** in the launcher builds one runtime from parts of several (VIctory-fxceless, VItoriaBr, ScarlettPort, Flamingo, KLBF Lab, Belobug). These mixes **compiled** in our own build check (made against older copies of each runtime). **None has been run in the game yet**, by us or by the runtimes' authors, so please try them and open an issue with what you see.

| Main program | Part swapped in | From |
|---|---|---|
| VIctory-fxceless | console system layer (`victory-hle`) | VItoriaBr |
| VIctory-fxceless | game file loader (`victory-loader`) | VItoriaBr or ScarlettPort |
| VIctory-fxceless | program loader (`victory-pe`) | VItoriaBr |
| VIctory-fxceless | `victory-core`, `victory-game` or `victory-formats` | ScarlettPort |

The shader translator, shader cache reader and pipeline builder (`victory-shader`, `victory-gpu`, `victory-pipeline`) always come from the main program, so its shader cache keeps working. **No mix has been tested yet.** Mixes with Flamingo, KLBF Lab or Belobug parts haven't even been compile-checked. A mix that fails to compile is remembered and refused next time.

## Watch on your phone (Tailscale)

Hand this to your AI agent (Claude Code, Codex and the like), or run it yourself in a terminal in the Neonbridge folder:

```powershell
.\neonbridge.exe --feed-only
```

1. It prints `Tailscale sign-in needed: open <link>`. Open that link, sign in to your own free Tailscale account and approve the PC (once; nothing to install).
2. It prints `live at https://neonbridge.<your-tailnet>.ts.net`. Open `http://127.0.0.1:8788` on this PC first and set the stream password.
3. Install the Tailscale app on your phone, sign in to the same account, then open the `ts.net` address.
4. Leave that window open and start games with `NEONBRIDGE_FEED=1` set (or skip `--feed-only` and launch with it set: the stream starts with the game). `NEONBRIDGE_FEED_SCREEN=1` streams your whole left monitor instead of the game window.

## Keys

| Key | Does |
|---|---|
| **F1** | Shows or hides the controls |
| **F2** | Hides the controls |
| **F3** | Shows or hides the frame rate |

Each runtime has its own keyboard and mouse layout. F1 shows it in game.

## Where things are

Everything Neonbridge keeps is in `%LOCALAPPDATA%\6ixpack`. Paste that into the Explorer address bar to open it.

| What | Folder |
|---|---|
| Saves | `6ixpack\save` |
| Shaders and other caches | `6ixpack\cache`, and `6ixpack\<runtime>\cache` |
| Logs | `6ixpack\logs` |

## Troubleshooting

- **The game crashed or closed.** Every run is logged, so there is always a log to send. Neonbridge shows its location. Send that file from `%LOCALAPPDATA%\6ixpack\logs` (the newest one) when you report the problem.
- **A runtime is greyed out.** It needs something that isn't installed yet. Select it to see what, then click **Install...**.
- **The first start is slow.** That's the shader preparation. It's only slow the first time.
- **It says no game was found.** Click **Change** and choose the folder that holds `game_scarlett_master.exe`.

## FAQ

- **Can I play GTA 6 on PC with this?** Not yet. Every runtime here is still a work in progress.
- **Is this a download or a crack?** No. There are no game files, keys or protection bypasses. You need your own dump.
- **Whose work is this?** Mostly the authors of the projects it runs. Neonbridge is the launcher around them. See [CREDITS.md](CREDITS.md).
- **Does it go online?** Only when you say yes, for example to an **Install...** download.

<details>
<summary><b>For developers</b></summary>

Building from source, the command line, runtime settings, patches and environment variables are in [DEVELOPERS.md](DEVELOPERS.md).

</details>

## Credits

Every runtime and every project Neonbridge borrows from is named in [CREDITS.md](CREDITS.md), with its authors, licence and version. Thanks especially to [realblastex](https://github.com/realblastex) for [Flamingo](https://github.com/realblastex/flamingo) and to [Belobug](https://github.com/Belobug) for [VIctory-Belobug-AMD](https://github.com/Belobug/VIctory-Belobug-AMD).

## Licence

GPL-3.0-or-later ([LICENSE](LICENSE)). Each runtime keeps its own licence (`vendor/<name>/LICENSE`): xcl is GPL-2.0-only, the VIctory forks (Flamingo and VIctory-Belobug-AMD among them) and ScarlettPort are GPL-3.0, and Neonbridge's own TuxBox-based code is Apache-2.0. Grand Theft Auto, GTA and Rockstar Games are trademarks of Take-Two Interactive; Xbox is a trademark of Microsoft. Neonbridge isn't affiliated with or endorsed by them.

<sub>Keywords: GTA 6 PC, GTA VI PC, GTA 6 launcher, Grand Theft Auto VI Windows, Xbox Series X|S to PC, Scarlett, VIctory, Flamingo, Belobug, ScarlettPort, xcl, ztrx.</sub>
