# Nouveautés des repos VIctory — 25 nov. 2026, ~00:50 (Paris)

Comparé à notre base fork-stack-a / prison-stack-a (= fxceless/victory-fork main 4e17478 + nos commits).
Recherche en lecture seule (rien posté sur GitHub). Classé par importance pour nous (RX 7600, 8 Go).

Note : fxceless main est maintenant à 2119b0e (seulement 2 commits de plus que notre base : la modif "lane-mask #11" retirée puis remise, rien à faire).

## 1. Réglages à essayer tout de suite (zéro code)

| Quoi | Lien | En une phrase | À faire ? |
|---|---|---|---|
| Testeur RX 7600 XT sur notre base exacte | https://github.com/fxceless/victory-fork/issues/321 (= upstream #1842) | `VICTORY_WAVE_WIDE=1` évite le plantage du pilote AMD et affiche plus de géométrie ; `VICTORY_RELAY=2` fait crasher en mémoire (OOM) vers la frame 1723. | OUI : vérifier si WAVE_WIDE=1 change quelque chose chez nous ; ne pas mettre RELAY=2. |
| Blocs noirs « mosaïque » | https://github.com/fxceless/victory-fork/issues/310 | Les passes de nettoyage des shaders (1684989, dans notre base) font des carrés noirs dans le prologue ; `VICTORY_SHADER_OPT=0` les enlève, même fps. | OUI : à tester (effacer le cache shaders avant). |
| Réglages mesurés sur RX 6800 (AMD) | https://github.com/fxceless/victory-fork/issues/264 | `VICTORY_TEXTURE_NO_WAIT=1` : 587 → 470 ms/frame ; `CARD_BUFFERS_MIN_MB=1`, `CARD_BUFFERS_MB=1024` (512 max pour 8 Go selon #1820), `SKIP_PIPELINES=39f0e9aeb156a61c`. | OUI pour TEXTURE_NO_WAIT. |
| Boucles infinies | https://github.com/Marvisak/VIctory/pull/1819 | `VICTORY_SHADER_LOOP_LIMIT=16384` évite qu'un shader fou gèle tout le PC. | Utile en sécurité. |

## 2. Performance (on est à 2-4 fps, GPU sous-utilisé)

| Quoi | Lien | En une phrase | À porter ? |
|---|---|---|---|
| Hachage des copies sur carte, 1 groupe par morceau | https://github.com/fxceless/victory-fork/pull/253 | Mesuré sur AMD RX 6800 : attente GPU 90 → 32 ms par frame ; déjà rebasé sur 2119b0e. | OUI, priorité haute. |
| Présent qui n'attend plus sa propre frame | https://github.com/Marvisak/VIctory/pull/1828 | Le CPU prépare la frame suivante pendant que le GPU finit la précédente : c'est exactement notre problème « préparer puis attendre ». | OUI (à adapter, conflit probable avec RELAY/LATE_WAIT). |
| Rendu à 1080p au lieu de 4K natif | https://github.com/Marvisak/VIctory/pull/1793 | La Series X rend en 4K, ce qui déborde la mémoire ; `VICTORY_FFX_SCALE=0.5` rend en 1080p via FidelityFX. | OUI mais gros : dépend du chemin FidelityFX d'upstream (0b35b7f) que fxceless n'a pas. |
| Frames de 3 s avec le constructeur RT = limite 1 023 copies | https://github.com/fxceless/victory-fork/issues/284 (+ PR #205) | Même limite COPIES=1023 que chez nous ; l'auteur prépare un « masque de pages sales ». | Suivre ; prendre la PR dès qu'elle sort. |
| Rafraîchissements en parallèle / workers selon nb de cœurs | https://github.com/fxceless/victory-fork/pull/38 , /pull/49 | Le CPU fait les copies sur plusieurs cœurs au lieu d'un seul. | OUI (notre CPU est sous-utilisé). |
| File COPY pour le relay | https://github.com/fxceless/victory-fork/pull/71 | Les copies passent par la file de copie du GPU et ne bloquent plus. | Peut-être. |
| Banc de test des copies | https://github.com/Marvisak/VIctory/pull/1743 | Mesure des PR de copie sans le jeu. | Outil, pas prioritaire. |

## 3. Textures en attente / VRAM pleine (12-24 k textures en file)

| Quoi | Lien | En une phrase | À porter ? |
|---|---|---|---|
| Rafraîchir les textures sans attendre les remappages | https://github.com/fxceless/victory-fork/pull/270 (et #95) | Les textures ne bloquent plus sur la mémoire du jeu qui bouge. | OUI. |
| Détuilage parallèle des grosses textures | https://github.com/fxceless/victory-fork/pull/271 (et #92) | Plusieurs cœurs pour préparer les grosses textures. | OUI. |
| Budgets calés sur la VRAM réelle | https://github.com/fxceless/victory-fork/pull/45 , /pull/42 , /pull/48 | Les limites de textures/tampons s'adaptent aux 8 Go au lieu de valeurs fixes. | OUI (8 Go). |
| -2,1 Go de mémoire | https://github.com/fxceless/victory-fork/pull/265 | `VICTORY_LEAN_WINDOWS=1` : fenêtres sur la carte sans copie CPU. | À tester. |
| File de textures réglable / libérer les plus vieilles / réessayer après manque de mémoire | https://github.com/fxceless/victory-fork/pull/106 , /pull/140 , /pull/241 | Débloque la file quand la VRAM est pleine. | OUI (#140 surtout). |
| Libérer les tampons inactifs | https://github.com/fxceless/victory-fork/pull/194 | Éviction des tampons non utilisés. | Peut-être. |

