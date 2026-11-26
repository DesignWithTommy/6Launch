# Failing store operands and f018c205 join (pattern notes, words/mnemonics only)

## 55679a05 @0x3584 / 17af15c4 @0x2bfc: buffer_store_dword
- data v22/v19, index v23/v20 (idxen), descriptor s[16:19]/s[12:15], soffset const 0, offset 12. First of 4 stores (offsets 12,4,0,8) in a small loop (back-edge 0x35a4->0x3568 / 0x2c1c->0x2be0).
- Other data: off 4 v21/v18 = v_mov s38/s34; off 0 v20/v17 = v_mov s36/s32 (low half of pair, s_or_b32 packed); off 8 v19/v23 = v_or of s37/s33 (high half, <<14, &0xff000000) with a VGPR.

## Writer chains
- data off12: v_lshl_or(counter s47/s41, 6, v18/v15); v18/v15 = v_lshlrev 16 of v16/v13 = buffer_atomic_add glc return (init v_mov 1). s47/s41 = loop counter (s_mov 0, s_add +1): loop phi.
- index: v_add(s47/s41, v17/v14); inner v17/v14 = exclusive DPP scan of the whole-wave section; old s47/s41 = v_readfirstlane of a single-lane atomic return.
- descriptor: s_load_dwordx4 from s[52:53]/s[48:49] + 0x40; low = s_mov of user SGPR s5, high = s_mov 0.
  BEFORE the load, the descriptor's low pair was used as a MASK: 5567 s_mov_b64 s[16:17], exec (0x2e58) ... s_mov_b64 exec, s[16:17] (0x2f10), load at 0x2fac. 17af s_mov_b64 s[12:13], exec; s_and_b64 exec, s[0:1], s[12:13]; s_andn2_b64 s[12:13], s[12:13], exec in a waterfall loop (0x24d4->0x2530), load at 0x26b8. 5567 outer back-edge 0x40a0->0x2940 encloses both.
- high half s37/s33: s_add_i32 of two CB dwords; before that the pair was a mask (s_cselect_b64 exec,0 + s_or_b64 + s_and_b64 vcc, exec, pair), then reloaded.

## Searched items
- VOP3 carry-out to an SGPR pair: none in chains (only v_add_co_ci with vcc on v10/v9, earlier, off-chain).
- v_writelane: none. v_readlane: only whole-wave reduction ends into s51 (5567), s40/s48/s49 (17af). In 17af s48/s49 (descriptor table ptr) are overwritten by v_readlane at 0x280c,0x2818,0x28c0,0x2b30, after the descriptor load (not re-read).
- 64-bit pair reads: (a) descriptor low pair read as 64-bit mask before reload; (b) high-half pair read as 64-bit (s_or_b64, s_and_b64 vcc) before s_add, rewritten after store loop.
=> Strong candidate: the descriptor is loaded into registers that were an exec copy / waterfall mask just before ("lane mask holding lanes elsewhere read as data").

## f018c205 join @0x4cd8 (wave32)
- Preds: P1 = s_cbranch_scc1 at 0x4c68 (index < 0); P2 = s_branch at 0x4cd4. Upstream of P1 a jump at 0x4c20 (s_andn2_b32 vcc_lo, exec_lo, s50; s50 32-bit mask) to 0x4c60 skips the v_readfirstlane, so s54 is itself a phi.
- Phi args: s3: P1 = word 3 of descriptor (s_load_dwordx4 s[0:3], table user_data 4 +0x960) or older value via s50 jump; P2 = s_cselect_b32 s3, -1, 0. s0: P1 = CB index dword; P2 = record word 0. s1,s2: P1 descriptor words 1,2; P2 record words 1,2. s54,s14,s6: P1 entry values; P2 recomputed. s[12:19], s[20:23], s[28:31], s8, s48: only defined in P2. exec/vcc untouched nearby.
- After join: s3 not re-read (rewritten at 0x4cf8 s_and_b32 s3, s19, const). s0 read at 0x4cec s_cmp_eq_u32 s0,0, then shifted as s_load_dwordx8 offset, then read bytewise (v_cvt_f32_ubyte0-3). The mask actually read is VGPR v51 (phi of 0 | splat of s3) by v_cmp_eq_u32 vcc_lo, -1, v51 at 0x4cd8, then s_andn2_b32 vcc_lo, exec_lo, vcc_lo, s_cbranch_vccnz. No 64-bit read of a joined pair.
