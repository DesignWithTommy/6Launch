# Code existant à reprendre pour le sol et les bâtiments (2026-11-24, ~16:45 Paris)
Base comparée : fork victory-stack-d13z2 (tip f45ce64). Détails : upstream-prs.md, forks.md, xcl-idees.md (même dossier).
Tous les PR upstream ci-dessous sont non fusionnés et entrent en conflit avec d13z2 (portage à la main). Exclu : #234.

| Rang | Source | Ce que ça règle | Dans d13z2 ? |
|---|---|---|---|
| 1 | VIctory #645 (499bd98) | Adresses jointes par phi / pointeur porté par une boucle, arithmétique d'adresse (sub, s_addk, lshl_add, v_add3, masques) → 6f6a359d, "addr not from root param" | non (partiel : phi même base dans resource.rs::pointer_address) |
| 2 | VIctory #948 | ClearUnorderedAccessViewUint/Float pas implémenté : les passes de compactage remettent leurs compteurs à 0 avec ça | non |
| 3 | VIctory #956 (71967c1, db86e1e) | ExecuteIndirect avec signatures à plusieurs arguments (constantes racine + draw/dispatch) ; list.rs::execute_indirect n'accepte qu'un argument et jette le reste | non |
| 4 | VIctory #650 + #709 (3 commits) | Accès par vue racine bornés (faute de page RX 7600 avec VICTORY_ROOT_UAVS=1) ; "typed root view" via vue brute | non / partiel |
| 5 | VIctory #1056 (417f4e3), #1012 | s_load depuis l'adresse d'une vue racine lit la mémoire (1er pas d'une chasse de pointeur) ; phi de phi pour descripteurs | non |
| 6 | VIctory #242 (2 commits sur 3) | Buffers d'arguments/compteurs écrits par compute passés en état "indirect argument" avant clamp + ExecuteIndirect | partiel |
| 7 | VIctory #986 (d8d7c2c) | Vues non alignées copiées dans un scratch aligné au lieu d'être refusées ; à étendre aux vues brutes/racine pas sur 16 octets (c1488f48) | non |
| 8 | xcl (GPL-2, idée seulement) | Chasse de pointeur 9205d265 : aucun PR VIctory. Idée : repli "mémoire plate" = un grand UAV brut racine sur le tas de la mémoire GPU du jeu, accès à va - base. Règle aussi typed root view / alignement 16 octets | non |
| 9 | xcl (fait) | Buffers de constantes : stride 16, nombre d'enregistrements = taille/16 ; sinon chaque lecture indexée renvoie la ligne 0 (plans du frustum faux → tout est éliminé). À vérifier chez nous | à vérifier |
| 10 | xcl (fait) | Barrière UAV globale + transition INDIRECT_ARGUMENT avant ExecuteIndirect quand plusieurs ressources couvrent la même mémoire | à vérifier |

Déjà équivalent chez nous (ignorer) : #489/#1077 (unspill, root_uavs), #667/#710 (POINTER_SLOT), #40/#583 compteurs, #123/#586, #493/#994/#704.
Faible valeur : #1066 (perf), VICauseway, victory-shader-census.
