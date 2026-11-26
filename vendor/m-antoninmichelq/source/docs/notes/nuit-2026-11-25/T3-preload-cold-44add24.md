# Preload run 1 (cold), perf/preload 44add24, rapide env + PIPELINES_ALL_EARLY=1, 03:20-03:33 Paris, clean end
Log on PC: _work\runs\run-20261125-032048-jouer-prechargement.log
- World +124 s. ALL_EARLY: being made 9662 before world, 32846 at world, 32436 at +1 min gameplay, still 31181 at end.
- Library end: saved bytes=8184172 early=2781 made=51015 loaded=2047 started_early=84244 (0.7 MB -> 8.2 MB)
- Private max 23.6 GB, slow climb 21.9 GB (03:25) -> 24.2 GB (end). PC ~6 GB free, card 6557/6851
- memory, ours end: host copies 4456 / 4056 MB (windows outside buffers 760 / 3136), host textures 6847 / 1650 MB, lists 765, scratch 197
- fps after world avg 1.86 (last 10: 2.84), 2100-4900 draws, ~886 dropped/frame (compile backlog)
- textures waiting 18092 at world -> ~2370-2410 plateau from +2.5 min
- card gather end: 733 pieces (47.0 MB) merged, 240 fetched after all (239 past a plan's room of 160, 0 merged-from-newer-fetch)
- 0 refused pipelines (f5e50eb2 mesh refusal absent), no OOM
Promotion: Desktop = 64dc663 rapide (4F103DC70AE3608C) + WIDE_TEXTURES + CARD_GATHER at 03:33; _work "Jouer precedent" = 440e19e.
