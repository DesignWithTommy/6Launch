# i95-Victory

One VIctory runtime with the fixes from several forks merged, aimed at an 8 GB NVIDIA card (built for the i95 launcher). GPL-3.0, like everything it comes from.

**Base:** fablinoo's [Kelvaroq](https://github.com/fablinoo/kelvaroq) 0a3c30d, which is fxceless's [victory-fork](https://github.com/fxceless/victory-fork) f85101b plus Kelvaroq's 64 patches and its `kelvaroq-render` crate. Here those patches are already applied, so the tree builds straight away and every later fix is a normal commit. The original victory-fork README is in `docs/VICTORY-FORK-README.md`.

## What is merged on top, and from whom

- **Shaders mark the textures they really read, and the card keeps those** (black skin, hair and clothes on 8 GB). From edsuuu's [VI-BR-PORT](https://github.com/edsuuu/VI-BR-PORT) commits 44823d61, da32cb58 and 719e3b15. On by default; `VICTORY_HEAP_USAGE=0` turns the marks off, `VICTORY_TEXTURE_SPILL=0` stops textures going into shared memory.
- **`VICTORY_TEXTURE_DROP_MIPS=N`**: leave out each texture's N largest mip levels so 8 GB does not run out. From [soldsawn/victory-nvidia-8gb](https://github.com/soldsawn/victory-nvidia-8gb) (b4ff304 by ofc0urse, fc4b3dd and 7d0e398 by Filip722); VIniVIdiVIci carries the same.
- **Character skin is baked from textures that are really there.** Before a decoration bake (the draws that bake skin once per character), the textures it names are uploaded whatever the card's room, the card may fill to 97 %, and textures whose bytes just landed are refreshed first. Textures the card drew into are never evicted, and a texture given room gets its mip tiles at once. From edsuuu's VI-BR-PORT: 277bc02, 4b79ed7, f273d02, 42196b1, 3dfeb17 (the last re-done on Kelvaroq's code).
- **Cards with a small budget**: the space kept for texture copies is a third of the budget at most (a360025 by Dathinel, via soldsawn). No change on 8 GB.
- **DROP_MIPS copies go one level at a time** (5cd45f7 by ofc0urse, via VIniVIdiVIci).
- **`VICTORY_RESOLUTION=1920x1080`**: caps the size the game renders at, so a 1080p screen is not handed a 4K frame (`VICTORY_DISPLAY_MODE` alone only tells the game 1080p). Also `VICTORY_WINDOW_PERCENT` for a window a bit smaller than the screen if it shows black. 037ef58 by DKTTT, via soldsawn.
- **`VICTORY_AMBIENT=x`**: a constant stands in for indirect light while ray tracing does not run, so dark surfaces are not black. ad702f7 by UwUCammie, as carried by VItoriaBr. Off unless set.

## Looked at and not merged

- wolfattack199's "a failed read no longer uploads zeros": Kelvaroq already does this (a refresh only skips waiting when the texture was shown once before). Their `MAP_RUNS` was left out on purpose: it broke faces.
- VIniVIdiVIci's heap-pass speedups (fxceless PRs #1043 and #1190): Kelvaroq rewrote that same pass its own way (page snapshots); the two clash in 20 places and would do the same job twice.

## Build

```
cargo build --release --locked -p victory --bin victory
```

The program is `target/release/victory.exe`. `NEONBRIDGE.toml` is the launcher spec, with an `8gb` preset.

Credits in `CREDITS.md`.
