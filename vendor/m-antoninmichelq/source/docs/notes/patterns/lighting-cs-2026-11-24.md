# Lighting-path translate failures: pattern notes (2026-11-24)

Pattern descriptions only (no game code). Source: offline translate + diag run on the user's PC (D9/diag stack). These 5 computes are dropped "no host pipeline" before the lighting composite 34fb51b8a0312e82 (drops per frame in brackets).

## b22a16a178976e1b @0x22c "a whole-wave section step the simulation does not follow" [x28]
- Short compute (~220 instr). Root: one Table (user_data 4), 4 buffer descriptor ranges, stride 4 (regs 6-9).
- Failing instruction: buffer_store_format_d16_xyzw. Data: VGPR pair (4 packed f16), address: index VGPR (idxen), descriptor: 4 SGPRs, offset 0.
- Wave64 section before it (masks in SGPR pairs): s_or_saveexec -1 (whole wave); v_cndmask zeroes originally inactive lanes; 3 f16 adds with DPP8 (1, 2, 4-lane swaps); add with row_mirror; v_permlanex16; add; v_readlane lane 32 -> SGPR; add SGPR; v_readlane lane 0; exec restored. No v_writelane.
- Then lane 0 only (v_cmp == 0, s_and_saveexec): index = SGPR << 3 + VGPR, conditional buffer_load_format_d16_x (else 0), add wave total, v_pack_b32_f16, then the failing store.
- Reading: whole-wave f16 sum (luminance/exposure-like reduction) written by lane 0.

## 55679a0547649d0b @0x3584 and 17af15c4015cb4b6 @0x2bfc "a lane mask holding lanes elsewhere read as data" [x16 each]
- Failing instruction: buffer_store_dword (data VGPR, index VGPR idxen, offset 12) inside a scalar loop: SGPR counter +1, s_cmp_eq_u32 against an end SGPR.
- Each iteration stores 3 dwords at offsets 12, 4, 0; index = counter + base VGPR.
- Stored values built in SALU (s_lshl 22 and 14, s_and 0xff000000, s_or), then v_mov to VGPR or combined (v_or_b32, v_lshl_or_b32 with counter << 6).
- Mask origin not seen in the inspected window: likely the loop's exec or one of the assembled SGPRs coming from a mask. Needs a wider upstream window.

## f7c4f227a2a4c4b2 @0x2a78 and b3e7781d53ff8326 @0x1e50 "an address from a phi" [x5, x4]
- Failing instruction: global_load_dwordx2, 64-bit address in a VGPR pair, no SGPR base, offset 0.
- Address = base s[2:3] (looks constant, likely a root parameter) + ((h & 63) << 3) via v_add_co_u32 / v_add_co_ci_u32.
- h = hash of a pseudo-random generator state: state * SGPR + literal constant (LCG step), then shifts/XORs (lshr 28, +4, variable lshr, xor, mul by SGPR, lshr 22, xor).
- The loop phi carries the generator state (re-multiplied and incremented each iteration). So: a read from a 64-entry table of 8-byte items at a pseudo-random index.

## One of the 101 "descriptor read from the first lane of a buffer load mixed with other words": f6f879ef57e69f94 (pixel) @0x3028
- Stops on image_sample_lz 1D in a classic waterfall loop.
- 4 v_readfirstlane on 4 consecutive VGPRs form a 4-dword SAMPLER descriptor (s[12:15]); two v_cmp_eq_u64 check both halves; s_and then s_and_saveexec; sample; s_xor exec; loop back.
- The IMAGE descriptor is 8 dwords loaded scalar (s_load_dwordx8 from a table pointer, offset 0xc0).
- Which of the 4 sampler VGPR words come from the buffer_load vs other ops: not yet traced word by word (cbee7b1, constant/SGPR words, did not change the count, so at least one word is likely a VALU op on loaded data).

---
## Indirect computes dropped before the composite (frames 1185/1188/1192)
These look like indirect-lighting (probe/irradiance cache) passes and a raytracing-resolve pass, by their role in the frame.

### "address not from a root parameter"
**f607780aa8e502d0 @0x384**: global_load_dword (1 dword), 64-bit VGPR-pair address, no SADDR.
- Table pointer from user_data 5 (high part 0); s_load_dwordx4 at +0x80 (dword 32) = the BUFFER descriptor of SRV table slot 32 (lower_bound 355, stride 4).
- Address taken from that descriptor's words: word0 & 0xffffff00 | 48, word1 & 0xff (bits 32-47). v_mov into 2 VGPRs, then global_load.
- i.e. a raw memory read of the buffer base (256-aligned) + 48, bypassing the buffer view. No phi, no s_cselect. Exec already narrowed by a v_cmpx before.

**0a5d4f9047b3dce4 @0x2b90**: s_load_dword (scalar), base SGPR from two v_readfirstlane of a VGPR pair.
- Base = s_getpc_b64 + literal 0x650 (+carry): a constant table embedded in the shader code (PC-relative).
- Index: sign-extended int << 2, added in 64-bit VGPRs, made uniform by readfirstlane.
- Index is a loop phi: 0 on entry, +1 per iteration; exit when > 5 or on a float test from a wave-wide min (DPP row_mirror + permlanex16 + readlane 32/0). So at most 6 reads of a 6-float table. PC base fixed before the loop. Loop body contains an s_or_saveexec -1 section.