## 4. Personnages (peau/vêtements noirs)

| Quoi | Lien | En une phrase | À porter ? |
|---|---|---|---|
| Fork FQY55kai (même base que nous) | https://github.com/FQY55kai/victory-gta6 | Commits frais : v_permlanex16 (212a81e), demi-interpolations (c156d64), « descripteur lu comme nombre » ne jette plus le shader (da327bf), 7035d5d. Ce sont les échecs vus dans le log du RX 7600 XT. | OUI, cherry-pick direct. |
| +46 programmes : demi-interpolations f16, sommes 16-bit | https://github.com/fxceless/victory-fork/pull/203 | Corrige des shaders de matériaux/visages. | OUI. |
| Correctif « demi-wave » de PastiiJay | https://github.com/fxceless/victory-fork/issues/264 | La peau n'a plus l'air mouillée, ciel violet parti (yeux encore noirs). | OUI, retrouver le commit. |
| Skinning des persos lit des zéros | https://github.com/Marvisak/VIctory/issues/1567 | Diagnostic : le tampon de sommets choisi dynamiquement est vide côté hôte. | Piste pour le fil Personnages. |

## 5. Chargement

| Quoi | Lien | En une phrase | À porter ? |
|---|---|---|---|
| Bibliothèque de pipelines sous un seul verrou | https://github.com/Marvisak/VIctory/issues/1615 , PR https://github.com/Marvisak/VIctory/pull/1680 | Charger 1 225 pipelines prenait 3,5 min, un par un ; en parallèle ça va beaucoup plus vite. | OUI. |
| Pipelines mesh jamais rechargés du cache | https://github.com/Marvisak/VIctory/issues/1683 (corrigé upstream 9c58a1c) | 723 pipelines recompilés à chaque lancement (~55 s) car la topologie manque. | OUI, petit correctif à adapter. |
| Précompilation hors-jeu des pipelines | https://github.com/fxceless/victory-fork/pull/282 | Tout compiler avant de lancer le jeu. | Utile (nous avons déjà translate). |

## 6. Lumière indirecte / RT / voxels

| Quoi | Lien | En une phrase | À porter ? |
|---|---|---|---|
| 195 shaders voxel = un shader géométrie fusionné | https://github.com/fxceless/victory-fork/pull/309 + issues #311-#314 | Étape 1 d'un plan en 5 étapes pour traduire nos 195 GS_VoxelizeGeneric. Upstream ne l'a pas non plus. | Suivre, prendre chaque étape. |
| BVH : enfant absent = raté | https://github.com/fxceless/victory-fork/pull/213 , /pull/230 | Corrige des tests de rayons. | Peut-être. |
| Constructeur RT sur GPU 32 lanes | https://github.com/fxceless/victory-fork/pull/156 | Pour NVIDIA ; nous (AMD) avons du wave64 natif. | Non prioritaire. |

## 7. Vidéo d'intro saccadée, streaming, AMD

| Quoi | Lien | En une phrase | À porter ? |
|---|---|---|---|
| Vidéo d'intro / écrans qui clignotent | https://github.com/fxceless/victory-fork/pull/343 (issue #269) | La cause est le « texture guard » qui ne recalcule certaines textures qu'une frame sur 8 ; corrige les saccades de la vidéo. | OUI, petit et facile. |
| DirectStorage « physical pages could not be written » | https://github.com/fxceless/victory-fork/pull/329 (issue #268) | Même avertissement que chez nous ; peut donner une cinématique noire ; la PR dit quelle page. | OUI (diagnostic). |
| Plantages du compilateur AMD | https://github.com/fxceless/victory-fork/pull/317 , /pull/195 , /pull/135 | Évitent des crashs/erreurs propres aux pilotes AMD. | OUI si on voit ces crashs. |

## 8. Le reste

- **Upstream Marvisak** : grosse mise à jour 70564ae (28 commits à 22:36 hier). README : monde affiché (rues, bâtiments, véhicules, persos), surfaces sans éclairage, 1-2 s par frame, tous les shaders mesh traduits, voxels toujours manquants. Pas compatible directement avec notre base (fxceless part d'une version plus ancienne) : ne prendre que des morceaux (pipelines mesh, 1 texture hôte par mémoire).
- **Autres forks** : JMAN730/victory, DesertGamer/VIctory-V2, VIctory Forge = regroupements de PR, aucun ne montre mieux que nous. dudegio/VIctory-world-preview : couleurs plates seulement.
- **xcl** (GPL-2, idées seulement) : 425b869 évite un crash amdxc64 (MSAA), corrections de masques. **AnyXboxX** : prototype sur Unicorn, rien d'utile. **AgentRecompVI** : pas d'activité trouvée.
