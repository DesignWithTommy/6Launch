# Preload run 2 ("warm"), perf/preload 44add24, 03:33-03:45 Paris, clean end. NOT promoted.
Log on PC: _work\runs\run-20261125-033301-jouer-prechargement.log
- Library loaded bytes=8184172; end saved bytes=8690062 early=2789 made=3469 loaded=53698 started_early=84243
- being made 31381 at world -> 30041 at end (84k ALL_EARLY list never finishes)
- World ~122 s. First minute: 0.4-3.0 fps, 650-3819 draws, dropped 264-3800/frame, textures waiting 18090 -> 20301 -> 8763
- fps after world avg 2.49 (last 10: 5.47): ~1.8 fps until +8 min, then 5.5-6
- dropped/frame ~250-480 after 2 min; textures plateau ~2580
- private: 15.7 GB at start (library + early pipelines) -> 23.6 -> 24.2 GB end, slow climb
- card gather end: 767 pieces (49.2 MB) merged, 230 fetched after (228 past room of 160, 2 merged-from-newer-fetch)
- refused: f5e50eb2 mesh only
Summary fps avg / last10 / private: T1b 5.51/6.12/21.2 flat; T3 1.86/2.84/23.6 climbing; T4 2.49/5.47/24.0 climbing
