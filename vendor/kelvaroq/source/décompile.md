# Carnet de décompilation

## Échantillon et méthode

Image PE32+ AMD64, version observée 1.0.2.1, SHA-256 `e95020b2fa4a721db3df59fdecf75621730ac559d8835252f6c3b658d5a08993`. Le chargeur recense 10 sections, 587 imports et deux callbacks TLS. Les adresses de cette page sont des RVA relatives à l’image. L’analyse porte sur le démarrage, les ressources graphiques et la présentation. La couverture de l’exécutable reste partielle.

Trois preuves ont des portées différentes. Le désassemblage établit la forme d’un appel et ses arguments visibles. Une exécution de fonction avec des entrées construites établit son comportement pour ces entrées. Le démarrage depuis le fichier d’origine établit quelles fonctions sont réellement atteintes avec les services actuellement disponibles. Les adresses graphiques ci-dessous proviennent des deux premières méthodes.

## Lecture intégrale du contenu et portée de l’analyse

Le manifeste local couvre les 109 fichiers présents dans le dossier de cette version, soit **224 001 114 630 octets** lus séquentiellement, du premier au dernier octet de chaque fichier. Chaque entrée contient un chemin relatif, une taille et un SHA-256. La liste, la taille, l’identité et la date de modification des fichiers ont été comparées avant et après le parcours. L’empreinte de l’exécutable correspond à celle donnée plus haut. Le manifeste établit une référence pour comparer deux installations. L’authenticité de l’installation demanderait un manifeste de référence indépendant.

| Fichiers | Nombre | Octets lus |
| --- | ---: | ---: |
| Archives `.rpf` | 22 | 161 307 137 472 |
| Conteneurs `.ttpf` | 57 | 62 217 061 888 |
| Exécutable | 1 | 120 181 760 |
| Autres fichiers | 29 | 356 733 510 |

Les archives nommées `shaders.rpf` et `textures.rpf` occupent respectivement 411 695 600 et 192 585 024 octets. Ces tailles sont déjà incluses dans les totaux. Associer une ressource de ces archives à une commande de dessin nécessite encore une trace du jeu.

### Deux lecteurs d’archives reliés aux fichiers

Le contrôle local charge deux pages de code du PE d’empreinte connue dans un CPU de référence et exécute des blocs ciblés avec les en-têtes réels. Les segments exécutés se situent après la lecture du fichier par le système. Le rapport compte séparément les sélections de taille et les entrées des tables.

- **RPF9.** Les 22 fichiers portent les octets `39 46 50 52`. À la RVA `0x1fb268e`, le code compare cette signature. À `0x1fb26a7`, il choisit selon le mode l’une des deux longueurs de préfixe aux offsets 4 et 8. Les 44 sélections ont donné la valeur attendue et sont dans les bornes du fichier. Un en-tête témoin à signature incorrecte emprunte la branche d’échec.
- **RDS5.** Les 57 fichiers portent la signature `RDS5`. La routine `0xf435f0` exige une lecture d’au moins 16 octets sur ce chemin. Elle lit le compteur à l’offset 4, conserve les 40 bits bas du champ à l’offset 8, puis prépare une copie de `8 × compteur` octets depuis l’offset 16. Les 16 octets suivants sont recopiés dans l’objet du lecteur. Le programme de contrôle a relu les tables complètes, vérifié la taille arrondie à 4 Kio, les arguments de copie et le bloc final pour chaque fichier. Les compteurs totalisent **1 026 947 entrées**. Une mauvaise signature et une lecture de 15 octets suivent le chemin de repli. La signification de chaque entrée et le décodage de chaque ressource restent à établir.
- **Portée de la rétro-ingénierie.** Le PE fait 120 181 760 octets. La taille virtuelle de `.text` est de 99 875 174 octets. Le répertoire d’exceptions contient 299 257 entrées de déroulement de pile x64. Ce dernier nombre compte des plages de code décrites pour le déroulement de pile. Il ne fournit ni un nombre exact de fonctions source ni un taux de décompilation.

Pour reproduire avec `$image` défini comme dans le README, depuis la racine du projet :

```powershell
python scripts/audit_content.py (Split-Path -Parent $image) local/audits/content-sha256.json
.\.venv\Scripts\python research/check_archive_readers.py $image
```

