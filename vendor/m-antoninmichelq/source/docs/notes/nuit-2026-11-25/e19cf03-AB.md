# Préchargement corrigé e19cf03 (perf/preload), 2 parties — nuit du 25/11/2026

Cache : victory-cache\preload. Comparaison : T1b = 64dc663 (Jouer actuel).

| Partie | Démarrage | Pic mémoire | Stable | fps moyen / 10 dernières | Pipelines |
|---|---|---|---|---|---|
| A (cache froid) | - | 22.8 GB | 22.1-22.8 GB, calme 80 s après le monde | 5.85 / 6.60 | made=57144, set_aside=27129 |
| B (cache chaud) | 11.2 GB | 24.81 GB | 23.8 GB | 5.87 / 6.79 | made=3342, loaded=60284, set_aside=20673 |
| T1b 64dc663 | - | 21.0 GB | 21.0 GB plat | 5.51 / 6.12 | - |

Verdict : NON promu. Plus rapide que T1b (fps et chargement), mais la partie à cache chaud
(celle que l'utilisateur jouera) dépasse 24 GB et se stabilise à 23.8 GB, contre 21.0 pour T1b.

Cause probable : la bibliothèque chaude charge ~60k pipelines (~+1.7 GB). Piste : ne sauver
dans la bibliothèque que les pipelines réellement utilisés par des draws.
