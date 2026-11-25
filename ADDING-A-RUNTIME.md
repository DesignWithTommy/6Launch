# Adding a runtime to Neonbridge

1. **Check it is a leader.** It must be in the [race tracker](https://github.com/vuyp/gta6-pc-race)'s top 10. Its licence must be GPL-3.0 or more permissive.
2. **Copy it.** Clone it at its newest commit into `vendor/<name>/` without its `.git`. Put the full commit hash in `vendor/<name>/UPSTREAM`. Delete any game data: saves, `.zip`, `.rpf`, `.exe` or `.dll` files.
3. **Credit it.** Write `vendor/<name>/VENDORED.md` with the source link, the commit, the licence, the author and what was removed. Add a row for it to `CREDITS.md`.
4. **Write `vendor/<name>/NEONBRIDGE.toml`.** Take every value from the author's own README or launch script, and cite the line it came from:
   - `name`, `fork` (the project it forks, which the picker groups by), `rank` (its launcher order), `about`, `upstream`
   - `build`, `exe`, and `diag_build = []` unless the author plays the diagnostics build
   - `run` and `prewarm`: the author's run and shader-prebuild commands
   - `game_env`, `cache_env`, `agility`
   - `[env]`: the author's default switches, plus `VICTORY_DATA_DIR`
   - `[controls]`
   - `archived = true` when it drops out of the top 10 (kept, listed under Archived)
5. **Leave its shader cache key alone.** Do not change its `victory-shader` source, its build kind, `shader_cache.rs` or the switches the translator reads.
6. **Patch only if it crashes.** Patches go in `vendor/patches/<fix>-<name>.patch` and never touch `victory-shader`. Wire each one in `patch_root()` in `src/runtimes.rs`.
7. **Register its cache.** Add the name to `cache_format` in `src/runtimes.rs` (VSH1 or VSH2, matching its family).
8. **Check it.** Run `cargo check`, then build the launcher.
9. **Test it through the launcher only.** Prewarm first, then run it. Read the run with `triage.py`.
10. **Document it.** Add its row to the README standings table, with the author's claim and our result. Add its entry to `WIRING.md`.
11. **Ship it.** Stage files by name and check the staged diff for private details. Commit on main and push with the hash guard.
