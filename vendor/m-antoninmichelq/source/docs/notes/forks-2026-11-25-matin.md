# Dernières grosses mises à jour des forks VIctory et de xcl (25 nov., matin)

Comparé à notre version Jouer (variante/prechargement = cdfbe3a). Ne refait pas `nouveautes-repos-2026-11-25.md` ni `upstream-28-commits-2026-11-25.md`.

## Qui est le plus avancé
| Projet | Ce qu'il montre | Code public ? |
|---|---|---|
| **hyphennn = fxceless/victory-fork** (même personne, confirmé par PortWatch) | Cour de la prison éclairée, ombres, cheveux, textures, foule, ~3 fps (streams Rumble) | Oui, c'est notre base |
| suri (sur victory-fork) | Prologue « presque comme la console », ~3 fps | Non |
| ZETTROX `Zettrox/ztrx` | Joue jusqu'à la mission « Gone Fishing », 1 h 30 sans plantage, rendu pauvre, RTX 3050 | Oui, mais = victory-fork 2119b0e + un README (rien de neuf dans le code) |
| `uiliannn/VItoriaBr` | Prologue terminé, 4 fps max | Oui : 56 commits sur victory-fork c96b4f2 (shaders, audio XMA, courses mémoire) |
| `JoshuuDev/victory-amd` → `FQY55kai/victory-gta6` | Fork « stabilité AMD » : +48 shaders traduits, 0 pipeline refusé dans le prologue | Oui |
| `tadymila/xboxmericavi` | Fork « performance » (préréglage TURBO) basé sur upstream | Oui, surtout des réglages |
| xcl (ivall/xcl-dev) | ~5 fps dans le monde sur RTX 5090, image incomplète, ne tient pas longtemps | GPL-2 : idées seulement. Moins avancé que nous dans le jeu |
| upstream Marvisak | 940d32f : pages écrites marquées par les shaders + ~40 PR fusionnées (tests et petits correctifs shaders) | Conception mémoire différente de la nôtre : rien de repris pour l'instant |

## Ce qui a été repris (branche `port/forks-2026-11-25`, a8622f4)
1. **fxceless main a26859f** (7 commits, ~9 500 lignes) : « la première pièce s'affiche bien » (descripteurs copiés octet par octet, vues décalées, place sur la carte pour la première image du monde), écriture vue depuis toutes les vues d'une page, textures emballées gardées dans la file, test de profondeur (depth bounds), peau et cheveux sans couleur corrigés (textures revues après envoi), niveaux de mip à la demande (désactivé), clamp de tas AMD de triippywhale.
   - Gardé chez nous quand les deux faisaient la même chose : cache de shaders + `translate --cache` (leur `--warm` retiré), tas de vues 2^21 + tas d'échantillonneurs 2048 (prouvés sur la RX 7600), détuilage parallèle, notre `descriptor_data`, notre v_permlane16/x16.
   - Combiné : garde mémoire du PC + place carte de fxceless ; budget de textures selon la VRAM + leur rattrapage (STAGE_MB/STAGE_MS/CATCHUP) ; éviction des vieilles textures dans leur passe « carte pleine ».
2. **victory-amd** : pas de blending sur cibles entières (un pipeline refusé de moins), log des pipelines refusés, copies de tampons prioritaires sur la carte (`card_room_percent`), descripteurs lus seulement pour leur taille (puddles, eau peu profonde), textures dont seuls les petits mips sont là.
3. **VItoriaBr** : course des copies retour du relay (cause des crashs 0xc0000005 au chargement), SHADER_HEAPS tenu pendant le miroir, scan des descripteurs dynamiques (fausse saturation), DLL hôte chargée hors verrou, 2 correctifs audio XMA (boucle infinie), logs BVH.

Compile pour Windows (cargo check --target x86_64-pc-windows-msvc) ; tests victory-shader (173), victory-formats, victory-game OK sous Linux. **Pas encore testé en jeu.**

## Pas repris (et pourquoi)
- victory-amd 5cfc8d8 (doubles complets, eau) : 11 conflits, en partie déjà là via fxceless. À faire si le validate montre les shaders d'eau en échec.
- victory-amd 11cdbec : met la vsync par défaut et recoupe ce qu'on a déjà.
- victory-amd 4436f8f (40 conflits), 4d64efe (pages lues à l'exécution : même idée que notre essai dstorage qui a régressé), 6c79025 (basse résolution : eux-mêmes l'ont désactivé, ça retirait le périphérique).
- VItoriaBr : ~20 commits shaders/GPU en conflit avec nos propres versions (mesh, LDS, barrières, échantillonneurs immédiats). La couverture annoncée (25 670 validés) est sous la nôtre (25 694).
- 39185a2 (nom d'opcode décodé une fois, -4 à -8 % de temps de traduction) : casse notre ir.rs.

## À tester sur le PC (fil Sync)
Build release de `port/forks-2026-11-25` a8622f4, cache neuf (le traducteur a changé), même protocole 12 min que cette nuit contre Jouer cdfbe3a (20,2 Go, 6,4 fps). Promouvoir seulement si : pas de crash, mémoire ≤ 21,5 Go, fps ≥ 6,4, et image au moins aussi bonne (peau/cheveux colorés attendus).
