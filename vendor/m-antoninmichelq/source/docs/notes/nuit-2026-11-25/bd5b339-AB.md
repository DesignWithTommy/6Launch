# Préchargement v3 bd5b339 (perf/preload), 2 parties, cache victory-cache\preload2

| Partie | Monde | « En cours » à 0 | fps moy / 10 dern. | Mémoire | Pipelines |
|---|---|---|---|---|---|
| A froid | +151 s | ~9.5 min après le monde | 1.91 / 5.36 | 17.2 au monde, pic 23.51 GB, pas stabilisé | made=54069 loaded=614 started_early=84236 set_aside=30883 |
| B chaud | +101 s | ~1 min après le monde | 5.24 / 6.09 | plat 19.9-20.0 GB, pic 20.11 GB | made=128 loaded=1964 started_early=1877 set_aside=0 |
| T1b 64dc663 | ~128 s | - | 5.51 / 6.12 | 21.0 GB plat | - |

Verdict : non promu. B est bon (1 GB de moins que T1b, fps égal en régime), mais A (cache neuf) a une boucle :
les frames où les draws sont abandonnés comptaient comme « chargement » et relançaient tout en avance.
Corrigé dans cdfbe3a (compte les draws demandés), A/B en cours dans victory-cache\preload3.
Seul refus : le connu f5e50eb2 + ce3f0432 (0x80070057).
