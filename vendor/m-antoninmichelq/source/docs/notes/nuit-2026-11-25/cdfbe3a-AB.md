# Préchargement v4 cdfbe3a (perf/preload), 2 parties, cache victory-cache\preload3

| Partie | Monde | « En cours » à 0 | fps moy / 10 dern. | Mémoire | Pipelines |
|---|---|---|---|---|---|
| A froid (ALL_EARLY=1) | +175 s (10 Entrée) | jamais (~20 000 tout le long) | 1.53 / 1.68 | 21.7 -> 23.27 GB, pas stabilisé | 1 seule ligne avant le monde : made=50712 started_early=60745 set_aside=10032 |
| B chaud | +122 s | ~1.2 min après le monde | 5.09 / 6.36 | plat 19.76-19.91 GB, pic 20.38 | made=495 loaded=1479 started_early=1788 set_aside=0 |
| T1b 64dc663 | ~128 s | - | 5.51 / 6.12 | 21.0 GB | - |

Lecture (fil fps) : en froid avec ALL_EARLY, ~55k pipelines créés, mémoire à 21-23 GB, donc le
garde-mémoire ne laisse passer qu'~1 compilation à la fois : la file ne se vide jamais.
Décision : ALL_EARLY retiré (désactivé par défaut). Candidat = cdfbe3a sans ALL_EARLY sur cache gta6b
(early-pipelines.txt 2906 lignes, pas de host-pipelines.bin). Partie de confirmation après T5c.
