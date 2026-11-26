# T5b sondes de lumière (e207975, VICTORY_NO_COMPILE=none, cache froid victory-cache\sondes), 12 min

- 39f0e9ae (ScreenProbes) : premier bind 04:24:50, traduit 04:25:32, pipeline créé 04:26:50 (~2 min). 0 refus, pas de 0x8007000E.
- "host pipelines all made made=1810 loaded=133 started_early=0 waited_for_memory=46 tried_again=0".
- Mémoire privée : 15.6 GB au monde, pic 21.08 GB (04:26:34), puis plat 19.31-19.34 GB jusqu'à la fin. Pas d'emballement (ancien 22→34 GB disparu). ~1.7 GB sous T1b.
- fps après le monde : moyenne 4.77, 10 dernières 5.66 (1-3 fps pendant la compilation, puis 5-6 stable). Cache totalement froid.
- Fin : dropped 340, 0 en cours, ~2570 textures en attente. Un "library saved bytes=0" isolé (04:28:18), la sauvegarde suivante normale.
- Capture locale à 11 min (cellule) : éclairage chaud, pas de brouillard violet.

Verdict : non promu (fps froid < T1b 5.51/6.12). Mémoire et sondes OK. À faire : relance à cache chaud ; si fps ≥ ~6.1, passer NO_COMPILE=none dans Jouer.

## T5c (même cache, chaud), 12 min
- 39f0e9ae chargé depuis la bibliothèque au démarrage (pas de recompilation, pas de pic).
- all made : made=67 loaded=2041 started_early=2023, ~30 s après le monde (+102 s).
- fps 5.11 / 5.67 (175-200 ms/frame, ~20 ms de plus que sans sondes). Mémoire plat 19.33-19.41 GB, pic 20.1.
- 0 refus, pas de violet.
Verdict : non promu (5.67 < 6.1). Les sondes coûtent ~10 % de fps. Option : lanceur séparé « avec sondes » dans _work si l'utilisateur préfère l'éclairage.
