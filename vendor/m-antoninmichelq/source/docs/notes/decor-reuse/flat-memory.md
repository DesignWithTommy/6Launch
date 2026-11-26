# Flat-memory fallback for pointer chasing (9205d265): how xcl maps memory, and what VIctory has today

Date: 2026-11-24.
- xcl: ivall/xcl main 930e8b4. This part gives ideas and facts only, in our own words; xcl is
  GPL-2.0-only.
- VIctory: /home/user/victory, branch victory-stack-d13z4, commit 72df0f2. Read only; nothing
  was changed.

## 1. How xcl lets a shader read any GPU address

### Where the memory comes from
xcl uses **both** ways, as fixed "windows" of 4 GiB each.

- **The memory block.** `gpu/uma.rs::for_runtime_with` (~l.164–205) reserves one 16 GiB range
  in the CPU address space, below 2^40.
  - Windows 0–2 are **reserved only**, with write watching on.
  - Window 3 is **committed**, with write watching on. Its size comes from the last run (at
    least 512 MB).
- **Window 3, the coherent one.** It is opened with
  `ID3D12Device3::OpenExistingHeapFromAddress`. Limits xcl measured (HANDOFF l.140–147):
  - It must be one fully committed `VirtualAlloc` allocation. Partly committed memory or a
    placeholder is refused.
  - It must be under 4 GiB: 4095 MiB works, 4 GiB is refused.
  - Opening a 4 GiB window takes about 0.5 s.
  - It costs its whole size in commit.
  - It holds xcl's own pool (per-draw arguments, root-argument tables, readback). Since PR #40
    it also holds the game's heaps when they fit.
- **Windows 0–2, the sparse ones** (`hle/d3d12x/sparse.rs`, module doc l.1–17).
  - On the GPU, each window is a 4 GiB *reserved* buffer. Its 64 KiB tiles are mapped with
    UpdateTileMappings onto 64 MB video-memory pool heaps as the game commits pages.
  - The game's memory and GPU heaps go here: XMem graphics allocations, physical pages, D3D12
    heaps and the console descriptor heaps (HANDOFF l.1423–1430).
  - Window 0 starts with a 512 MB range kept for the 32-bit descriptor heaps.
  - Buffers placed here get reserved resources over the same tiles, so copies, views and
    barriers all see the same memory.
- **What goes into the windows.**
  - Graphics `VirtualAlloc` / `XMemAlloc` (memory type not 0), and large reservations of 64 KiB
    pages.
  - Everything else is plain memory the GPU cannot see. Reads there return 0. HANDOFF l.1751
    counts about 4.8 GB of such memory in the retail game.

### How a shader turns an address into a buffer offset
- The low 32 bits of the address are the byte offset in the window's `RWByteAddressBuffer`.
- The window is (high 32 bits) minus the block base's high half, which is passed in an `io`
  slot.
- The full 64-bit address is computed first, so additions keep their carry across 4 GiB
  boundaries.
- Each window is bound as a root UAV at its base. The bases are aligned, so there are no
  alignment problems.
- A buffer that does not start on 64 KiB is placed from the 64 KiB boundary below it. The gap
  is added back in copies and in ExecuteIndirect (`buffer_delta`).

### Keeping CPU and GPU in step
- **Coherent window:** this is real shared memory, so nothing is copied.
- **Sparse windows:** before each ExecuteCommandLists, xcl reads the write-watch bits and copies
  only the 64-byte chunks whose hash changed, through a staging buffer on its own queue. The
  game's queue waits on that copy.
  - Copying only changed chunks avoids overwriting counters or indirect arguments the GPU
    wrote on the same page.
  - GPU writes are **not** copied back to the CPU view. Only Write32/64, Fill, copies,
    timestamps and Map-with-read are.
- **Cost:** the per-submit write-watch scan of about 2 GB took about 0.8 ms per submit. The
  window `switch` in shaders roughly doubled DXC and driver compile time.

### GPU caveats xcl found
- The AMD driver reports the 64 MB pool heaps as private write-combined process memory, and
  that counts against commit.
- Tiles mapped onto heaps opened from file mappings removed the device: "do not try again".
- `OpenExistingHeapFromFileMapping` works on SEC_COMMIT sections and is refused on SEC_RESERVE
  ones (HANDOFF l.1642).
- The game's memory must stay below 2^40: the BVH builder refuses addresses above it, and the
  exe is built without high-entropy ASLR (02bfd1d).
- UAV barriers are made global: AMD drivers flush for any UAV barrier, NVIDIA's need not.
- Nothing about the RX 7600 was found.
- **Plan xcl adopted but has not built yet** (HANDOFF l.1640–1656): a GPU page table.
  - Reservations become placeholders; each committed 2 MiB chunk becomes a SEC_COMMIT section
    opened as its own heap, with a raw buffer over it.
  - A table from address chunk to descriptor index is passed to shaders, which read through
    `ResourceDescriptorHeap[i]`.