### Tables de streaming et bornes des blocs

Dans chacun des 57 fichiers, la dernière entrée délimite la fin des données. Les 1 026 947 entrées représentent **1 026 890 blocs**. La routine à la RVA `0x55b23e0` cherche par position logique. Les bits 0 à 25 d’une entrée donnent une position sur disque en unités de 256 octets. Les bits 30 à 51 donnent la position logique en unités de 4 Kio. Les 12 bits hauts indiquent combien d’octets soustraire à la fin du bloc stocké. La sonde contrôle la progression de chaque position et la concordance de la dernière entrée avec les tailles déclarées.

Le lecteur x64 du fichier d’origine et le calcul indépendant ont donné les mêmes positions et longueurs pour **226 requêtes sur un bloc et 57 traversant deux blocs**. Le lecteur a rejeté les 57 requêtes à la position de fin. Dans les échantillons retenus, 146 flux zlib ont passé la vérification de somme de contrôle et de longueur après décompression. La correspondance d’un bloc avec une tuile de texture reste à tracer au premier dessin qui la consomme.

```powershell
.\.venv\Scripts\python research/check_asset_blocks.py $image
.\.venv\Scripts\python research/check_embedded_shaders.py $image
```

### Conteneurs de shaders embarqués

La sonde retrouve dix conteneurs DXBC aux tables valides dans `.data`. Les en-têtes DXIL annoncent Shader Model 6.6, un programme vertex et neuf pixel. Dans les dix cas, la zone attribuée au bitcode a une taille de quatre octets et contient `NOBC`. Ces dix zones ne fournissent donc pas de bitcode LLVM à soumettre au pilote PC. Chaque conteneur possède une section `PRIV`, dont la signification reste à déterminer. Il reste aussi à identifier quels programmes le jeu soumet effectivement au GPU.

