<!-- claude-projects-attribution: {"github_login":"AntoninMichelq"} -->
_Requested by **4nto** · [project thread](https://claude.ai/code/project/chan_01XYoPEzLL1bj63HP9pniGvs?thread=cmsg_01XYoPEzLL1bj63HP9pniGvsYRPS2v37oeDeGaCHT7dFsi)_

## Summary

Two changes to host texture lifetimes. Together they stop a GPU hang in the world and halve host texture memory on an 8 GB card.

1. **Free textures and slot resources only after the running submissions.** A released title texture used to drop its host copy right away. A rewritten descriptor slot did the same with the resource it held. Both happened while a submission that still read them was running, sometimes for most of a second in the world. On an RX 7600, device-removed data (DRED) showed a page fault inside a freed 1024x1024 BC7 texture about 65 s into the world, ending in `DXGI_ERROR_DEVICE_HUNG`. These objects now go to a retired list, tagged with the submission count. They are dropped once that submission has been waited for, or right away if none is running.
2. **Make a texture's host copy on first use, not at creation.** In the world the title creates thousands of textures, and many are never used by the GPU (no view, target, copy or barrier). At the world's peak there were 3,630 host textures, 8.9 GB, against a 7 GB video memory budget. The driver moves the excess to system memory, where it counts against the commit limit. The creation state and clear value are now kept, and the host copy is made on first use (`OnceLock`).

## What changed

- `host.rs`: `retire()` and the retired list, emptied after each waited submission.
- `descriptor/host.rs`: `HostHeap::keep` retires the old resource instead of dropping it.
- `resource.rs`:
  - `Resource.host` becomes a `OnceLock` made by `host()` on first use, from the kept `made_with` (state, clear value).
  - `Drop` retires the host copy.

This overlaps a little with #135, which keeps replaced resources for some frames. This PR covers texture releases and slot rewrites tied to the submission that reads them, and does not depend on #135.

## How it was tested

- `cargo check --tests -p victory-gpu` and `cargo check -p victory` for `x86_64-pc-windows-msvc` (with `CARGO_FEATURE_PURE=1`) pass.
- In the game on an RX 7600 (Windows 11, 32 GB RAM), on an integration stack that includes these two commits:
  - **Hang:** before change 1, every world run hung at 50 to 65 s. After it, the next runs went 1 min 40 s to about 4 min with no hang. A later hang came from another freed buffer, which another branch handles.
  - **Memory, one minute into the world:** host textures went from 8.8 to 4.6 GB, peak commit from 24.3 to 17.4 GB, and video memory from 9.6 of 7.0 GB budget to 5.2 of 7.4. No missing objects were seen.

## Where 3D rendering stands on an RX 7600 (Windows 11)

- In the world's first cutscene, props, characters, cables and vehicle tyres render (seen in a G-buffer view).
- Ground, roads, buildings, textures, sky and lighting are still missing: the lighting output is black.
- No GPU hang in the latest world runs of several minutes. About 10,000 draws per frame, at well under 1 fps.

These observations come from the integration stack (upstream main plus many open PRs), not from this branch alone.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

https://claude.ai/code/session_01AZ9pSB4ZoXYUifuXg8gm66
