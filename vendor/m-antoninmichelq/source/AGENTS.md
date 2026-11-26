# Agent guide (Claude, Gemini, Codex…)

**Read [HANDOFF.md](HANDOFF.md) first, in full.** It holds the project state, the current playable build
("Jouer" = branch `jouer/peau3`), its exact environment, what was rejected, the open problems and the
user's rules. Text notes from earlier work are in `docs/notes/`.

Key rules (details in HANDOFF.md §2–3):
- The user speaks French and is not very technical: answer in simple French, step by step.
- Only ONE VIctory instance on the PC at a time, launched by ONE agent (the "PC" agent). Never kill the user's game.
- New test runs need the user's written OK naming them. Close runs with Alt+F4.
- Never commit/attach game files, extracted assets, shader dumps or generated shaders.
- Never run `cargo fmt` on `victory-shader` / `victory-gpu`; rustfmt only touched files.
- Files must stay < 512 KiB and be text (xtask asset policy, CI). Keep `crates/victory-shader/src/emit/lower.rs` small.
- `victory-gpu` only builds for Windows: `cargo clippy -p victory-gpu --target x86_64-pc-windows-msvc -- -D warnings`.
- Fps only counts on release builds, warm cache, same env as the current Jouer.
- Work on your own branch (use `git worktree` if another agent uses the folder) and push to the `fork` remote
  (`https://github.com/AntoninMichelq/VIctory-forkAMD`); `origin` on the PC is Marvisak upstream (read-only).
- Old branches were archived as tags `archive/<branch>` on the fork (`git fetch fork 'refs/tags/archive/*:refs/tags/archive/*'`).
