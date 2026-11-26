# Handoff (2026-11-26)

Where Flamingo stands, what was found, and what to look at next. Measured on one PC: Windows 11,
NVIDIA RTX 4070 12 GB (32-lane waves, a capped 1,000,000-slot view heap), Ryzen 7 7800X3D,
32 GB. All frame rates are medians over a scripted run through the first mission's boat
cutscene (Series S profile unless said otherwise).

## Playable settings

About **9-10 fps** in the boat cutscene (run 193: 9.8 fps, 102 ms a frame: 46 ms preparing,
29 ms on the GPU):

```
set VICTORY_NO_WATCHDOG=1
set VICTORY_PIPELINE_WAIT_MS=24
set VICTORY_RELAY=1
set VICTORY_LATE_WAIT=3
set VICTORY_CARD_BUFFERS_MIN_MB=1
set VICTORY_CARD_BUFFERS_MB=1024
set VICTORY_INDIRECT_WINDOW=0
set VICTORY_SHADER_OPT=0
set VICTORY_WIDE_MERGE=1
set VICTORY_WIDE_TEXTURES=1
set VICTORY_VIEW_GUARD=0
set FLAMINGO_BVH_MISS=1
set VICTORY_HEAP_WORKER=1
set VICTORY_AMBIENT=0.6
set VICTORY_SKIP_PIPELINES=7115e46a79c78f98,59b1ba494f18c894,b8ca32a6899b87fb,659fe9cae59dbd88
target\diag\victory.exe run <your dump>\<the title's executable> --execute --console series-s
```

For water without the dark blocks, add `,5dc3b25986622949,6f225b641c5b0708` to
`VICTORY_SKIP_PIPELINES`: the water's transmission volumetrics are left out, and the water turns
grey instead of turquoise (run 221; see Water below).

What each step gave, in order (same scenes):

| Step | fps | |
|------|-----|-|
| Series X profile (2560x1440 targets) | 5.4 | |
| Series S profile (1280x720, the title upscales) | 7.1 | GPU 65 to 35 ms a frame |
| `VICTORY_HEAP_WORKER=1` | 9.1 | textures picked by index uploaded by a thread of their own: the submitting thread's texture pass went from 16 ms to 0 |
| `VICTORY_AMBIENT=0.6` and the impostors left out | 9.8 | |

`VICTORY_TEXTURE_NO_WAIT=1`, `VICTORY_RELAY=2`, `VICTORY_WIDE_BACK=1`,
`VICTORY_CARD_REHASH_RUNS=1`, `VICTORY_JOIN_COPIES=1`, `VICTORY_JOIN_PARTS=1` and
`VICTORY_LEVELS_ON_DEMAND=1` together gave nothing more here (9.0 against 9.1).

The submitting thread is now the limit (about 45 ms of preparing a frame). The faster forks
(avatarmanz/VIctory-7900XTX's speed work, victory-fork main's later commits) target exactly that.
victory-fork main is 79 commits ahead of this tree; a trial merge conflicts in 22 files.

## Fixed this round

