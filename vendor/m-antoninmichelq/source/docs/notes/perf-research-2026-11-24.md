# Raising VIctory's frame rate: existing work to reuse (2026-11-24)

Compared against our build `victory-stack-d13x`. Upstream main ffe310e. Everything listed is in Marvisak/VIctory or a fork of it, so all of it is **GPL-3 and can be reused, keeping the authors**.

**Limits of this search:** this session's GitHub API access does not cover Marvisak/VIctory, so I could not read PR descriptions, review comments, issues or the list of forks. I got all 612 PR branches with git and read their commit messages, then test-merged each candidate (`git merge-tree`) onto d13x and onto upstream main. The only measured gains are the ones written in commit messages. Of the forks, I checked only justr4mzi and VICauseway.

## Ranked candidates (none merged upstream, none in d13x)

1. **PR #600 (desp): ask once per span of memory for indexed SRVs.** https://github.com/Marvisak/VIctory/pull/600
   It sorts the merged ranges by address, groups those less than 1 MiB apart, and makes one write-watch query per group instead of one per range. This targets our 580 ms "indexed SRV refresh" and our ~235k queries per frame. The commit message says the old code made "hundreds of thousands of system calls per world frame". `VICTORY_NO_SPAN_QUESTIONS=1` turns it off for A/B tests. It contains #462, which we already have. Merging onto d13x: one conflict in `host.rs`. Most likely to help.

2. **PR #595 (wrothgaril, with @aycow): run submissions asynchronously.** https://github.com/Marvisak/VIctory/pull/595
   This is exactly point (1) of the brief. `Host::execute` stops waiting for the GPU: each submission signals a host fence, and a completion thread copies shader writes back to title memory in order. A later step waits only when it reads memory an earlier submission writes. A refresh waits only if the title wrote that range. A list reset while it runs records into fresh memory, so the CPU keeps recording while the GPU works. Present still waits for the frame to finish. It removes our 19 blocking fence waits per frame and overlaps the ~570 ms GPU time with CPU work. No numbers given. Size: 1,286 lines added. Conflicts: `draw.rs` on main, and 8 files on d13x (`host`, `list`, `queue`, `fence`, `texture`, `capture`, `draw`, `timeline` add/add). Biggest gain, but a large rebase and a real risk of race bugs.
   Safety PRs that go with it: **#609** (LAKSHYA: free textures, shadow buffers and descriptors only after the GPU is done with them) and **#575** (Len1k: free resources only behind the queue). Both merge cleanly on main and conflict on d13x.

3. **PR #693 (wrothgaril): shadow buffers in VRAM.** https://github.com/Marvisak/VIctory/pull/693
   Shadow buffers move from CPU-visible WRITE_BACK memory to DEFAULT (video memory). Dirty pages are uploaded through 4 MiB staging blocks, and copy-back reads only the bytes the GPU changed. This should cut the GPU wait, because shaders currently read our buffers over PCIe. No numbers given. Clean on main; conflicts in `host.rs` and `shadow.rs` on d13x. It changes the same code as #595, so take one first.

4. **PR #413 (wrothgaril): cheaper per-draw bindless work.** https://github.com/Marvisak/VIctory/pull/413
   Skips unchanged bindless copies and caches partial shadow pages. A draw with nothing to prepare takes no lock, and recording threads do not block each other on the heap lock. Targets our 0.4 to 1.3 s of recording.

5. **c326bfa (justr4mzi / sammitchell97): ask the kernel about each page once per submission.** Measured on an RX 9070 XT: 1,500-1,800 queries and 120-180 ms per frame. Still not in d13x.

6. **PR #435 (melon0918): merge identical write declarations within a submission.** Fewer copy-back ranges. Clean on main; `host.rs` conflict on d13x. VICauseway carries the same change as c8fb88f.

7. **PR #401 (cat4blep): less work in texture refresh and readback.** Touches `texture`, `shadow` and `watch`. Clean on main, conflicts on d13x.

8. **PR #472 (Davi Andrei): write snapshot grows in place, idle pool capped at 256 MB.** Fewer allocations, and it fixes our memory growth.

9. **PR #411 (FIocker): require only the descriptor heaps the pipeline uses.** Less root-signature and heap work per draw. Small (103 lines added).

10. **PR #509 (asc): opt-in execution performance census.** A measuring tool, useful for checking each step above.

## Already in d13x (verified)
#230, #431, #436, #440, #462, #464, #490, #497.

## Nothing new found for
- Mapping title memory directly (OpenExistingHeapFromAddress) or placed resources on title memory: no PR found beyond #501, which you already know.
- Several frames in flight beyond what #595 does.

## Suggested order
1. Rebase #600 onto d13x and measure the "preparing" time.
2. Add c326bfa and #435.
3. In a separate branch, port #595 with #609 and #575 (largest win, largest risk).
4. Then try #693.
