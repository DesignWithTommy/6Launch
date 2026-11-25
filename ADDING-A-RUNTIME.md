# Adding a runtime to Neonbridge

Each step is something an earlier runtime needed (named in brackets).

1. **Check it is a leader.** It must be in the [race tracker](https://github.com/vuyp/gta6-pc-race)'s top 10, and its repository must still exist (ztrx and victory-perf vanished). The licence must be GPL-3.0 or more permissive.
2. **Copy it.** Clone it at its newest commit into `vendor/<name>/` without its `.git`. Put the full hash in `UPSTREAM`. Delete game data (`.zip` saves in victory-amd, and any `.rpf`, `.exe` or `.dll`).
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
10. **Document it.** Add its row to the README table (author's claim beside our result). Add its build, cache format and family to `WIRING.md`.
11. **Ship it.** Stage files by name and check the staged diff for private details. Commit on main and push with the hash guard.
12. **When it drops out of the top 10:** set `archived = true`. It stays in the launcher under Archived; don't delete it.
