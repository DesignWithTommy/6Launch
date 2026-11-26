<!-- claude-projects-attribution: {"github_login":"AntoninMichelq"} -->
_Requested by **4nto** · [project thread](https://claude.ai/code/project/chan_01XYoPEzLL1bj63HP9pniGvs?thread=cmsg_01XYoPEzLL1bj63HP9pniGvsYRPS2v37oeDeGaCHT7dFsi)_

## Summary

When it runs short of SGPRs, the console compiler spills a dword, often a lane mask, into one lane of a VGPR with `v_writelane_b32`. It reloads the dword later with `v_readlane_b32` at the same lane. Reads of such a reload now read the spilled value directly, so a reloaded lane mask is again the same mask for the structurer and the lowering. This is a port of the spill resolution from #54 by alessiodeia, on its own and without #54's Wave64/host-wave changes.

## What changed

- New `victory_shader::sgpr_spill` module:
  - `forward(&mut Program)` replaces every read of a reload whose VGPR lane provably holds one scalar value with that value. "Provably" covers other lanes' writes, agreeing joins, lane bits masked to the wave size, and the value being available wherever the reload runs.
  - `dead(&Program)` names the spills and reloads that nothing reads any more.
- `forward` runs right after `ir::build`, in `emit::translate` and in `victory translate`.
- Lowering skips dead spills and reloads. They are also left out of the console-lane and whole-wave-section scans.
- When a scalar mask operation's SCC is read as data (for example by `s_cselect`), it now lowers as "some lane is set", meaning the mask's ballot is not zero. This applies where masks may be read as data (compute). Before, it failed with "a lane mask read as data".
- A whole-wave section now ends at the EXEC write that reads the saved mask from any register, so a forwarded reload of a spilled saved EXEC closes it.

No dependency on other open PRs.

## How it was tested

- `cargo test -p victory-shader` on Linux: all tests pass. New tests in `sgpr_spill::tests`:
  - `a_reloaded_mask_is_the_spilled_mask`
  - `a_spilled_mask_holding_lanes_elsewhere_is_used_as_a_mask_again`
  - `only_the_same_lane_written_by_a_lane_write_is_forwarded`
  - `a_reload_needs_the_value_wherever_it_runs`
  - `the_scc_of_a_mask_operation_is_whether_a_lane_is_set`

  Also `emit::wave::tests::a_section_restores_exec_from_a_reload_of_the_saved_mask`.
- `cargo check` for Windows (`x86_64-pc-windows-msvc`) passes, for `victory-shader` and for `victory` with `CARGO_FEATURE_PURE=1`.
- On the game's 26,148 programs, "a lane mask holding lanes elsewhere read as data" went from 276 to 42. This was measured with `victory translate` on the integration stack.

## Where 3D rendering stands on an RX 7600 (Windows 11)

- In the world's first cutscene, props, characters, cables and vehicle tyres now render (seen in a G-buffer view).
- Ground, roads, buildings, textures, sky and lighting are still missing: the lighting/compose output is black.
- No GPU hang (DXGI_ERROR_DEVICE_HUNG) in the last two ~4-minute world runs.
- About 10,000 draws per frame, at well under 1 fps.

These observations come from the integration stack (upstream main plus many open PRs), not from this branch alone.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

https://claude.ai/code/session_01AZ9pSB4ZoXYUifuXg8gm66
