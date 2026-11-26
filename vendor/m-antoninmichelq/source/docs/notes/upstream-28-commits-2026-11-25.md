# Les 28 nouveautés de VIctory officiel (ffe310e..70564ae) : est-ce que ça vaut le coup ?

Verdict court : **notre version devient une variante à part** (base fxceless + nos corrections), et on rapporte **une dizaine** de nouveautés une par une. Environ la moitié des 28 existent déjà chez nous sous une autre forme. Le reste est soit du diagnostic, soit des tests, soit trop lié à la façon de faire d'upstream.

## À rapporter (utile pour toi)

| Commit | Ce que c'est | Pourquoi |
|---|---|---|
| 9c58a1c + 286e33f + 7389303 + c47b529 + 819f622 | Pré-construction des pipelines et des shaders à partir du pack du jeu (`translate --cache --pipelines`) ; topologie explicite des pipelines mesh pour la bibliothèque | Démarrage rapide même pour une nouvelle version ; c'est le plus gros gain de confort |
| 86a5ed6 | Vues qui ne commencent pas sur un élément : leur propre copie ; copies d'octets du tas recopiées | Nos journaux montrent encore « buffer view not on an element of its host copy » : constantes de culling et tables de couleurs lues à zéro |
| 77053a0 | Précision des shaders : discard, arrondi f16, dénormaux, med3, zéros signés | Peut corriger des défauts visuels ; on a déjà fait une partie (arrondi f16) |
| 13091f6 | Entrées/sorties pixel et mesh : registres réutilisés, empaquetage 16 bits | Lié aux « aiguilles » et aux cheveux des personnages |
| e0034c2 | Bornes des déplacements relatifs aux registres | Shaders plus justes et plus légers |
| a7fc47a | Vivacité des phi | Petite correction de traduction |
| 4f57f46 | Échantillons cube avec gradients | Petit, améliore les reflets |
| 0b35b7f (partie restante) | Atomiques 64 bits sur images, intersection BVH sur l'hôte | Le FSR est déjà repris ; le reste aide les pré-passes alpha et le RT |
| 328eec1 | Tests de sémantique des instructions | Aucun risque (que des tests), attrape les bugs de traduction |

## Déjà chez nous autrement (on saute)

- 27c3dc9, 0f63757 : mémoire du jeu par table de pages. Notre base a sa propre version (copies numérotées).
- c75fdba, bacb06a, 6d021fe : vues du tas et journal des changements. C'est une autre conception que la nôtre. Il faudra comparer leurs idées de performance plus tard.
- 683ad7a : rectangles par geometry shader. On l'a déjà (rect.rs).
- 75677cb : taille des buffers lue dans les descripteurs. On l'a déjà (09cd958).
- 4051149 : vues nulles. On l'a déjà.
- 8af0472, d71c388, 13bbd6f : masques de lanes, vues bornées, jumeaux de profondeur. Ça existe en grande partie ; à vérifier au cas par cas si un bug pointe vers eux.

## Sans intérêt pour jouer

- 532cb7c (VICTORY_POKE) et 7000573 : diagnostic.
- 224e786 : tolérance d'un test.
- 70564ae : README.
