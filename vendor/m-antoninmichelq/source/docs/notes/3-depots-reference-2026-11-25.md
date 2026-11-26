# Les 3 dépôts de référence comparés à notre version (25 nov. 2026, ~15 h Paris)

Dépôts suivis désormais (choix de l'utilisateur) :
- **fxceless/victory-fork** (main 0b0074e) : notre base d'origine (4e17478). RTX 4070.
- **Belobug/VIctory-Belobug-AMD** (main 01aadd1) : part de l'upstream Marvisak 8e79c80, pas de fxceless. RX 9070 XT 16 Go.
- **pragyaangaur/VIctory-macOS** (main dc095db) : upstream 940d32f + portage Mac (Wine + D3DMetal).

Notre version : Jouer = `nuit/tout-3q2` 9975116 (+ TEXTURE_HALF=2), branches en cours `perf/30fps-pipeline` 893a56b, `perf/heap-pass` 924b5c6, `cand/peau-bake` 864538b, `fix/alias-copies-refreshed` d8780f2.
Lecture seule : rien n'a été porté ni publié.

## Où en est chacun

| | Nous (RX 7600, 8 Go) | fxceless (RTX 4070) | Belobug (RX 9070 XT) | macOS (Mac M3/M5) |
|---|---|---|---|---|
| Image | prison éclairée, bâtiments, personnages, ciel ; intérieurs sombres (pas de RT) | première pièce juste, prologue éclairé ; ombres noires (pas de RT) | monde éclairé avec textures, gens, véhicules, végétation, eau ; ombre remplacée par une lumière constante | le monde apparaît à peine (ville sombre) |
| Vitesse annoncée | ~5,8-6 fps | 2,5-4 fps par défaut, 7-8 fps avec tous leurs réglages | 3-5 fps | 0,1-0,2 fps |
| Mémoire | ~19-21 Go RAM, tient dans 8 Go de VRAM | n.c. | ~22 Go RAM (33 Go après 15 min), dépasse 16 Go de VRAM | n.c. |
| Problèmes | peau noire (cuisson), lignes réglées, RT absent | perte du périphérique une fois en monde ouvert | téléphone casse l'éclairage, une perte du pilote | gels Wine, compilateur Apple qui plante |

Les fps ne se comparent pas directement (cartes différentes, scènes différentes), mais nous tournons au moins aussi vite qu'eux sur une carte bien plus petite.

## Ce qu'on a de mieux qu'eux

- **Tenir sur une carte de 8 Go** : textures réduites (TEXTURE_HALF), tas de vues 2^21 bornés pour AMD, garde mémoire du PC, compilations selon la mémoire libre, budget textures selon la VRAM. Aucun des trois n'a ça (Belobug dépasse même 16 Go).
- **Les 195 shaders voxel (GS_VoxelizeGeneric)** traduits (b311bdf, étages ES+GS fusionnés) : aucun des trois ne les a.
- **FSR de l'hôte** (0425b6e, c3ab396) : absent chez fxceless et Belobug (macOS a seulement la version upstream).
- **Sondes d'écran 39f0e9ae** maîtrisées (NO_COMPILE, compilation sans exploser la mémoire) ; Belobug et fxceless n'en parlent pas.
- **Lignes sur les bords de triangles** : réglé chez nous (3024779 + bd85106) comme chez Belobug (03629f3) ; fxceless ne l'a pas encore.
- Cache de shaders indexé par le traducteur (rebuilds à chaud), tour de carte, préchargement, stats à l'écran.

## Ce qui nous manque et vaut la peine d'être porté (classé)

### 1. Vitesse
1. **Belobug « clamps regroupés » (HOIST_CLAMPS)** : ccab973 + 918ab03 + afca189 + 76387c2. Avant chaque dessin indirect le jeu passait par un petit calcul + 3 barrières (9 300 par frame) ; ils les regroupent dans une liste avant les dessins : **225 → 72-80 ms de travail carte par frame**. fxceless dit aussi que c'est la limite principale de la carte. Chez nous la carte travaille 102-128 ms/frame : **c'est le port le plus prometteur pour viser 10 fps**. Code écrit sur upstream : à réécrire sur notre `draw.rs`/`indirect.rs` (~400 lignes).
2. **fxceless 9376fd6 : empreinte rapide dans la synchro des descripteurs** (BLAKE3 → multiplication, cases lues par tables laissées aux dessins) : passe sur le tas **70-220 ms → ~3 ms par frame**. Notre `perf/30fps-pipeline` contient leur synchro (a26859f) mais pas ce correctif : à prendre avant de promouvoir cette branche. ~1 000 lignes.
3. **fxceless f85101b : fil d'envoi propre (VICTORY_ASYNC_SUBMIT=1), file de copie (VICTORY_QUEUE2=1), remplissages sans attente, réutilisation des listes**. Sur 4070 : 3,5-4 → 6-7 fps avec tout activé. Gros (7 900 lignes) et tout derrière des réglages. Note : `VICTORY_ASYNC_SUBMIT`, présent dans d'anciens lanceurs de `_work` (époque d13z5), n'existe plus dans notre code actuel : il n'y fait rien.
4. fxceless a253706 : journal de frame + compteur carte (pour mesurer), surveillance du pilote, **copie d'une texture sur elle-même retirée** (402 en 5 min, interdite par l'hôte), sortie du mode « efficacité » de Windows. 0b0074e : copies en place (COPY_PLAIN), fils sur cœurs rapides (utile seulement sur processeur hybride Intel).
5. Belobug a0a4591 (PR #18) : copies retour sur plusieurs fils (43 → 33 ms, sans gain de fps chez eux).

### 2. Image
1. **Belobug lumière ambiante de remplacement** : 4f45ab8, 0603123, 4d63a09, 4ffc476, cd9f78c. Sans RT, la passe de lumière indirecte laisse du magenta sur feuilles/tissus et des blocs de couleur ; ils la remplacent par une lumière constante réglable (VICTORY_AMBIENT, TINT, SHADE pour assombrir les coins). **Réponse directe à nos intérieurs trop sombres**, en attendant le RT. Petit (~250 lignes).
2. Belobug f520772 : `v_readfirstlane` d'un pixel « assistant » lit sa propre valeur (upstream 0601955). Petit, nous ne l'avons pas.
3. Belobug 2aad398 + 788ea46 : deux fois plus de cases de descripteurs dynamiques (des dessins des scènes lourdes étaient abandonnés) et recherche de case libre qui ne se répète pas (frames de plusieurs secondes).
4. Belobug d88566e : tampons structurés en multiples de 16 octets lus en vue brute (sinon AMD lit l'élément 0). Spécifique AMD, à vérifier chez nous.
5. macOS 4ac4d09 : **copies texture → tampon (relecture)**. Chez nous aussi elles sont ignorées (list.rs : « not done ») ; chez eux le jeu s'endormait juste après. ~120 lignes.

### 3. Stabilité AMD et blocages
1. Belobug 95e935d + 1b0c6b5 : fils de compilation perdus dans le compilateur AMD remplacés, compilations qui ne finissent pas arrêtées (c'est notre souci 39f0e9ae) ; 1503444 + 1ebf29d : nombre d'éléments pris dans la table de l'hôte au lieu d'une requête qui fait planter le compilateur AMD.
2. macOS 7e5f7c3 + 4f55770 + c938e42 : trois blocages définitifs (faute sous le verrou des gardes, en-tête du tas mis en lecture seule, jeu qui suspend un de nos fils). Trouvés sous Wine, mais les deux premiers peuvent arriver sous Windows ; à comparer avec notre `watch`.
3. macOS dd97087 : attente de fin de soumission qui revérifie la barrière chaque seconde si l'événement est manqué. 7258b18 : soumissions retenues pendant le remplissage d'un tas de vues (des listes perdaient leurs vues 30-50 ms).
4. macOS 916a6f8 : DirectStorage, chaque essai jugé sur sa propre copie de la liste de pages. Même sujet que nos murs du diner ; à comparer avec notre 8fd69c0 (attente) plutôt qu'à empiler.
5. fxceless : désormais avec les « fenêtres » indirectes, ils ont eu des pertes du périphérique ; à ne pas activer chez nous sans test.

### Déjà chez nous (rien à faire)
Garde sur toutes les vues d'une page (Belobug ae90937 = fxceless 8dd30ab, dans `perf/30fps-pipeline`), pas de mélange sur cibles entières (Belobug 8629211 = notre 4d5cb27), lecture des quads (Belobug 03629f3 = notre 9975116).

### Pas utile pour nous
macOS : Wine/Rosetta/D3DMetal (masked stores, sin(-0), flotte de tests Mac). Belobug : réglages pour 16 Go et Windows 11 uniquement.

## Proposition (à faire seulement avec ton accord)
Une branche `port/refs-2026-11-25` sur Jouer 9975116 avec, dans l'ordre : (1) HOIST_CLAMPS de Belobug réécrit chez nous, (2) 9376fd6 fxceless, (3) lumière ambiante Belobug, (4) petits correctifs f520772, 2aad398/788ea46, 4ac4d09, a253706 (auto-copie), (5) correctifs AMD de compilation 95e935d/1b0c6b5. Puis un essai de 12 min comparé au Jouer actuel (promu seulement si plus rapide, pas plus de mémoire, image au moins aussi bonne). Le fil d'envoi f85101b viendrait ensuite, à part, vu sa taille.

## Fait (accord de l'utilisateur, 25 nov. ~15 h 15)
Branche `port/refs-2026-11-25` f283970, sur `perf/heap-pass` 7c4eaee (base du fil vitesse) :
- eb849c6 : clamps regroupés avant une série de dessins indirects (Belobug ccab973/918ab03/76387c2 réécrits sur nos listes). `VICTORY_HOIST_CLAMPS=7` par défaut, `0` = ancien comportement.
- 343827a : lumière ambiante à la place de la sortie vide des sondes (Belobug 0603123/4f45ab8/4ffc476/cd9f78c), active seulement sans constructeur BVH. `VICTORY_AMBIENT=off` pour couper.
- f283970 : copie d'une texture hôte sur elle-même ignorée (fxceless a253706).
Branche `port/fx-heap-sync` a898679, sur `perf/30fps-pipeline` 893a56b : empreinte rapide de la synchro des descripteurs + #670 (fxceless 9376fd6, sans le #648).
Compile pour Windows, clippy propre ; pas encore lancé en jeu. Paquet git : `ports-refs/refs-2026-11-25.bundle` (copié dans `_work` sur le PC), confié au fil Sync pour push + essai 12 min.

Écartés après lecture du code :
- macOS 4ac4d09 (relectures texture → tampon) : déjà chez nous (`read_back`).
- Belobug 788ea46 : déjà chez nous (`too_long`). 2aad398 (2× cases dynamiques) : notre tas est borné à 2^21 pour la RX 7600, à revoir à part.
- Belobug f520772 : notre traducteur traite déjà le cas des boucles « waterfall ».
- Belobug 95e935d/1b0c6b5 : couperait au bout de 20 s la compilation des sondes 39f0e9ae (~2 min chez nous) : dangereux.
- Belobug 1503444 : change le traducteur (validation complète nécessaire) : pour le fil shaders si on voit des plantages amdxc.

## Résultat (25 nov. ~17 h Paris)
Promu dans Jouer (Peau3, b74ac64, branche jouer/peau3) : clamps regroupés seuls, copies sur soi-même gardées, lumière douce coupée. 10 min : 6,15 fps (contre 5,74, +7 %), 21,06 Go, image complète.
Lumière douce Belobug : écrit dans une image qui n'est pas notre lumière indirecte, efface murs et vitre. À reprendre après avoir trouvé notre vraie image (journal de la passe de lumière).
Saut des copies sur soi-même : casse la vitre et rend les chemises chromées, désactivé par défaut.