### "lane-mask join of an untracked mask" (f018c205b5199d39 @0x4cd8, a1266cc7391ae0d6 @0x54c8, identical)
- The "mask" is a uniform boolean, not a v_cmp: index read from a constant buffer, readfirstlane; if >= 0, read a 100-byte record from another constant buffer (table user_data 4 at +0x320), compute (word0 | word2) != 0, s_cselect_b32 sX, -1, 0, v_mov into a VGPR.
- Join block, 2 predecessors: (i) direct jump when index < 0, VGPR set to 0 before s_cbranch_scc1; (ii) load path, VGPR = sX (-1/0), s_branch. A VGPR phi (0 | -1/0). No writelane/readlane/spill, no exec s_or/s_andn2 between predecessors.
- Reader at the failing offset: v_cmp_eq_u32 vcc, -1, vPhi; s_andn2 vcc, exec, vcc; s_cbranch_vccnz.
- Pattern: scalar -1/0 bool (s_cselect) carried through a VGPR phi, compared to -1 to branch.

### "whole-wave section step" (3a3c2b3f2364d207 @0x334, 5a51983c7d66607b @0x149a0, same pattern)
- Failing: buffer_store_format_xyzw (4x32-bit, not d16), data 4 VGPRs, index VGPR idxen, SGPR x4 descriptor.
- Before: buffer_atomic_add glc on a counter (constant index 20), result made uniform by readfirstlane. Then s_or_saveexec -1; v_cndmask (0 for inactive lanes); wave32 EXCLUSIVE PREFIX SCAN: v_add DPP row_shr:1/2/4/8; s_mov exec_lo = 0xffff0000 (upper half only); v_permlanex16 fi:1 + add; exec_lo = -1; subtract own value. Exec restored; add atomic base; original exec back.
- Per-lane result is the store index: compact "append" allocation.
- Differences from b22a16a1: wave32 (exec_lo) vs wave64; integer scan (row_shr) vs f16 reduction (dpp8 + readlane); has a partial literal exec mask (0xffff0000) around permlanex16; per-lane value feeds the store address; 32-bit format store vs d16.
- Hypothesis: the "exec = partial literal mask" step inside the whole-wave section is what the simulation doesn't follow.

---
## Wider windows (requested by triage thread)

### 55679a0547649d0b @0x3584 / 17af15c4015cb4b6 @0x2bfc (culling passes; same pattern, registers offset by 4)
- The SGPR later shifted by 22/14 and ANDed with 0xff000000 (s37 / s33) is the HIGH HALF of a pair used shortly before as a LANE MASK: s_cselect_b64 pair, exec, 0 on a scalar compare of a constant-buffer value; a second s_cselect_b64 exec, 0 OR-ed into the pair (s_or_b64); s_and vcc, exec, pair; branch.
- The pair is then overwritten twice: by s_load_dwordx4 (descriptor, table user_data 4 at +0xac0 in 5567), then by s_buffer_load_dwordx4 of a 12-byte record indexed by a counter (word1 also gives word1 >> 24 and word1 & 0x00ffffff).
- DIRECT writer of the shifted value: s_add_i32 high <- sA + sB, both s_buffer_load_dword from the same constant buffer loaded much earlier (~0x170 bytes before in 5567, ~0x440 in 17af). sB is also the loop bound counter (s_cmp_ge_u32 sB, limit).
- Between the s_add and the use: wave64 whole-wave section: exclusive scan DPP row_shr, exec_lo = exec_hi = 0xffff0000 around permlanex16, readlane 31, exec_lo = 0 / exec_hi = -1, then exec = -1. Preceded by a dpp8 reduction + readlane 32/0 and a single-lane atomic (s_ff1 + mbcnt + v_cmpx). Value then splatted (v_mov / v_or) into a VGPR stored at offset 8 of a 16-byte store in a loop indexed by an SGPR counter.
- Hypothesis: the "mask" tag on the pair survives the reloads and the s_add, or the whole-wave section with literal exec values confuses the value. The AND/shifts really read a sum of constant-buffer integers.

### f018c205b5199d39 (join @0x4cd8) / a1266cc7391ae0d6 (join @0x54c8): SGPR phis at the join
- Exactly 2 predecessors: P1 = s_cbranch_scc1 when index < 0; P2 = s_branch after loading the 100-byte record. No fallthrough.
- vcc is written in neither predecessor; the join recomputes it (v_cmp_eq -1, s_andn2 vcc, exec, vcc).
- s3 phi (the only mask-shaped SGPR): P2 = s_cselect_b32 s3, -1, 0 ((word0 | word2) != 0). P1 = word 3 of a BUFFER DESCRIPTOR loaded just before by s_load_dwordx4 s[0:3] from table user_data 4 at +0x960 (s0 then replaced by an s_buffer_load_dword from that buffer). In a126 a third upstream path ("index == 0xff") skips that load; s3/s0 then carry older values (goes through the v_readfirstlane's intermediate join).
- s0 phi: P1 = index dword from the constant buffer, P2 = record word 0; used right after by s_cmp_eq_u32 s0, 0 (not a mask).
- Other SGPRs defined only in P2 (s1, s2, s8/s48 or s36/s37, s[12:19], s[28:31], s[20:23]); on P1 they keep older values. No exec s_or/s_andn2 in predecessors: same exec on both sides.
- The mask actually read (v_cmp_eq -1 on the VGPR) is the splat of s3. Problem phi: VGPR(0 | splat(s3)), with s3 = (descriptor word | s_cselect -1/0) on the SGPR side.
