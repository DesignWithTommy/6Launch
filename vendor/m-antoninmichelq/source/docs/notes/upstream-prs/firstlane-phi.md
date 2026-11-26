<!-- claude-projects-attribution: {"github_login":"AntoninMichelq"} -->
_Requested by **4nto** · [project thread](https://claude.ai/code/project/chan_01XYoPEzLL1bj63HP9pniGvs?thread=cmsg_01XYoPEzLL1bj63HP9pniGvsYRPS2v37oeDeGaCHT7dFsi)_

## Summary

Per-lane descriptors are now traced through the VGPR phis that join them. Tracing used to stop there with "a descriptor read from the first lane of a phi".

A waterfall loop's descriptor is often held in VGPRs that phis join:

- carried unchanged through a loop, through copies the phis do not see through;
- loaded in the two branches of an if/else on EXEC;
- reloaded in some passes of an outer loop.

This PR builds on the waterfall lowering in the `up/waterfall-descriptors` branch. Its first two commits are that branch, unchanged. Only the last commit is new here.

## What changed

- `resource.rs`:
  - The tracer follows VGPR phis. A phi whose arguments all come to one dword (through other phis and `v_mov` copies, ignoring self-references and VGPRs no instruction wrote on some path) is that dword.
  - Other joins become `Source::Paths`: one source per predecessor, chosen by the path each lane takes into the block. After structurisation, EXEC branches are per-lane branches.
  - `Source::Carried` stands for a loop's back edge to the join being traced. `Source::Unset` stands for VGPRs never written on a path. These are listed in `Waterfall::unset`, and lowering refuses them if the hardware loads them.
  - Arguments that cannot be traced fail with their own reason (LDS read, input VGPR, ...).
- `emit/lower.rs`: `Paths` becomes a DXIL phi of each predecessor's heap index, computed at the end of that predecessor (filled after all blocks, for back edges). It is used through `createHandleFromHeap` with `nonUniformIndex`. A per-lane descriptor loaded under a partial EXEC (a guarded load in an if/else without `s_cbranch_execz`) now fails instead of giving every lane the loaded index.
- Junk edges: in the game's joins, one predecessor usually brings a real descriptor (an `s_load_dwordx4` from a table) and the others bring VGPRs that cannot be one (computed values, a buffer load, an input VGPR). Lanes on those edges never reach the access. Such paths become `Source::NoDescriptor` (heap index 0), as long as at least one other path is a real descriptor. LDS reads are not counted as junk.
- First-lane reads outside a waterfall loop: a descriptor whose `v_readfirstlane_b32` results are not all compared with each lane's own dwords in the same loop is not a waterfall. The console uses the first active lane's descriptor for every lane, so the lowering does the same: the handle index is `WaveReadLaneFirst` of the per-lane heap index, read where the console reads the dwords, and the handle stays uniform. Compares of those reads outside a loop lower as a compare of heap indexes. A loop that compares only some of the dwords still fails, because the hardware result there cannot be reproduced from heap indexes.
- `apps/victory/src/translate.rs`: the tally covers the new `Source` variants (`Fetched`, `Select`, `Lanes` from the waterfall commits, and `Paths`, `Carried`, `Unset`). Without this, `cargo check -p victory` does not build on top of the waterfall commits alone.
- `docs/architecture.md`: a short note on phi joins.

## How it was tested

- `cargo test -p victory-shader` (Linux) passes: 76 tests, including new tracer tests (a loop-carried phi, an if/else join, a descriptor replaced in a loop, one joined from LDS, joins with junk edges, first-lane reads outside a loop) and lowering tests for a load under a partial EXEC and for first-lane reads outside a waterfall.
- `cargo check --tests -p victory-shader -p victory-gpu` and `cargo check -p victory` for `x86_64-pc-windows-msvc` (with `CARGO_FEATURE_PURE=1`) pass.
- On the game's 26,148 shader programs, on an integration stack that includes this change, "a descriptor read from the first lane of a phi" went from 294 programs to 0. The programs that still stop now stop later, on other reasons: computed VGPRs (138), a buffer load (124) and an input VGPR (11).

## Where 3D rendering stands on an RX 7600 (Windows 11)

- In the world's first cutscene, props, characters, cables and vehicle tyres render (seen in a G-buffer view).
- The first lights now show in the lit image (flares, lens streaks and some lit areas). Most of the lighting, plus ground, roads, buildings, textures and sky, is still missing or black.
- No GPU hang in the latest world runs of several minutes. About 10,000 draws per frame, at well under 1 fps.

These observations come from the integration stack (upstream main plus many open PRs), not from this branch alone.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

https://claude.ai/code/session_01AZ9pSB4ZoXYUifuXg8gm66
