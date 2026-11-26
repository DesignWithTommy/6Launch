# Dark first cutscene: what is missing (RX 7600, 2026-11-24)

Build: origin/main fa5ef73 + PRs 55, 48, 52, 60, 61, 35, 51, 67, 70. Diag build, real GPU, frame 2500 (title card) captured with render-target dumps; GPU hung ~frame 2509.
Captures/logs stay on the user's PC (game content), only counts and reason strings here.

## First failing pass: geometry, not lighting
- G-buffer (2560x1440 RGBA8) and depth: ~6% of pixels covered (poles, cables, a roof edge); terrain and buildings missing.
- All 15 shadow maps empty. HDR lighting/composite targets (R11G11B10) all zero. Final output = title card only.
- So world draws are not emitted; lighting/tonemap have nothing to work on.

## Dropped work per world frame (2-3 s per frame)
- ~1,300 draws skipped: pipeline still compiling (~440 queued); should drain over frames.
- 1,500-1,650 indirect draws skipped; ~100/frame "indirect kind 0: prepare failed (compute false, mesh false)".
- Rest: untranslated shaders. Most are compute dispatches (likely GPU culling / indirect-argument builders), so the indirect/mesh args are never written.

## Untranslated-shader drop reasons, weighted by draws/dispatches per frame
1. "an address in an SGPR that is no root parameter": ~1,150/frame, 18 compute shaders (top three: 534, 283, 96+96). NOT the same message as "address not from a root parameter". Biggest lever.
2. "a structured buffer of unknown stride": ~250/frame (compute + mesh). Triage thread is on it.
3. "a descriptor neither loaded nor built": ~130/frame, mesh shaders (direct geometry).
4. "image_gather4_l": ~50/frame (compute).
5. "a hardware SGPR input": ~34/frame.
6. "a scalar memory read as data": ~32/frame (VS + CS). Triage thread is on it.
7. "barycentrics read directly": ~22/frame (PS).
+ "select or lane-mask join on an untracked mask": ~12/frame. 255 distinct shaders fail in total; 58 are "lane mask holding lanes elsewhere" but rarely hit here.

## GPU hang clue
DXGI_ERROR_DEVICE_HUNG on a single-list submission; DRED breadcrumbs show a compute dispatch whose table has two null UAVs (s0+16, s0+24). Possible lead for upstream #59/#71.
