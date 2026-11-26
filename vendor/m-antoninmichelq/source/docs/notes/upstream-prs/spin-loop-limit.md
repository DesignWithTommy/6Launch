<!-- claude-projects-attribution: {"github_login":"AntoninMichelq"} -->
_Requested by **4nto** · [project thread](https://claude.ai/code/project/chan_01XYoPEzLL1bj63HP9pniGvs?thread=cmsg_01XYoPEzLL1bj63HP9pniGvsYRPS2v37oeDeGaCHT7dFsi)_

## Summary

Translated loops now stop after 2^14 (16,384) iterations instead of 2^20, and spin-wait loops after 4096. On an RX 7600, a loop limit of 4096 cut GPU time per world frame by a factor of 3 to 10. On an RTX 5070, the 2^14 default kept every submission under the host's ~2 s GPU watchdog, where 2^20 reached 10.6 s and removed the device.

Some compute loops only reread one address until another workgroup, queue or the command processor changes it (decoupled look-back, persistent-thread queues, locks). On the console the write comes within microseconds. On the host it may never come: the writer's view can be another host resource, a null view, or not yet mirrored, and the host compiler may hoist the load out of a loop that writes nothing. The loop then runs to the general limit in every lane: with the old 2^20, about a second per wave of uncached reads.

1. **`VICTORY_SHADER_LOOP_LIMIT` in release builds too.** It was gated on the `diagnostics` feature. The shader cache already keys on it.
2. **Spin-wait loops get their own, lower limit.** A new pass (`emit/spin.rs`) finds such loops in the IR. They stop at `VICTORY_SHADER_SPIN_LIMIT` (default 4096, never above the loop limit).
3. **The general loop limit drops from 2^20 to 2^14 (16,384).** Nixitov measured 110 s world runs on an RTX 5070:

   | Loop limit | Slow submissions | Longest | GPU time per frame | Device removed |
   |---|---|---|---|---|
   | 2^20 (old default) | 9 | 10,637 ms | 24-457 ms | yes, twice with the desktop |
   | 2^16 | 11 | 1,089 ms | 54-162 ms | no |
   | 2^14 | 3 | 371 ms | 42-50 ms | no |

   The frame rate stayed the same (2.6-4.4 fps). The 10 s submissions were not spin waits, so only the general limit fixes them. Trade-off: a legitimate loop of more than 16,384 iterations per lane is cut short and gives a wrong result there; `VICTORY_SHADER_LOOP_LIMIT` raises the limit in any build to check.

## What changed

- `emit/spin.rs` (new): a loop is a spin wait when it has no stores, exports, LDS writes, barriers or messages; every memory read and atomic is at an address the loop does not change; and nothing loop-carried is made by arithmetic, only lane masks, compares and what was read.
- `emit/lower.rs`: `LOOP_LIMIT` is 2^14, with why in its doc comment; `count_iteration` uses the spin limit for spin-wait loop headers.
- `emit/mod.rs`: `spin_limit()` and `DEFAULT_SPIN_LIMIT`; the loop limit is read in every build; the tests check the 16,384 default and that the spin limit stays at or below it.
- `victory-gpu/src/shader_cache.rs`: the cache key includes the spin limit.
- `README.md`: the new variable and the new default.
- Upstream main is merged in (no rebase); the conflicts in `emit/lower.rs` were the test-only fields of `Lowered` and the test helpers.

Open PR #292 also reads `VICTORY_SHADER_LOOP_LIMIT` in release builds (the same as commit 1), and changes the loop counters to one shared budget per invocation. It has no spin-wait limit. The two touch the same counter code, so whichever lands second needs a small rebase.

## How it was tested

- `cargo test -p victory-shader` (Linux) passes after merging main: 102 tests (1 ignored), including new ones for a spin wait on a table buffer (spin limit), a loop that writes LDS (loop limit), the spin-loop finder and the limit parsers.
- `cargo check --tests -p victory-shader -p victory-gpu` and `cargo check -p victory` for `x86_64-pc-windows-msvc` (with `CARGO_FEATURE_PURE=1`) pass.
- In the game on an RX 7600 (Windows 11), on an integration stack that includes these commits, with A/B diagnostic runs in the world:
  - With the loop limit at 4096 (`VICTORY_SHADER_LOOP_LIMIT=4096`), GPU time per frame went from 410 to 1,384 ms to a median of 127 ms.
  - That run lowered the limit for all loops. This PR gives 4096 to spin waits and 2^14 to other loops.
- On an RTX 5070, Nixitov's runs above: 2^14 removed the device removals with no frame rate cost.
  - No hang in 5 minutes.

## Where 3D rendering stands on an RX 7600 (Windows 11)

- In the world's first cutscene, props, characters, cables and vehicle tyres render (seen in a G-buffer view).
- Ground, roads, buildings, textures, sky and lighting are still missing: the lighting output is black.
- No GPU hang in the latest world runs of several minutes. About 10,000 draws per frame, at well under 1 fps.

These observations come from the integration stack (upstream main plus many open PRs), not from this branch alone.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

https://claude.ai/code/session_01AZ9pSB4ZoXYUifuXg8gm66