Dans chacun des dix conteneurs, le lecteur de VIctory identifie un programme natif à partir de `XDAT` et `XBCH` dans la partie DXIL. Il décode 43 instructions dans le vertex shader et, au total, 113 dans les neuf pixel shaders. Son traducteur produit dix programmes hôtes. Avec des états graphiques de test, les pilotes de la RTX 5080 et du Radeon intégré acceptent chacun les neuf paires vertex/pixel. Le banc transmet les conteneurs au lecteur en mémoire. La création d’un état de pipeline atteste l’acceptation de la paire par le pilote. Une soumission provenant du démarrage normal du titre reste à observer. [Source du traducteur utilisé](https://github.com/fxceless/victory-fork/tree/2119b0e44c53aa9648307b0c5c73590290305b1f/crates/victory-shader)

```powershell
.\.venv\Scripts\python research/check_shader_reference.py $image --reference $clone --draw
```

Pour le dessin contrôlé, le vertex shader à `0x688e13c` lit les positions et couleurs de trois sommets synthétiques. Le pixel shader à `0x688f346` restitue la couleur interpolée. Après le dessin et l’attente de la fence, les deux pixels du haut de la cible RGBA32F 2 × 2 valent exactement `(0.25, 0.5, 0.75, 1)`. Les deux du bas conservent `(0, 0, 0, 0)`. La relecture concorde sur les deux adaptateurs. Le banc de recherche choisit cette paire. Son usage par le menu reste à tracer.

## Démarrage effectivement exécuté

| Ordre | RVA | Observation |
| --- | --- | --- |
| 1 et 2 | `0x55dd4f4`, `0x55e2ebc` | Les deux callbacks TLS reviennent après attachement du processus. |
| 3 | `0x56a1ec8` | Le point d’entrée commence. Le code invité initialise son cookie et demande les identifiants de processus et de thread ainsi que des horloges. |
| 4 | `0x55dd354` | Un initialiseur C résout des services de synchronisation via les modules virtuels. La table C contient quatre cases dont trois non nulles. La table C++ contient 1 115 cases dont 1 114 non nulles. Ces effectifs décrivent la table du fichier. |
| 5 | `0x5c1b680` | Quatrième initialiseur entré sur ce parcours. L’appel à `memset` dont l’adresse de retour est `0x14714b4` se termine dans la référence actuelle. |
| 6 | `0x4e72fdf` | Écriture non alignée de 32 octets de YMM0 vers l’adresse RCX. Elle reproduisait une instruction invalide dans Unicorn 2.1.4. Une fixture indépendante vérifie l’écriture sur une adresse impaire, les voies hautes et les permissions. Le démarrage franchit désormais cette instruction. |
| 7 | `0x55debac` | Appel de `advapi32.dll!RegOpenKeyW`. Adresse de retour `0x55debb2`. L’ouverture, la lecture et la fermeture aboutissent dans les deux profils. |
| 8 | `0x1e1ceec` | Copie vectorielle non alignée avec `VMOVDQU`. Le contenu complet des deux voies est conservé par la référence. |
| 9 | `0x1e1cfe0` | Appel de `d3d12_xs.dll!D3D12ConfigureVirtualMemoryX`. Le service renseigne les bornes invitées, puis l’appel revient à `0x1e1cfe5`. |
| 10 | `0x1e1d00a`, `0x1e1d039` | Deux appels de réservation réussissent sans engager de pages. Les plages distinctes figurent dans le JSON. |
| 11 | retour `0x1e1d186` | Première allocation avec engagement, type `0x20003000`. L’invité reçoit une base alignée sur 2 Mio et poursuit l’initialisation. |

La fonction à `0x55deb88` prépare le handle prédéfini `HKEY_LOCAL_MACHINE`, un sous-chemin UTF-16 et une sortie sur la pile. Elle teste le code de retour de l’ouverture. En cas de succès, elle cherche la valeur `ConsoleMode` dans `Software\Microsoft\Windows NT\CurrentVersion\OEM`, puis ferme la clé. La suite compare la donnée lue à plusieurs constantes. Les valeurs de modèle proposées par le dépôt étudié, `0x64` et `0x60`, produisent respectivement les types 7 et 6 en exécutant cette fonction avec le registre virtuel. Cette expérience confirme la branche du binaire. La valeur n’a pas été lue sur une console physique.

### Configuration de la mémoire graphique

Au retour `0x1e1cfe5`, le code teste le HRESULT. Le premier appel de réservation exécuté est `XMemVirtualAlloc` à `0x1e1d00a`. La valeur déjà préparée dans RSI donne la taille prévue de la réservation virtuelle. Le test n’a engagé aucune de ces quantités en mémoire physique.

| Profil | Résultat de la détection | DWORD à l’offset 4 de la structure de 32 octets | Taille prévue pour la réservation suivante |
| --- | --- | --- | --- |
| Series X | 7 | 4 | 24 Gio |
| Series S | 6 | 3 | 16 Gio |

Les sept autres DWORD valent zéro dans les deux exécutions. Cette mesure a révélé un défaut du moteur de référence initial. Unicorn 2.1.4 acceptait un `VXORPS xmm0,xmm0,xmm0` tout en conservant les 128 bits hauts de YMM0. Une instruction de stockage ultérieure plaçait donc des octets anciens dans la structure. Un test de régression démarre le registre entièrement à un et vérifie les 256 bits après l’instruction. La suite couvre aussi les déplacements vectoriels non alignés et `VZEROUPPER`.

Au jalon précédent, la trace comportait deux entrées dans `completed`, puis `stage = entry`, `initializers_entered = 4` et `stop.return_rva = 0x1f676a6`. La suite synthétique peut être rejouée indépendamment du fichier.

### De l’allocation à la topologie du processeur

Le même démarrage crée dix sémaphores anonymes et engage quatre régions de mémoire invitée dans chaque profil. Leur taille cumulée est de 4 750 049 280 octets en Series X et de 3 031 433 216 octets en Series S. Le compteur porte sur la mémoire engagée par le CPU invité. La VRAM de la carte hôte est mesurée séparément lorsqu’un travail graphique lui est soumis.

L’initialisation utilise une diffusion, une addition et une permutation de mots de 64 bits. Les tests vérifient les voies hautes, le débordement entier et la lecture d’un mot en fin de page. La boucle à `0x554f6f3` avance de 20 octets par entrée. Dans le profil X, le registre portant la taille de la table vaut 131 072. À la fin d’une tranche d’instructions, le CPU reprend avec les mêmes registres et sous le même délai global.

Le premier appel de `GetLogicalProcessorInformation`, à `0x1f676a0`, passe un tampon nul. Le code attend l’erreur 122 et la taille requise, puis effectue un second appel. D’après le désassemblage à partir de `0x1f67711`, il parcourt des enregistrements de 32 octets, compte les cœurs et additionne le nombre de bits de leurs masques. Le manifeste local demande `SMTEnabled=true`. Les masques effectivement présentés au titre sur console restent à vérifier.

### Comptage des cœurs et arithmétique des initialiseurs

Au jalon `4f26dcb`, les profils Series X et Series S entraient dans le 36ᵉ initialiseur, à `0x24709c0`, puis s’arrêtaient sur `VPXOR ymm14,ymm8,ymm3` à `0x24716b3`. Les cinq régions engagées totalisaient 4 771 020 800 octets en X et 3 052 404 736 en S. Les deux listes de cases libres décrites ci-dessous permettent de vérifier le résultat de cette boucle.

Au dernier démarrage vérifié avec le moteur de référence, les deux profils terminent leurs deux callbacks TLS et entrent dans le 72ᵉ initialiseur, à `0x2b8fb20`. Il s’arrête sur `vpsrlq ymm3, ymm0, 2` à la RVA `0x1dd6538`, avant la première présentation du jeu. Le compteur mesure les entrées dans les initialiseurs. L’exécution native avec VIctory est décrite dans [le journal d’architecture](documentation.md#composants-à-reprendre-pour-aller-au-menu).

Le binaire écrit les totaux de cœurs et de processeurs logiques aux RVA `0x71be5c0` et `0x71be5c4`. Sur le PC de développement, ces données valent 7 et 14 avec le modèle console, 16 et 32 avec la topologie Windows. La boucle à `0x1ad0fa4` construit un masque avec un décalage sur 32 bits. Pour 14, le masque vaut `0x3fff`. Pour 32, le décalage repart à zéro et le masque calculé vaut zéro. Une autre routine fournit explicitement `0x3fff` à l’appel d’affinité à `0x577fa9b`. La découverte de topologie fonctionne. Le placement effectif des threads invités reste à implémenter.

La référence corrige aussi des opérations vectorielles atteintes par les initialiseurs. Un test reproduit `VSUBPS` à la RVA `0x4fa34d8` avec deux sources et un ancien destinataire différents. Sur Unicorn 2.1.4, la première voie utilisait la valeur de l’ancien destinataire. Le test impose désormais les quatre différences attendues et la remise à zéro des bits hauts de YMM. Les tests des multiplications-additions fusionnées vérifient le résultat arrondi une seule fois et la restauration de l’environnement flottant de l’hôte.

### Initialisation de deux listes de cases libres

La première boucle, de `0x247164d` à `0x24719f4`, effectue 17 tours. Les 270 mots de 16 bits à `0x75a635c` prennent dans l’ordre les valeurs 1 à 270. Le mot suivant, initialisé à zéro, est la tête de liste. Le prélèvement lit le lien de la case en tête et la restitution réinsère la case libérée. La deuxième boucle effectue 32 tours et écrit les liens 1 à 512 à `0x75a92e0`. La sonde ci-dessous observe chaque sortie pendant le démarrage normal, compare chaque lien et vérifie la préservation des données voisines.

```powershell
.\.venv\Scripts\python research/check_startup_tables.py $image
```

Le moteur de référence garde en cache au plus 2 048 décodages x64 par machine virtuelle. La clé inclut l’adresse et les octets relus lors de l’exécution. Sur le PC de développement, pour six mesures alternées sur un programme synthétique de 10 000 tours, la médiane d’une boucle de XOR vectoriel est passée de 459 à 174 ms. Avec un chargement vectoriel en mémoire, elle est passée de 646 à 363 ms. Ces mesures portent sur le décodage pendant une boucle CPU. Le temps d’une image du jeu demande une capture de présentation.

## Textures, barrières et dessin

| RVA | Observation | Conséquence à tester dans le backend |
| --- | --- | --- |
| `0x979c75`, `0x979c9f` | Création d’un calculateur de texture, puis appel virtuel à l’offset `+0x10` pour son layout. | Vérifier le placement par sous-ressource avec une texture à motifs distincts par mip, couche et plan. Contrôler les données après transfert. |
| `0x95e0`, `0x3210d` | Construction de groupes de barrières de type 2, en-tête de 16 octets et entrées de 40 octets portant une ressource. Le chemin de soumission rejoint le slot `+0x490`. | Des entrées synthétiques de zéro, un et deux groupes ont servi à contrôler le comptage. Conserver la granularité de chaque ressource et tester la visibilité des écritures. |
| `0x24800d5`, `0x247fe66` | Un chemin tente le pipeline en cache via `+0x218`. Un résultat nul permet un repli via `+0x1f0`. | Tester le cache froid et chaud avec le même résultat de dessin. Une entrée de cache manquante peut mener à un pipeline valide. |
| `0x4e33f91`, `0x555e976` | Deux sites d’`ExecuteIndirect` passent une commande maximale, une ressource d’arguments et un décalage, sans tampon de comptage sur ces sites. | Tester le contenu et le décalage effectifs du tampon avant toute optimisation de regroupement. Ces deux sites ne décrivent pas tous les dessins. |

Les deux appels chargent le même appareil (`0x8759d00`) et demandent `ID3D12PipelineState`. Après traitement du HRESULT, le test à `0x24800e4` renvoie zéro si `+0x218` a échoué, sinon le pointeur de sortie. Une clé absente ou un pointeur nul conduit au repli. À `0x247fe66`, `+0x1f0` reçoit zéro extension et écrit dans le champ `+0x120`. Le test à `0x247fe70` accepte tout HRESULT non négatif sans revérifier le pointeur. En cas d’échec, le code appelle `Release` sur l’objet en `+0x118` s’il existe, rejoint le nettoyage commun et renvoie faux. L’appelant à `0xf9fb81` propage ce booléen. Celui à `0x247fd0c` l’ignore et renvoie vrai. Dans la vtable de l’appareil du fork, `+0x218` met la sortie à zéro et renvoie `E_NOTIMPL`, tandis que `+0x1f0` délègue à la création du pipeline de calcul. Les entrées de mêmes offsets dans l’interface XStore appartiennent à l’installation et à la désinscription. [Vtable de l’appareil](https://github.com/fxceless/victory-fork/blob/2119b0e44c53aa9648307b0c5c73590290305b1f/crates/victory-gpu/src/device.rs#L438-L441) · [Création du pipeline de calcul](https://github.com/fxceless/victory-fork/blob/2119b0e44c53aa9648307b0c5c73590290305b1f/crates/victory-gpu/src/pipeline.rs#L572-L585) · [Vtable XStore](https://github.com/fxceless/victory-fork/blob/2119b0e44c53aa9648307b0c5c73590290305b1f/crates/victory-hle/src/modules/xstore.rs#L398-L403)

## Présentation et reconstruction temporelle

- La fonction à `0x28f03db` prépare une structure temporelle. Le chemin observé utilise l’identifiant `0x10001`, inverse le signe du jitter, fournit des facteurs unitaires dans ce cas et écrit un indicateur de réinitialisation à `+0x198`. Il reste à vérifier quels tampons et quelles unités arrivent réellement à chaque image.
- Les fonctions à `0x24821dd` et `0x24824c1` utilisent un identifiant de trame issu du même global à `0x75e0e88` pour la soumission et la présentation. Des appels synthétiques à un et deux plans donnent un pas de `0x48` octets entre descripteurs de plans. Un identifiant commun permettrait de relier les traces CPU et GPU lorsque le chemin sera raccordé.

La référence actuelle a réservé deux fois 24 Gio d’adresses virtuelles dans le profil X et deux fois 16 Gio dans le profil S. Le troisième appel demande une allocation avec engagement, respectivement de `0x49400000` et `0x48a00000` octets. Les indicateurs valent `0x183` et la protection demandée `0x22004`. Les accès CPU invités sont implémentés pour ces plages. La visibilité des écritures entre CPU et GPU reste à vérifier lors de la création des ressources. Les observations statiques n’établissent encore ni la cause des textures manquantes d’une scène particulière ni un débit d’images du jeu sur PC.