## 2. VIctory today: how game memory reaches the GPU

### Allocation (`crates/victory-hle/src/modules/xmem.rs`)
- `XMemVirtualAlloc` (l.88–181): uses `VirtualAlloc2` with address limits (4 GiB up to
  2^40–2^46, `address_range` l.79). Reservations of 64 KiB pages become **placeholders**
  (l.113–125). Other reservations get `MEM_WRITE_WATCH` (l.133–138).
- `XMemAllocDefault` (l.281): a Rust-heap block, above 2^40 (shortcut at l.294).
- Physical pages (l.340–520): pagefile **sections** (`CreateFileMappingW`, **SEC_RESERVE**
  outside Wine, l.414–421) mapped into placeholders (`map_view` l.746). They cannot be
  write-watched; they are guarded per 64 KiB instead (`victory-gpu/src/watch.rs` doc).

### D3D12 objects (`crates/victory-gpu/src/resource.rs`)
- A resource's "GPU address" is its CPU address (module doc).
- `create_heap` (l.796) allocates with `Memory::new` → `alloc_zeroed` (l.768), from the Rust
  heap.
- Committed resources use the game's allocator hook (`Memory::for_resource` l.732; `ALLOCATOR`
  l.688).
- `create_placed` (l.1587) places at the heap address plus the offset.

### How the GPU sees it: one host copy per buffer
- `shadow.rs`:
  - `find` (l.781) gives the copy of the innermost registered buffer. Loose memory outside any
    buffer gets copies of 4 MiB windows (`loose`, l.981).
  - `allocate_buffer` (l.841) → `host.rs::shared_buffer` (l.3824): a CUSTOM write-back L0
    buffer with UAV.
  - Copies are refreshed from the write watch (`update` l.468) and written back after
    submission (`gpu_writes` l.283, `copy_back` l.375).
- Root views: `draw.rs::root_view` (l.231–270) rounds SRV/UAV addresses down to 16 bytes and
  binds `shadow.gpu(address)`. The shader adds the low bits back (`lower.rs::view_offset`
  l.2797, commit 72df0f2).
- Shader side: `victory-shader/src/resource.rs` (doc l.1–35) traces each address back to a
  root parameter. When that fails it refuses with "a typed root view" (`lower.rs` l.6810), "an
  address in root constants" (l.2812 / l.6741–6765), and so on.

### What prevents a flat fallback today
- The game's memory is not in a few GPU-visible ranges. It is spread over the Rust heap,
  `VirtualAlloc2` regions and section views.
- The GPU only sees **per-buffer copies at different addresses**, refreshed only for the ranges
  the host *knows* a submission reads.
- A pointer chase reads ranges nobody declared, so it would see stale or missing copies.

## 3. What would need to change in VIctory

**Option A: xcl-like windows (bigger change, most general)**
1. Add a block module in victory-gpu that reserves 1–4 windows of up to 4095 MiB, aligned to
   4 GiB, below 2^40.
   - Committed windows are opened with `OpenExistingHeapFromAddress`, with one raw UAV buffer
     placed over each.
   - Sparse windows, if commit cost matters, use reserved buffers plus tile mappings and
     write-watch chunk uploads, as described above.
2. In xmem, route graphics `XMemVirtualAlloc` and `XMemAlloc(attr≠0)` into the windows.
   Physical-page sections:
   - either switch them to SEC_COMMIT and open each with `OpenExistingHeapFromFileMapping`
     (one heap per section, reached through a descriptor index);
   - or copy pages into window memory, as xcl's "home" pages do.
3. Make `resource.rs::create_heap` (l.819) and `Memory::new` (l.768) take memory from the
   windows instead of `alloc_zeroed`. Place committed resources there too.
4. For addresses in a window, root views, ExecuteIndirect arguments and copies become offsets
   into the window buffer. The shadows remain only for memory outside the windows.
5. Translator fallback: when tracing fails, emit `windows[(va>>32) - base_hi].Load(lo32)` with
   the windows bound as root UAVs. Addresses outside the windows read 0.

**Option B: a page table over the existing copies (smaller, no change to the memory model)**
- Give every copy a UAV in a reserved part of the view heap.
- Upload a table from `va >> 16` to (descriptor index, offset of the copy's start) for the
  registered buffers. Fallback loads go through `ResourceDescriptorHeap[idx]`.
- Catch: the pages the shader may chase into must be fresh. Refresh whole registered buffers
  (write-watch makes that cheap: only the written pages are copied), and write back every UAV
  the fallback can write.
- This keeps VIctory's design, and it is close to the page-table plan xcl adopted for itself.
