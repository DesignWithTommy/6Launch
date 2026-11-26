<!-- claude-projects-attribution: {"github_login":"AntoninMichelq"} -->
_Requested by **4nto** · [project thread](https://claude.ai/code/project/chan_01XYoPEzLL1bj63HP9pniGvs?thread=cmsg_01XYoPEzLL1bj63HP9pniGvsYRPS2v37oeDeGaCHT7dFsi)_

## Summary

The whole-wave section simulation now follows more of the steps the console compiler uses in wave reductions and prefix sums. Lane-local code that the compiler schedules into a section is lowered as it would be outside one. Before, these programs failed with "a whole-wave section step the simulation does not follow".

## What changed

`emit/wave.rs`: the simulation now also follows:
- `v_writelane_b32` with a constant or known-uniform lane. This covers the exclusive prefix sum that carries lane 15 into lane 16 (and 31/47 into 32/48 on Wave64).
- Scalar ALU on uniforms:
  - `s_mov_b32` copies;
  - `s_add`/`min`/`max`/`and`/`or`/`xor` combining two `v_readlane` results (the two halves of a Wave64 reduction, read at lanes 0 and 32);
  - `s_mov` copies of a mask derived from the saved EXEC.
- `v_readlane_b32` with an SGPR lane the simulation knows (lane bits masked to the wave size), and `v_readfirstlane_b32`.

A lane's own value from before the section, once moved to another lane by DPP, `v_permlanex16` or `v_readlane`, is now unknown. Before, it was wrongly taken as the reading lane's own value.

`emit/lower.rs`: some values have no shape in the simulation but are computed only by steps that read no other lane: scalar code and loads, or vector code without DPP, permutes, lane reads/writes or EXEC operands. The lanes active on entry now compute those values under their own EXEC.

Adapted to main: main does not model `v_permlane16` (#98 adds it), so only the `v_permlanex16` arm is touched. No dependency on other open PRs.

## How it was tested

- `cargo test -p victory-shader` on Linux: all tests pass. New tests:
  - `emit::wave::tests::writelane_carries_a_row_total_into_an_exclusive_prefix`
  - `emit::wave::tests::wave64_halves_combine_in_a_scalar_add`
  - `emit::wave::tests::another_lanes_own_value_is_not_the_lanes_own`
  - `emit::lower::tests::scalar_code_inside_a_whole_wave_section_is_lowered`

  All test programs were assembled with llvm-mc for gfx1030.
- `cargo check -p victory-shader --target x86_64-pc-windows-msvc` passes.
- On the game's 26,148 programs, "a whole-wave section step the simulation does not follow" went from 51 to 14. This was measured with `victory translate` on the integration stack.

## Where 3D rendering stands on an RX 7600 (Windows 11)

- In the world's first cutscene, props, characters, cables and vehicle tyres now render (seen in a G-buffer view).
- Ground, roads, buildings, textures, sky and lighting are still missing: the lighting/compose output is black.
- No GPU hang (DXGI_ERROR_DEVICE_HUNG) in the last two ~4-minute world runs.
- About 10,000 draws per frame, at well under 1 fps.

These observations come from the integration stack (upstream main plus many open PRs), not from this branch alone.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

https://claude.ai/code/session_01AZ9pSB4ZoXYUifuXg8gm66
