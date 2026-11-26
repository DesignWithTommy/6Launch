# xcl ideas for the culling / indirect-arg computes (ideas only)

Date: 2026-11-24. Sources: ivall/xcl main at 930e8b4, plus RipcuBAN/xcl-openworld (32a526a),
delicoindev/xcl-compat-fixes (f3dca3b) and w1spo/xclayer (50062cf). The GitHub API was not
reachable from this session, so other forks were not listed; the repos were cloned blobless
under /tmp/claude-0/xcl/.
xcl is GPL-2.0-only. This note gives techniques and facts in our own words only. No code is
copied from it.

## Ranked techniques

### 1. One flat memory model: every address is a byte offset into a raw root UAV (most useful)
- **Where:** `crates/xcl/src/gpu/recomp/emit.rs`. SMEM handling is around l.1480–1570,
  `emit_flat_addr` around l.1597, and `window_read` / `window_write`, `windowed` and
  `mem_accessors` around l.5085–5170. The root signature is `pso.rs::abi_root_signature_with`
  (~l.656), and the binding is `hle/d3d12x/bind.rs::bind_abi` (~l.856). Notes are in
  `gpu/uma.rs` (module doc) and HANDOFF.md l.140–147.
- **How it works:** The translator never asks where an address came from. SGPRs and VGPRs are
  ordinary HLSL variables, and every memory access works the same way:
  - `s_load_*`, `global_*`, `flat_*`, buffer ops (base from the V# held in registers) and atomics
    all compute an address and use it.
  - The address is `base_hi:base_lo + offset`. Its low 32 bits are the byte offset into one
    `RWByteAddressBuffer` called `mem`.
  - So a phi or loop-carried address, a pointer loaded from memory and then followed, a
    64-bit VA in root constants, and an address not taken from any root parameter all work with
    nothing special.
  - Typed buffer access (`buffer_load_format_*`) is decoded inside the shader from the V# format
    field (78 GFX10 formats). So no typed D3D12 view exists at all.
- **Memory windows (when the title's memory is larger than 4 GiB):** Each memory window is
  bound as a root UAV (`u2..u5`).
  - The window index is the high dword minus the UMA base, sent in an `io` slot.
  - The effective address is computed in 64-bit, so carries across 4 GiB are kept (commit
    f9e5ee9).
  - Each access becomes a `switch` on the window.
  - An address outside the windows reads 0, and stores to it are dropped. A null high dword
    maps to window 0 for reads only (32-bit table pointers).
  - Cost: the switch roughly doubled DXC and driver time on big programs (HANDOFF ~l.1776).
- **How it maps onto VIctory:** When the "tie to a root param" analysis fails (phi, pointer
  chase, root-constant address), do not refuse. Fall back to a global path:
  - Bind one big raw UAV (root UAV, so no descriptor size limit) over a heap that holds every
    GPU-visible title allocation.
  - Emit `Load/Store/Interlocked*` at `(va - heap_base)`.
  - If title VAs cannot be placed at fixed offsets, add a small VA→(buffer, offset) lookup in
    the shader, such as a page table indexed by `va >> 16` in an SRV, or a few
    range-compare windows.
  - Keep the precise per-root-param path for the cases already handled. Use the global path
    only for the families that fail.

### 2. Root arguments become raw SGPR words, so there are no views and no alignment issues
- **Where:** `hle/d3d12x/console_list.rs` (`user_data_for`, `sgprs`, `inline_for`,
  ~l.142–177) and `bind_abi`.
- **How it works:**
  - The title's root arguments (constants, 8-byte root descriptor VAs, 4-byte table handles)
    are packed in root-signature order.
  - The first N go straight into user SGPRs. The rest are copied to a small UMA block, and
    s[0:1] points to it.
  - A root SRV or UAV is therefore only a 64-bit number in two SGPRs. No D3D12 view is ever
    created at the title's address, so the 16-byte raw-view alignment rule never applies.
  - The only D3D12 views are the window root UAVs, which start at a base aligned to 4 GiB or
    64 KiB.
  - Sub-dword and unaligned accesses load the aligned dword and then shift or mask
    (`flat_small`, `d16_small`). Dword ops mask with `~3`.
- **How it maps onto VIctory:** For "typed root view" or an unaligned root SRV/UAV, do not make
  a view at the title VA. Pass the VA as root constants (or via a small upload table), and
  address the global buffer from item 1. Formats are decoded in the shader.

### 3. ExecuteIndirect: check the command-list vtable slot first (possible direct cause of zero counts)
- **Where:**
  - `hle/d3d12x/real.rs`: the comment block and `NEWER_LIST_SLOTS` around l.5020–5050.
  - Commit 956a476 and HANDOFF ~l.1769–1771.
  - `device_x.rs::create_command_signature_x` (~l.998).
  - `bind.rs::execute_indirect` (~l.232).
- **Facts:**
  - The newer console runtime (the retail game) shifts command-list methods by two from #59
    onward, so ExecuteIndirect sits at slot #61.
  - xcl first sent #61 to FillMemoryWith32BitValueX, and none of the title's indirect draws or
    dispatches ran.
  - Write64BitValueBottomOfPipeX is at #72 and Wait64 at #74. Write32 is at #71 with
    arguments (address, value, 0, 0).
  - In command signatures, the console's indirect argument type **105 (DispatchX)** is turned
    into a PC Dispatch. Other argument types pass unchanged. The root signature is kept only
    when the signature changes root arguments.
  - With a translated pipeline, xcl binds its ABI root signature and then calls the PC
    ExecuteIndirect with the title's argument and count buffers.
  - These cases are skipped: signatures that change root arguments (constants, CBV/SRV/UAV),
    and pipelines that are still compiling.
  - Argument and count offsets get a per-resource delta (`buffer_delta`) for buffers placed
    below their console address.
- **How it maps onto VIctory:** Confirm the game's ExecuteIndirect actually reaches our hook,
  with the right slot and arguments. Map type 105. Check whether the count buffer address is
  shifted.

### 4. Compute-written arguments: buffers share memory and barriers are global
- **Where:** `list_x.rs` around l.3631–3680 (ResourceBarrier) and around l.2700 (#146 barrier).
  Buffers in UMA are described at HANDOFF ~l.498–504.
- **How it works:**
  - Title buffers are placed resources on the UMA heap, so the argument and count buffers given
    to ExecuteIndirect are the same memory the culling compute wrote through `mem`.
  - Because the window UAVs are different resources over that memory, every console UAV barrier
    becomes one global UAV barrier per batch. A barrier on the title's resource alone does not
    order it on NVIDIA.
  - Console-only state bits are dropped.
- **How it maps onto VIctory:** If the culling compute writes through a global buffer, add a
  global UAV barrier, plus a transition to INDIRECT_ARGUMENT of the real argument and count
  resources, before ExecuteIndirect.

### 5. Silent zeros to rule out
- **Memory outside the windows reads 0.** HANDOFF ~l.1751 notes that the game's own
  `VirtualAlloc` memory, about 4.8 GB, is not GPU-visible, so shaders saw empty V#s.
- **CPU/GPU mirror reservations.** The same physical pages are reserved twice: a CPU view, and
  a no-access graphics view at +size. On the PC the second view was empty memory (commit
  9bd7635).
- **CBV descriptors must use stride 16 with `num_records = size/16`** (commit 01a7446).
  - With a raw V# (stride 0), `buffer_load idxen` reads row 0 for every index.
  - Culling constants such as frustum planes would then be wrong, and could cull everything.
- **Bottom-of-pipe writes into non-GPU memory are done by the CPU** after a fence
  (`cpu_writes.rs`). Otherwise the title waits forever.

## Not found
- Nothing about GPU culling, terrain, buildings or zero indirect counts in any fork's HANDOFF
  or docs.
- The openworld fork's `coop/` only lists "world is dark / untranslated shaders drop world
  draws" as open tasks.
- w1spo/xclayer has patch audits only.
- No special handling for phi addresses is needed in xcl, because of item 1.
