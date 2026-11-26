<!-- claude-projects-attribution: {"github_login":"AntoninMichelq"} -->
_Requested by **4nto** · [project thread](https://claude.ai/code/project/chan_01XYoPEzLL1bj63HP9pniGvs?thread=cmsg_01XYoPEzLL1bj63HP9pniGvsYRPS2v37oeDeGaCHT7dFsi)_

## Summary

Descriptors that a waterfall loop reads lane by lane are now traced and lowered, so the loop runs once with every lane using its own descriptor. Before this, tracing stopped with "a descriptor neither loaded nor built" and the whole program was dropped, along with every draw that used it. The branch also traces descriptors that are chosen without a branch, by `s_cselect`/`s_cmov`.

## What changed

Two commits, both based on main:

1. **Trace descriptors chosen by `s_cselect` and `s_cmov`.** The compiler sometimes hoists both descriptor loads and replaces the if/else with `s_cselect_b64` per half, or `s_cselect_b32`/`s_cmov` per dword. Those words are now split by the SCC value they read, and each side is traced into a new `Source::Select`. Lowering picks the heap index of the two loaded descriptors by SCC and creates one heap handle. A choice that mixes two descriptors still stops, with "a descriptor changed after loading".
2. **Lower waterfall loops over per-lane descriptors in one pass.** The `v_readfirstlane_b32` reads of a VGPR descriptor are followed to where each lane gets its own dwords:
   - `global_load_dword*` from a table or heap at a per-lane address, or
   - a `v_cndmask_b32` per-lane choice between such sources.

   Other sources fail and name what they are (LDS, buffer load, phi, input, computed VGPRs). The access gets each lane's heap index, with `createHandleFromHeap`'s `nonUniformIndex` set. The recorded readfirstlanes are lowered as the lane's own dword and their `v_cmp_eq` as true. `docs/architecture.md` is updated.

Adapted to main:
- Added test helpers `lower_with`/`lower_wave`/`root` to `emit::lower::tests`.
- The per-lane load line-up check compares `param` and `dynamic` only. The `includes_base` field it also compared is not on main.

## How it was tested

- `cargo test -p victory-shader` on Linux: all tests pass. New tests:
  - `resource::tests::descriptors_selected_by_scc_are_traced_per_outcome`
  - `resource::tests::a_selection_that_mixes_two_descriptors_fails`
  - `resource::tests::descriptors_read_from_the_first_lane_of_per_lane_loads_are_traced_to_them`
  - `resource::tests::descriptors_read_from_the_first_lane_of_a_per_lane_choice_are_traced_per_lane`
  - `resource::tests::descriptors_read_from_the_first_lane_of_lds_fail_naming_it`
  - `emit::lower::tests::a_descriptor_selected_by_scc_picks_its_heap_index`
  - `emit::lower::tests::waterfall_loops_over_per_lane_descriptors_sample_with_each_lanes_own` (Wave32 and Wave64)
- `cargo check -p victory-shader --target x86_64-pc-windows-msvc` passes.
- On the game's 26,148 programs, "a descriptor neither loaded nor built" went from 298 to 3. This was measured with `victory translate` on the integration stack.

## Where 3D rendering stands on an RX 7600 (Windows 11)

- In the world's first cutscene, props, characters, cables and vehicle tyres now render (seen in a G-buffer view).
- Ground, roads, buildings, textures, sky and lighting are still missing: the lighting/compose output is black.
- No GPU hang (DXGI_ERROR_DEVICE_HUNG) in the last two ~4-minute world runs.
- About 10,000 draws per frame, at well under 1 fps.

These observations come from the integration stack (upstream main plus many open PRs), not from this branch alone.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

https://claude.ai/code/session_01AZ9pSB4ZoXYUifuXg8gm66
