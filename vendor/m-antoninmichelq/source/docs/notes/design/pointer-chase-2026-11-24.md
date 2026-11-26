# Pointer chase: reading title memory at addresses the shader computes from data

Shaders: 9205d265 (s_load at (loaded<<8)|12), 6f6a359d @0x320 (global_load at (loaded<<8)|16/28),
caf96a32 / f607780a (BVH node pointers), 190f/198f/cb96 (BVH builders), ~25 "address not from a root parameter".
Today: refused at translation, so the culling/BVH computes they belong to never run.

## Idea
The host keeps a table of the title-memory windows it mirrors (shadow copies), and the shader
looks a computed address up in it, then reads through a dynamically indexed raw view.

Host (victory-gpu):
1. A "window table" buffer: sorted entries {start_lo, start_hi, len, srv_heap_index} for every live
   shadow window, rebuilt when windows are added/released (generation counter), uploaded once per
   change, bound as a root SRV at a reserved slot (POINTER_SLOT + 127, say) for pipelines that ask.
2. For each entry a raw SRV over the window (start rounded down to 16, as root views now) in the
   shader-visible view heap (static slots, freed with the window).
3. Before a dispatch that declares the table: refresh dirty pages of all listed windows (dirty
   tracking makes clean ones free). Reads only in v1: no write-back of chased stores.

Shader (victory-shader):
1. resource.rs: an address that is not from a root parameter but a 64-bit value the program computes
   becomes Address { param: CHASE, dynamic: lo/hi values } instead of an error (loads only).
2. lower.rs: for such a load, emit a binary search over the table (<= 16 steps, a loop) giving
   (index, start); out of every window -> zeros. Then createHandleFromHeap(index, non-uniform)
   and rawBufferLoad at addr - (start & ~15). Needs SM 6.6 dynamic resources (RX 7600: yes).
3. Kill switch VICTORY_NO_POINTER_CHASE=1; part of the shader cache key.

## Risks
- A target never mirrored (no shadow) reads zeros: same as a null view today.
- Cost: one binary search per chased load; fine for culling/BVH sizes.
- Stores/atomics through chased pointers stay refused (v2: UAV twin + write-back of touched windows).

Owner: translation thread ("Afficher la scène du monde"). Host part could be split to another thread.
