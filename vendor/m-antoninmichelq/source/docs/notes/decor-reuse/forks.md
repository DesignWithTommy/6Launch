# Forks survey: dynamic addresses / pointer chase / indirect args (2026-11-24)

Baseline: AntoninMichelq/VIctory `victory-stack-d13z2` (c743ce8, contains upstream main ffe310e).
Method: no GitHub REST access in this session (the API proxy allows only configured repos), so the fork
list endpoint was unusable. Instead: blobless fetch of upstream with all 726 `refs/pull/*` heads (every fork that
opened a PR), plus justr4mzi/VIctory, dreaming66/VICauseway, Rafa23xz/victory-shader-census and our fork;
2291 commits ahead of upstream main and not in our fork were grepped and read. **Forks that never opened a PR were not covered.**
Tracker 4lwayz/portwatch-vi lists ~45 other projects; only ivall/xcl was checked (it is the one with a relevant design).

## Bottom line
- **Pointer chase (load an address from memory, then load through it): no VIctory fork solves it.**
- **Phi addresses with different bases: no VIctory fork solves it.** Same-base loop/branch phis are already in d13z2 (`resource.rs::pointer_address`).
- The only generic solution anywhere is **ivall/xcl's UMA design** (GPL-2.0-only: ideas only, no code copy into GPL-3).

## Ranked findings
1. **ivall/xcl `main` (930e8b4), GPL-2.0-only.** Title GPU memory sits in up to 4 heaps of 4 GiB each, opened with `OpenExistingHeapFromAddress`/sparse VRAM. Shaders see them as `RWByteAddressBuffer mem, mem1..3`. Any 64-bit address becomes window `(hi - uma_base)` plus a 32-bit offset. That is generic BDA: phi, pointer chase, root-constant writes and "addr not from root param" all go away. Files: `crates/xcl/src/gpu/uma.rs`, `gpu/recomp/emit.rs` (~l.1577-1592, 4241-4290), `hle/d3d12x/sparse.rs`; commit 4a8344c. Not in d13z2. **Recommendation: clean-room reimplementation** as a fallback path for programs the tracker refuses. This is a large architectural change (it needs title memory allocated inside heap-openable regions). It is the only route found for pointer chase.
2. **pr/956 Noname0202, 71967c1 + db86e1e (GPL-3).** ExecuteIndirect with multi-argument signatures (root-constant args, then draw/dispatch), mapped onto our root signature. The clamp runs at `arg_offset`. Files: `victory-gpu/src/{indirect,draw,list}.rs`, `pipeline/host.rs`. **Not in d13z2**: `list.rs::execute_indirect` takes only `[[kind]]` one-argument signatures, and drops everything else. **Port first**: GPU-driven world draws usually carry a per-draw root constant, so this can be the zero-draw cause by itself. Check with the existing "indirect signature [..]" diagnostics line.
3. **pr/1056 adamfx, 417f4e3 (GPL-3).** An `s_load_dword*` from a root SRV/CBV address used as data, not as a descriptor, now reads memory. d13z2 still refuses this with "a descriptor read only as data" (`lower.rs` ~6142/6155). This is step 1 of a pointer chase (it reads the pointer) but not step 2. Small; **port**.
4. **pr/986 CitizenCInema, d8d7c2c (GPL-3).** Structured SRVs whose offset is not a multiple of the stride are copied into an element-aligned scratch slice at submit, with `FirstElement = at/element`. d13z2 nulls these. The same trick applies to raw views not on 16 bytes: d13z2 `descriptor/host.rs::raw_srv_at` and the root-UAV path null them. **Port and extend it to raw/root views.**
5. **pr/645 nathanbzrr, 499bd98 (also in justr4mzi ultimate-rx6700xt), GPL-3.** Address tracker additions: s_sub, s_addk, (v|s)_lshl_add, v_add3, aligned s_or/v_or, readfirstlane, VA masks, two dynamic terms, plus same-base phis. It still refuses phis with different bases. Partly overlaps d13z2 and our parked `fix/described-smem` (shifted bases carried in loops, joined code-table pointers). **Mine its arithmetic forms** for "addr not from root param". It is 2.2k lines on an older base, so cherry-picking it whole will conflict.
6. **Already equivalent in d13z2 (skip):** spill-table reading (pr/489 2a30b95/fe7210e; d13z2 has `unspill.rs`). Root UAVs (pr/1077 873548f; d13z2 has `root_uavs`, gated by `VICTORY_ROOT_UAVS=1`, so **make sure that flag is on in runs**). Address in two root constants (pr/667 995f7f3, pr/710 b1faa34/ac1bc81; d13z2 has `POINTER_SLOT`). Count buffer and arg-buffer barriers (pr/40 asc, pr/242 1711205, pr/583 07bae5d; d13z2 `indirect.rs` honours `min(max,count)`, and `draw.rs` sets `BUFFER_READ` on args and count). 16-byte raw-view refusal (pr/123, pr/586).
7. **Low value:** pr/1066 bfcdf58 (indirect draw compaction, performance only). pr/994 and pr/493 (root SRVs bound as tables bounded to their host copy). VICauseway (diagnostics, mips, heap lifetime; nothing on compute addresses). The census repo is a measurement tool only.

## Versus fix/described-smem
None of the fork work overlaps its shifted-base / loop-carried-VGPR-base commits; 499bd98 overlaps only its same-base phi part. "Typed root view" (`lower.rs` ~6749): no fork fixes this (873548f handles raw views only).
