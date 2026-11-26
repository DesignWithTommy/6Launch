<!-- claude-projects-attribution: {"github_login":"AntoninMichelq"} -->
_Requested by **4nto** · [project thread](https://claude.ai/code/project/chan_01XYoPEzLL1bj63HP9pniGvs?thread=cmsg_01XYoPEzLL1bj63HP9pniGvsYRPS2v37oeDeGaCHT7dFsi)_

## Summary

Pixel programs that use their barycentrics in other ways than plain `v_interp_*` now translate, through `SV_Barycentrics`.

1. **Barycentrics read as values.** Programs that use the I/J VGPRs as plain floats (custom interpolation, math on the weights) failed with "barycentrics read directly". I and J are now channels y and z of an `SV_Barycentrics` input. GFX10 interpolates P0 + i (P1 - P0) + j (P2 - P0) with P0 the provoking vertex, which the host's barycentrics weight as vertex 0, so i = y and j = z.
2. **Interpolation with computed barycentrics.** `v_interp_p1`/`p2` (and the f16 forms) fed with an i or j that is not the loaded input VGPR failed with "an interpolation with computed barycentrics". Each half is now evaluated as the hardware does, from the vertex values read by `attributeAtVertex`.

## What changed

- `emit/lower.rs`, `emit/mod.rs`, `emit/shader.rs`, `emit/container.rs` (commit 1):
  - `SV_Barycentrics` is declared with the interpolation mode of its kind (perspective or linear; center, centroid or sample). There is one element per mode (semantic index 0 and 1), with the Barycentrics shader flag and SFI0 feature.
  - The element is left unpacked (no row, packing location -1), like other non-packed system values. The DXIL validator refuses it in a row.
  - The VGPRs are loaded as a value on first use, so programs that only feed them to `v_interp_*` declare nothing new.
  - A console signature's own `SV_Barycentrics` row (D3D name 23) is parsed and reused. Pull-model barycentrics stay unsupported.
- `io.rs`, `emit/lower.rs` (commit 2):
  - A computed i or j gives `Interp::Weighted`: P0 + i (P1 - P0) for the first half, the old dst + j (P2 - P0) for the second.
  - `attributeAtVertex` needs the element declared constant, so every other interpolation of that attribute row is weighed from its vertices with `SV_Barycentrics` of its mode. Flat reads stay `loadInput` of the provoking vertex.
  - `v_interp_mov` of P10 and P20 reads P1 - P0 and P2 - P0 the same way when the program also interpolates the attribute. Alone it is still refused.
- `apps/victory/src/translate.rs`: the tally counts `Interp::Weighted` as interpolated.

Open draft PR #281 also emits `SV_Barycentrics`, but only for direct perspective-center reads and behind an opt-in build feature, because of a concern about the hardware's vertex order. This PR covers all six modes and computed barycentrics, on by default. The two touch the same code in `lower.rs`.

## How it was tested

- `cargo test -p victory-shader` (Linux) passes: 65 tests, including 9 new ones (values read as `SV_Barycentrics`, two modes, interpolation alone, both together, computed barycentrics in `io` and in the emitter, constant weights, computed and loaded on one attribute, P10 of an interpolated attribute). On Windows these tests also run the DXIL validator when it is present.
- `cargo check --tests -p victory-shader -p victory-gpu` and `cargo check -p victory` for `x86_64-pc-windows-msvc` (with `CARGO_FEATURE_PURE=1`) pass.
- On the game's shader corpus, on an integration stack that includes these commits:
  - "an interpolation with computed barycentrics" went from 207 programs to 0.
  - Vertex and pixel shaders are 100% valid under the Windows DXIL validator.

## Where 3D rendering stands on an RX 7600 (Windows 11)

- In the world's first cutscene, props, characters, cables and vehicle tyres render (seen in a G-buffer view).
- Ground, roads, buildings, textures, sky and lighting are still missing: the lighting output is black.
- No GPU hang in the latest world runs of several minutes. About 10,000 draws per frame, at well under 1 fps.

These observations come from the integration stack (upstream main plus many open PRs), not from this branch alone.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

https://claude.ai/code/session_01AZ9pSB4ZoXYUifuXg8gm66
