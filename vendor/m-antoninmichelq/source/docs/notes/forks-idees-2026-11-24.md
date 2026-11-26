# Ce que les projets dérivés de VIctory / xcl font mieux (2026-11-24, ~14:30 Paris)

Comparé à notre pile victory-stack-d13s (4cd8549). Revue xcl précédente : xcl-idees-2026-11-24.md (pas répétée ici).

## Où en sont les autres (tracker PortWatch, forks GitHub)
- **Personne n'affiche une scène éclairée.** Le plus avancé, **CGBJ** (code privé, basé sur VIctory + un peu de xcl), est en jeu : un joueur marche, parle aux PNJ, mais l'image est « en couleurs plates, sans lumière » et très lente. Leur image publique est une vue des normales, la même chose que notre raccourci « vue 3D ».
- xcl et ses forks (xcl-openworld, xcl-compat-fixes, xclayer) : menus à 30 fps, monde chargé mais écran de chargement, puis plantage GPU / chien de garde du jeu au bout de 3 à 14 min. Pas d'image du monde.
- VI Causeway (dreaming66, GPL-3, dérivé de VIctory) : 0,6 à 1,4 fps, cartes d'intro visibles, monde sombre.
- justr4mzi/VIctory (GPL-3) : pile de PR communautaires pour RX 6700 XT, 1 fps, premières lumières lointaines, puis DEVICE_HUNG.
- Nous : pas de plantage, ciel + lumières + silhouettes ; les surfaces restent noires. **On est au niveau des meilleurs.** Les pistes ci-dessous sont donc des briques précises, pas un projet « qui marche » à copier.

## Licences
- Forks VIctory, VI Causeway, victory-shader-census : GPL-3 → le code peut être repris (garder les auteurs).
- xcl et tous ses forks : GPL-2-only → idées seulement, jamais de code.

## Classement des pistes (de la plus utile à la moins utile)

### 1. Surfaces noires : le shader ScreenProbes dont la pipeline ne finit jamais
Notre run D13r : le compute 39f0e9aeb156a61c se traduit mais la création de pipeline D3D12 ne se termine jamais (283 s). Ce que font les autres :
- **victory-shader-census** (Rafa23xz, GPL-3, utilisable tel quel) : mesure hors jeu, shader par shader, chaque étape, dont le **validateur DXIL** (`--validate`) et la **création de pipeline sur le GPU local** (`--pipelines`). Permet de reproduire le blocage de 39f0e9ae sur le PC sans lancer le jeu, et de tester des corrections en minutes.
- xcl (idées) : compilation en arrière-plan des grosses pipelines compute (2 à la fois), **pré-compilation des pipelines hors jeu** pour remplir le cache du pilote AMD (41,6 s → 0,2 s au lancement suivant), et surtout **flux de contrôle structuré** (if/boucles au lieu d'un « switch pc » géant) : DXIL 470 → 310 Ko et temps de compilation divisé. Le pilote AMD refuse même des CS de 15 720 instructions (manque de mémoire).
- PR amont #491 (journal des pipelines qui plantent) et #546 (priorité aux pipelines demandées par les draws, **déjà chez nous**).

### 2. Copies 1,4 s et 4,2 Go par image + mémoire qui monte de 3-4 Go/min
- **PR #472** (amont, ouverte) : l'instantané d'écriture grandit en place et le pool inactif est plafonné à 256 Mo (avant : jusqu'à 8 × 174 Mo = 1,4 Go gardés). Correspond à notre montée mémoire.
- **c326bfa** (justr4mzi, issue #776) : on ne demande au noyau l'état d'une page qu'une fois par soumission (1 500-1 800 demandes par image, 120-180 ms sur RX 9070 XT).
- **468e745** (VI Causeway) : réutiliser les vérifications de pages inchangées entre plages ; **cc4ee22**, **c8fb88f**, **a3ecbd1** (même famille).
- **6ad68a7** (gist #783) : VirtualQuery par région en secours.
- Aucune n'est dans d13s.

### 3. Surfaces noires : DXIL invalide acceptée quand même
Le census montre que des centaines de shaders émis échouent au validateur DXIL mais deviennent quand même des pipelines (VIctory signe lui-même le conteneur). Effet possible : noir ou blocage GPU. Cas vus : atomique image 64 bits sur une ressource du heap annotée float4 sans le drapeau AtomicInt64OnHeapResource, échantillonnage à LOD implicite dans un compute, sorties mesh > 32 Ko. À faire : passer nos traductions D13 au validateur (dxil.dll du SDK).
Briques liées, absentes de d13s : **#568 / #581** (descripteur nul du bon type pour une case vide : sinon comportement indéfini), **#559** (états des mips des cibles couleur : chaînes de flou/bloom), **#469** (UAV de profondeur via texture jumelle), **bb0dde8** VI Causeway (garder les textures nommées par les barrières UAV/aliasing).

### 4. Les 70 shaders encore refusés : PR qui visent nos familles
| Famille (D13r) | PR / commit candidat |
|---|---|
| packed input half (2 pixel) | #592 (2e ligne d'entrées 16 bits), #569 |
| LDS op / LDS non alloué | #571, #632, #628 (ds_min/max_f32) |
| v_permlanex16 | #404, #576 |
| f64 | #416 (v_cvt_f32_f64) |
| lanes held elsewhere (9) | #298, #309 (fil personnages au courant) |
| typed root view (26) / addr not from root (19) | #667, #710, #709, #650, #302, #489 |
| autres | #539 / #705 atomiques image 64 bits, #694 sample_d_cl / gather4_l, swaprel (fork talakasse), SMEM size + masques débordés (fork Wasler, #728) |
Le fork justr4mzi a déjà résolu les conflits de beaucoup de ces PR sur ffe310e (branche ultimate-rx6700xt) : bonne source pour les reprendre.

### 5. Outils
- Le census montre que l'ISA de la console est **GFX10.1 (gfx1013)**, pas gfx1030 : v_mac_f32 / v_mad_f32 existent, et VOP3 0x140 est v_mad_legacy_f32 (non fusionné). Nos décodages avec `llvm-mc -mcpu=gfx1030` peuvent donc se tromper : utiliser gfx1013.

### 6. Montrer la scène complète en couleurs (comme CGBJ)
CGBJ montre l'ouverture « en couleurs plates sans lumière ». Chez nous, `VICTORY_PRESENT_TARGET=<l>x<h>:<format>:<n>` peut afficher une autre cible du G-buffer que les normales : en essayant n = 0..3 avec un format RGBA8 (28 / 29), on devrait trouver la cible **albedo** et faire un raccourci « vue couleurs ».
