# Upstream Marvisak/VIctory PRs relevant to the culling / indirect-argument compute shaders

Date: 2026-11-24. Method: GitHub API and github.com both returned 403 from this sandbox, so open/closed/merged state and PR bodies could not be read. Everything comes from git: I fetched all 726 `refs/pull/*/head` with blobless git and compared commit subjects, file markers and `git merge-tree` results against our fork's `victory-stack-d13z2`.

Basics:
- The branch tip is **f45ce64**, not c743ce8.
- d13z2 contains all of upstream main (ffe310e).
- Upstream has no branch except `main`.
- Every PR below has commits that are not in main, so each is either open or closed without merging.
- Every candidate conflicts with d13z2 (`git merge-tree`), so each one has to be ported by hand.

## Important: the "typed root view" and "write through address in root constants" refusals are switched off by default
In d13z2, `emit/mod.rs::root_uavs()` only allows root UAVs when `VICTORY_ROOT_UAVS=1` is set. It is off because an RX 7600 page-faulted with it on. When it is off, these programs stop with "a typed root view" or "…without root UAVs". Upstream #650 and #709 deal with that page fault.

## Ranked table

| # | PR | Title (head commit) | What it does for these shaders | Files | In d13z2? | Conflicts | Recommendation |
|---|---|---|---|---|---|---|---|
| 1 | #645 | Trace multi-term, scaled, masked, and **phi-joined** addresses in resource tracking | Rewrites the address tracer to handle phi joins, loop-carried pointer induction (test `root_srv_loop_carried_pointer_induction_phi`), sums of two computed terms, VA masks (`s_and` with low byte 0xFF), add3/lshl_add, and high halves carried through phis. Goes straight at "an address from a phi" (6f6a359d). | shader/resource.rs (+1.4k), emit/lower.rs, valu_tests.rs | no. Our 513c93b and fix/described-smem only cover descriptor-base and unwritten-VGPR phis. | high: resource.rs and lower.rs; 131 commits behind main | **Port first.** Port the tracer by hand and keep its tests. It complements fix/described-smem: that branch handles SMEM descriptor bases, #645 handles root-SRV/global addresses. |
| 2 | #650 | Bound root view accesses by the size the host binds | The host passes each root view's size, and every raw, typed or CB access is range-checked (reads past the end give 0, writes past it are dropped). This removes the page fault that made us gate root UAVs. | emit/lower.rs, gpu/draw.rs, tests | no | medium | **Port, then test with VICTORY_ROOT_UAVS=1.** |
| 3 | #709 | Structured access through a root SRV/UAV uses its raw view; cap root SRV/UAV refresh ranges | An indexed load, store or atomic through a root view goes through its raw view at `idx*stride+off`, which fixes "a typed root view". Also caps how much of the buffer is refreshed or copied back. | lower.rs, draw.rs, pipeline/host.rs, shadow.rs | partly (only the #294 commit) | medium | **Port the 3 commits that are missing.** |
| 4 | #1056 | Scalar loads from a root view's address read its memory | `s_load` at a root descriptor's address, used as data, now reads the buffer instead of stopping with "a descriptor read only as data". | lower.rs, root_view_tests.rs | no | low | Port; it is small. |
| 5 | #1012 | Descriptors joined by nested control flow | Computes the heap index of a phi of phis recursively. | lower.rs | no | low | Port. |
| 6 | #1077 (contains #489) | Root UAVs + spill-table hardening | The spill-table commits require the mask to keep all 40 address bits, only spill when s16 is free, and read the arguments as they were on entry. The root-UAV part overlaps our #294 and 306dc64. | native.rs, emit/*, gpu/draw.rs, descriptor/host.rs | partly (we have our own unspill and root UAVs) | high | Compare `unspill.rs` with the 3 hardening commits and port only the fixes. |
| 7 | #710 / #667 | Addresses passed in two root constants | Reads through a root-constant address pair as a root SRV. #710 also covers `s_load` through it. | resource.rs, lower.rs, gpu | mostly superseded by our d7159aa and 4cd8549 | high | Port only the s_load case from #710 if we refuse it. |
| 8 | #242 (2 of 3 commits missing) | Move UAV-written indirect argument buffers to a read state for the clamp; bump-allocate scratch | Argument and count buffers written by compute are moved UNORDERED_ACCESS → NON_PIXEL_SR\|INDIRECT_ARGUMENT before the clamp and ExecuteIndirect. Without this, args written by compute can be read incorrectly. | gpu/indirect.rs, host.rs, draw.rs | partly | medium | **Port**; it is host-side and needed once the compute shaders translate. |
| 9 | #956 | Multi-argument ExecuteIndirect signatures with root constants | Translates command signatures that contain CONSTANT arguments, and restores the overwritten root constants afterwards. | indirect.rs, draw.rs, list.rs | no | medium | Port if the logs show indirect signatures being dropped. |
| 10 | #948 | Implement ClearUnorderedAccessViewUint/Float | Neither d13z2 nor main implements UAV clears. Culling and compaction usually reset their counters with ClearUAV. | list.rs, descriptor/host.rs | no | medium | **Port**: append counters that are never reset give wrong counts. |
| 11 | #583, #40 | Respect the indirect count buffer, guard overflow, null root views; synchronize indirect buffers | Count-buffer and barrier handling for indirect args. | indirect.rs, host.rs, shadow.rs | no | high | Review; overlaps our #242 and #334 merges. |
| 12 | #479, #367, #334 (2nd commit), #588 | Track the state of host copies of buffers | Compute-written copies get the right barriers before vertex, index or argument use. | host.rs, indirect.rs | partly (#334 first commit) | high | Port the 2nd commit of #334, which keeps a copy writable within the command that writes it. |
| 13 | #986 | Copy unaligned buffer SRVs into element-aligned scratch | Copies instead of refusing. The same approach could replace our refusals of root SRVs/UAVs not on 16 bytes (f45ce64, 44aba1f). | descriptor/host.rs, draw.rs, host.rs | no | low | Adapt the approach for unaligned root views. |
| 14 | #493 / #994 / #704 | Root SRVs as bounded one-view tables; raw views for unknown stride | Our bd48701 already covers this. | gpu | mostly | high | Skip unless a gap shows up. |
| 15 | #1066 | Indirect draw stream compaction (bundled with wave/dot4 changes) | Performance only. | indirect.rs, lower.rs | no | high | Low priority. |
| 16 | #568, #385, #521, #483 | Waterfall table views; lane-mask phi edges; thread-group info SGPR; heap index from data | Compute refusals next to ours. | lower.rs, structure.rs | no | medium | Port if the logs show these refusals. |

## Pointer chasing (9205d265: "an address loaded from memory")
No upstream PR loads an address from a buffer and then dereferences it (BDA-style). The closest are #489 (the spill table: a pointer in s[0:1], which we already handle in unspill) and #402 (finding the buffer that holds an arbitrary address). This family still needs our own work.