- **Pipeline syncs order shader writes.** The console's pipeline syncs (list slots 0x408 and
  0x410; 0x418 and 0x420 in newer titles) wait for earlier work. They were no-ops ("host lists
  order their own work"), which is wrong for D3D12: a host list does not order its own
  dispatches. They are now a barrier on every shader write (`FLAMINGO_SYNC_BARRIER=0` restores
  the old behavior). Since then a character whose clothes came and went from run to run has
  been drawn whole in every run.
- **Faces move.** The title's face network keeps half floats in `R16G16B16A16_UINT` and
  `R16_UINT` views, in descriptor tables, and moves them with `_d16` typed accesses. They were
  converted as halves through a float handle, so its output was garbage. The view's format kind
  now rides in the table of strides beside the stride, also for views in tables (VIctory
  4243a96a by way of avatarmanz/VIctory-7900XTX 9a6c829d and that fork's #114).
- **The impostors are left out** (see below), which removes the biggest artifacts.
- **The heap buffer pass looks at a third fewer slots.** The pass that makes the view heap's
  buffer views ready for shaders that pick them by index marked every slot it made for the next
  pass, which looked at each of them again for nothing (after victory-fork #1043): 14,089 slots a
  frame looked at against 9,111 now, the pass 31.6 ms against 27.0 (runs 216, 220). It still runs
  on the title's recording thread at about 2.6 us a slot, so it is the next speed target
  (victory-fork #1190 and #1068 have more of it).

## Open: artifacts

### Impostors (distant trees and buildings)

The title's octahedral impostors draw as huge flat panels on this layer (olive or green, covering
much of the screen in some boat shots), and the water reflects them as dark rectangular blocks.
Leaving out their four pixel programs (the `VICTORY_SKIP_PIPELINES` list above) removes the
panels, most of the dark blocks on the water, and the dark boxes that floated over the beach; it
also brought back a character's body in several shots. Far trees and some far buildings are then
missing.

| Programs | What they draw | Left out |
|----------|----------------|----------|
| `MS_VS_OctaSLODWind` (`c89489afd89ee7f0`) + `PS_OctaSLODReZ` (`59b1ba494f18c894`), `PS_OctaSLODFireReZ` (`b8ca32a6899b87fb`) | the big flat panels | runs 190, 191 |
| `VS_OctaWind` (`b3ba0ef9ee352720`) + `PS_Octa` (`7115e46a79c78f98`), `PS_OctaVsplit` (`659fe9cae59dbd88`) | the dark blocks on the water, a body hidden | run 190 |

Suspect: the passes that build the impostors' instance lists. `CS_Tree_BatchCull_ImposterOnly`
(`89d08e561897d4e5`), `CS_Tree_BatchCull` (`55679a0547649d0b`) and `CS_Prop_BatchCull`
(`17af15c4015cb4b6`) are wave64 programs that, on 32-lane hosts, take the "local waves (idiom)"
form, whose report says "an LDS read and write in one lane-branch region": the two host halves
of a console wave can race on group-shared memory. `VICTORY_WAVE_LOCAL=0` puts all three in the
exact paired form: a hidden body came back, but the dark water blocks stayed and a long spike
polygon appeared, and the GPU time went up (run 192). So that is part of it at most. The
impostor programs themselves translate in their original form (the mesh one has ten
`s_andn2_wrexec_b64` waterfall loops and 22 `v_readfirstlane_b32`).

### Characters

**Newest (runs 204-219):** the meshes that go wrong come out of the skinning passes
(`CS_PedStandardSkinning_*`) wrong, the same meshes in every frame. Dumped right after a skinning
pass (`FLAMINGO_DUMP_AT` on direct dispatches), a few meshes it has just written hold coordinates
far off or not finite: in one capture one mesh of the `_6` variant (`a97f7e8b86eb064f`) and two of
`_9` (`ebe4e53f031f5249`), while other dispatches of `_9` are right. Each pass picks its geometry
buffer (eight vertex streams: weights, bone indices, a half-float scale with 14 more channels)
from the view heap by an index in its constant buffer (word 16) and a bone-remap buffer by
another (word 17). For the wrong mesh those views are made, at the right size, and the card's
copy of both buffers matches title memory byte for byte (`FLAMINGO_DUMP_SLOT_WORD=64:16,64:17`,
run 219). So the data is right and the translated program, or the driver, gets it wrong. Read
and found right: the typed 8_8_8_8 loads (format 56), the half conversion, the legacy multiplies
(0 x anything = 0), the source modifiers. Next: replay that one dispatch with its dumped inputs
on WARP and on the card, which tells translator from driver.

The same family shows in the opening cutscene: one character is drawn without the skin of her
head (teeth and the outline of eyes and glasses drawn), and a man's body is coarse facets with his
hair as shards.

**Runs 229-235** narrowed it further. The bone palette (`s0+8`, 48-byte rows) holds sane
matrices for the wrong meshes too, and the `_6` mesh that was wrong in run 205 came out right
when dumped just after its own pass (run 230). Which meshes are wrong changes from run to run and
does not follow the program variant: of two instances of one model, one can be right and the other
wrong. A few meshes' geometry buffers hold decoded vertex streams on the card while title memory
keeps an encoded container (a header of tagged chunk offsets); those come out right, so the
decoding is on the card and the card copy is the one to keep. What is left: something between the
skinning and the draws (the order of lists, or a write after the pass). Dumping the outputs needs
the copy after the last skinning pass: the first pre-skinned cull of a frame is recorded before
almost all of its skinning.

**Run 236, the clearest so far:** with the skinning outputs copied after the frame's last
skinning pass (one frame, 125 passes), 14 meshes come out wrong, and each of them reads a geometry
buffer whose card copy does not hold decoded vertex streams (one holds floats in the hundreds, one
a fill pattern, one what looks like another mesh). For every such buffer title memory holds an
encoded container (a header of tagged chunk offsets, `42 40 00 00 26 ...`); the 15 that come out
right have the decoded streams on the card. So the decoded data reaches the card some other way
than through title memory (a decode pass at load time, or a copy from an upload buffer the
op dump does not show), and for those 14 it did not arrive. `VICTORY_PAGE_WRITES=all` does not
change it (run 228). Next: record the title's buffer copies in the op dump and capture the frames
where the characters stream in, to find what writes those buffers and why it reads stale data.

Earlier notes, from before those runs:

One character's body is still missing in most shots of the boat cutscene: head, neck, arms and
legs drawn, torso and shirt not (seen on every captured frame of the boat, runs 193-203).

What the pre-skinned meshlet cull (`CS_BatchCullPreSkinnedMeshlets`) does, from its code: it
reads vertex i of a mesh at the mesh's skinned offset + 32 + i x stride (float3, 24:24:16 or
10:10:10, each axis mapped onto -3..3 m), places a vertex left all ones (the skinning's "not
written") at the mesh's origin, moves it by the instance matrix and two 4x4 matrices, and keeps
a meshlet when any of its vertices lies inside the view (|x/w| < 1, |y/w| < 1, z/w > 0); the
group ORs that in LDS between two barriers.

What it reads (run 201: the skinning outputs and the batch ring dumped right after the main
cull, `FLAMINGO_DUMP_AT`): most meshes hold plausible skinned vertices (about +-0.4 m round the
instance), some with 15-62 % left all ones (likely skin under clothes, hidden on purpose). A few
hold stale data: one mesh coordinates around 100,000 m, one all zeros, one every vertex at one
point; the cull drops those. Each of them is written by a skinning dispatch recorded *after* the
cull, in a list the title submits only after the next present; that list waits for the cloth
simulation's completion value, and the cull's list is held behind a list that waits for a value
of the compute queue's water simulation. Keeping held work across presents
(`VICTORY_WAIT_FORCE=0`, run 202) did not bring the body back, so the order the title expects
between those lists is the open question. Next: capture four windows in a row and replay the
queues' order offline.

Note for dumps: the skinning outputs' title addresses change from run to run (0xfb2600000 and
0xfb5800000 in runs 181-184; 0xac6160000 and 0xac9360000 in run 201): dump them by size,
`FLAMINGO_DUMP_RANGE=size:3200000,size:1900000,size:ae00000`.

What was ruled out earlier, with the data:

- The skinning output is consistent. With `FLAMINGO_DUMP_RANGE` (below) the two skinning
  buffers were dumped at frames 2850 and 3000: everything the frame's skinning dispatches write
  (output word: dword 13 of their 96-byte constant buffer, offset in 16-byte units in its low 24
  bits) changes between frames, the rest is old data no mesh points at.
- Every mesh the pre-skinned culls dispatch (from the per-group dwords at `t77[2]`) points at
  skinned data written that frame; the culls' mesh records (`t77[0] + 24 * mesh`, the skinned
  word at +12) and the instance matrices (`t77[1] + 48 * instance`) on the card match title
  memory at the frame's present.
- Not occlusion culling (`FLAMINGO_GATHER_ZERO=1`: the pyramid reads 0), not overlap between
  command lists (`FLAMINGO_LIST_BARRIER=1`), not the relay (`VICTORY_RELAY=0`), no dropped draws.
- About 55-60% of the character mesh draws run with no meshlet groups; many of those are
  probably empty legitimately (level-of-detail variants), so that count is not the measure.

What is left: the cull's per-meshlet decision for that character's meshes (frustum test on the
skinned vertices, the optional back-face test), or the draws themselves. Next step: map each
colour draw's record to its mesh and character, and test that character's meshlets on the CPU
from the dumps.

More on the impostors (runs 192-195, judged on every captured frame, not six): with all 17
programs that take a local-wave form put in the exact paired form (`VICTORY_WAVE_LOCAL=0`) no
full-screen olive panel showed in 20 frames; with only the four batch culls exact (194) it
showed in three, with only the four tile classifiers exact (195) none but many large dark
polygons. Not pinned yet.

### Other

- **Water:** the dark blocks come from the water's transmission volumetrics,
  `CS_TransmissionVolumetrics_Tiled` (`5dc3b25986622949`) and `_Tiled_LocalLights`
  (`6f225b641c5b0708`): with both left out the blocks are gone and the water turns grey (run 221).
  Their wave form is not the cause: in the exact paired form (`VICTORY_WAVE_WIDE_ONLY` without
  them; by default they take the wave-wide form, after victory-fork #823) the turquoise and the
  blocks both come back (run 222). So one of their ~40 inputs is wrong; the sun's shadow atlas
  they read (640x2560, four cascades) looks right. victory-fork reached the same two programs
  (its commit 9613092b: "gives the water its colour but leaves dark patches on it").
- Dark blocks on the horizon remain in some shots even with the impostors left out; a large dark
  triangle sticks up from the boat in one shot; one shot shows broken shards.
- **Dark faces at camera cuts:** a face drawn dark in the first frames of a shot is right a few
  seconds later (the opening cutscene, runs 223-228): its textures arrive late. `VICTORY_PAGE_WRITES=1`
  and `=all` change nothing (runs 227, 228).
- **Textures waiting for room:** from about a minute in, 5,400-11,000 textures picked by index wait for room on the card for the rest of the run (the store keeps to 90% of its budget, `VICTORY_TEXTURE_ROOM_PERCENT`; run 223's log). A texture that never gets there draws dark, which may be part of the dark faces and black silhouettes; to test next.
- `VICTORY_RELAY=0` turns skin grey and makes eyes glow: do not use it for picture checks.
- The water's GPU-written textures are copied back by default (`texback.rs`); with
  `VICTORY_TEXTURE_BACK=0` the boat sits on the seabed. Keep it on.

## Speed switches tried this afternoon (same scenes, Series S)

| Switches | fps | |
|----------|-----|-|
| the README's settings (run 193) | 9.8 | |
| + `VICTORY_CARD_BUFFERS_MIN_KB=64 VICTORY_CARD_BUFFERS_MB=1536` (run 203) | **11.1** | the card's time 13 ms a frame against 28; picture as mixed as before. From avatarmanz/VIctory-7900XTX's issue #213 (18 fps on an RTX 4070 in another tree) |
| + `VICTORY_WAIT_FORCE=0` (run 202) | 10.6 | held work kept across presents; can stop the title for good if a wait is never met |
| + `VICTORY_LATE_WAIT_LISTS=512 VICTORY_MERGE_RUNS=4096` (run 197) | 9.0 | much worse picture |
| + the 1,034 ray-traced resolve programs skipped (run 206) | 10.4 | 7 ms less preparing; no artifact gone |
| the heap buffer pass fix (run 220) | 10.0 | the pass 31.6 to 27.0 ms a frame |

## Tools added this round

- `FLAMINGO_DUMP_RANGE=<hex address>+<hex bytes>[,...]`, or `size:<hex bytes>` for the title
  buffer of that size wherever it lies (diagnostics builds, with `VICTORY_CAPTURE` and
  `VICTORY_CAPTURE_FRAMES`): `frame-N-range-<address>.bin` is the range as its host copy held it
  at the frame's first character mesh draw, `frame-N-title-<address>.bin` as title memory held it
  at the frame's present.
- The capture's op lines give a constant buffer of up to 128 bytes in full, and mesh pipelines'
  depth, stencil, cull and blend state; `frame-N-drawn.txt` gives each indirect record as the
  card ran it and as title memory held it while the command was recorded.
- `shader-info --disasm` limited to `FLAMINGO_DISASM_HASHES=<hash prefixes>`.
- Test switches (off by default): `FLAMINGO_GATHER_ZERO=1` (gathers at a named level read 0:
  occlusion off), `FLAMINGO_LIST_BARRIER=1` (a barrier at the start of every list).
- `VICTORY_SKIP_PIPELINES` works in every build.
- `FLAMINGO_DUMP_AT=<part of a pipeline's name>`: the ranges are dumped right after the first
  indirect command of that pipeline in a listed frame (the main character cull:
  `CS_BatchCullPreSkinnedMeshlets_b589f3cd`), not at the first character draw.
- `shader-info --disasm` prints each program's wave size and stage I/O (threads, LDS, mesh
  limits).
- `translate --warm` of one hash with `VICTORY_WAVE_WHY=1` and a scratch `VICTORY_CACHE_DIR`
  prints which wave64 form a program takes on a 32-lane host.
- `FLAMINGO_DUMP_SLOTS=<index,...>` and `FLAMINGO_DUMP_SLOT_WORD=<table dword>:<word>,...`
  (diagnostics builds, with a capture): at the pipelines `FLAMINGO_DUMP_AT` names, the op lines
  describe those view heap slots as the title wrote them and as the host made them, and the range
  dump takes their buffers (card copy and title memory). `64:16,64:17` at
  `CS_PedStandardSkinning` gives the skinning passes' geometry and bone-remap buffers.
- `FLAMINGO_DUMP_TABLE_AT=<part of a pipeline's name>`: the range dump also takes every buffer that
  program reads or writes through its tables (up to 64 MiB each), and the `FLAMINGO_DUMP_RANGE`
  ranges are taken there; each range is taken once a frame, at the first program that asks. At each program
  `FLAMINGO_DUMP_TABLE_AT` names the `FLAMINGO_DUMP_RANGE` ranges are copied again into the same
  read-back buffer, so the file holds them as the frame's last such program left them.

## Elsewhere: projects worth reading

Searched on 2026-11-26 among the Xbox Series compatibility layers on GitHub (about 70; the ones
named after the title's executable or promising a "PC edition" look like scams and were not
opened):

- ZigZagBoi1/ScarlettPort (license not stated: ideas only): links mesh programs whose outputs
  pass D3D12's 32 KiB (this tree does too) and sizes the view heap for NVIDIA (here the title's
  1,245,184 slots meet a host cap of 1,000,000; run 155 found shaders almost never pick past it).
- alexdeliu/rdna2-conformance (MIT): driver probes for console RDNA2-to-DXIL translators, with
  vectors from this lowering; NVIDIA results are asked for. A way to tell a driver bug from ours.
- californiaAlmonds/OpenScarlett (MIT, C++20): its own GFX10 translator, another reference.
- fxceless/victory-fork #1043, #1190, #1068: more of the heap pass speed-up; upstream VIctory
  #1567: skinning that read zeros through null heap slots (handled here already).
