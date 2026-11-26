# i95-Victory

One VIctory runtime with the fixes from several forks merged, aimed at an 8 GB NVIDIA card (built for the i95 launcher). GPL-3.0, like everything it comes from.

**Base:** fablinoo's [Kelvaroq](https://github.com/fablinoo/kelvaroq) 0a3c30d, which is fxceless's [victory-fork](https://github.com/fxceless/victory-fork) f85101b plus Kelvaroq's 64 patches and its `kelvaroq-render` crate. Here those patches are already applied, so the tree builds straight away and every later fix is a normal commit. The original victory-fork README is in `docs/VICTORY-FORK-README.md`.

## What is merged on top, and from whom

- **Shaders mark the textures they really read, and the card keeps those** (black skin, hair and clothes on 8 GB). From edsuuu's [VI-BR-PORT](https://github.com/edsuuu/VI-BR-PORT) commits 44823d61, da32cb58 and 719e3b15. On by default; `VICTORY_HEAP_USAGE=0` turns the marks off, `VICTORY_TEXTURE_SPILL=0` stops textures going into shared memory.
- **`VICTORY_TEXTURE_DROP_MIPS=N`**: leave out each texture's N largest mip levels so 8 GB does not run out. From [soldsawn/victory-nvidia-8gb](https://github.com/soldsawn/victory-nvidia-8gb) (b4ff304 by ofc0urse, fc4b3dd and 7d0e398 by Filip722); VIniVIdiVIci carries the same.
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
