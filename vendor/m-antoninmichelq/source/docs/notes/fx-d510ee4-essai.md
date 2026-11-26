# Essai : fusion fxceless d510ee4 sur Peau3 (branche fork port/fx-d510ee4, commit 23e5a09)

Base : jouer/peau3 b74ac64 + fxceless/victory-fork main d510ee4 (25 nov, 9,6-10,3 fps annoncés sur RTX 4070).
ZSlayerHQ/zslayer-pc-port-runtime : GPL-3, = fx 310c6eb + victory-amd (2,5-4 fps), rien de plus rapide -> rien pris.

Gardé de notre ligne : traducteur de shaders + clé de cache Peau3, file de textures 8 Go (HYSTERESIS, éviction, STAGING_POOL_MB),
self-copies enregistrées, HOIST_CLAMPS, CARD_GATHER. Coupé par défaut : VICTORY_CARD_ROOM_FOR_BUFFERS (réservait 3 Go).
Contrôles : cargo check workspace Windows (release + diagnostics + --tests) OK. Pas de test sur carte possible dans le cloud.

## Build
cargo build --release -p victory -> _work\victory-play-essai.exe (cache neuf : victory-cache\fxmerge, 1er lancement froid ~10 min)

## Runs proposés (env Peau3 complet + ...)
- F0 (régression) : rien de plus. Doit égaler Peau3 (6,4 fps, <= 21 GB, image OK).
- F1 (switchs fx) : VICTORY_ASYNC_SUBMIT=1 VICTORY_QUEUE2=1 VICTORY_COPY_PLAIN=1 VICTORY_JOIN_PARTS=1 VICTORY_BACK_OWN_VIEW=1 VICTORY_CARD_REHASH_RUNS=1
- F2 : F1 + VICTORY_INDIRECT_WINDOW=1 VICTORY_INDIRECT_GATHER=1 VICTORY_HOIST_CLAMPS=0 (chemin fx des clamps au lieu du nôtre)
Ne pas mettre VICTORY_CARD_GATHER=1 ni VICTORY_CARD_FETCH_INFLIGHT=1 avec F1/F2 (fx : ne pas combiner le fetch en vol).
Si F1 plante : retirer d'abord ASYNC_SUBMIT, puis QUEUE2.
Lignes à surveiller : "parts joined", "back own view", "gathered", "device removed".
