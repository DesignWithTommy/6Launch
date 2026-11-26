# Ta variante VIctory : point du 25/11 au matin

## En bref
- Le raccourci **Jouer VIctory (tout cumulé)** a une nouvelle version : **cdfbe3a avec préchargement des shaders**. Elle tourne à environ 6,4 fps, se stabilise à 20,2 Go et n'a pas planté en 12 min. Les shaders sont prêts avant d'arriver dans le monde.
- L'ancienne version (64dc663, 6,1 fps, 21 Go) reste dans `_work\Jouer precedent.bat`.
- Le brouillard violet est réparé, mais pas encore sur le raccourci : avec la réparation, les murs de la cellule disparaissaient au 2e lancement. Un test avec un cache neuf est en cours.
- Ta variante tient en 6 PR (#8 à #13), toutes avec la CI verte. Rien n'est fusionné sans toi.

## Les PR (dans l'ordre)
| PR | Contenu |
|---|---|
| #8 | Ta branche principale = la version stable du 25/11 |
| #9 | Préparer les shaders à l'avance (`victory translate --cache`) |
| #10 | Niveau de mip correct pour les cubes avec gradients (reflets moins flous) |
| #11 | Tests d'instructions d'upstream + 3 corrections du traducteur |
| #12 | La version « rapide » (correction mémoire, ~6 fps) |
| #13 | Le préchargement des shaders (la version Jouer de ce matin) |

## Les tests de la nuit (12 min chacun, sur ton PC)
| Version | Résultat | Retenue ? |
|---|---|---|
| 64dc663 « rapide » | 21 Go stable, 6,1 fps | Oui (cette nuit), puis remplacée |
| Préchargement 44add24 / e19cf03 | Plus lent, ou trop de mémoire au 2e lancement | Non |
| Préchargement bd5b339 / cdfbe3a avec « tout en avance » | Très bon au 2e lancement, mais 1re partie bloquée à 1,5 fps | Non |
| **cdfbe3a sans « tout en avance »** | **20,2 Go, 6,4 fps** | **Oui : raccourci Jouer** |
| Sondes de lumière réparées (e207975) | Plus de violet, 19,4 Go, mais ~10 % moins rapide | Option |
| Préchargement + sondes (7ae151c) | Plus de violet, 6,5 fps, mais murs absents avec son propre cache (2 fois sur 2) | Pas encore |
| Sondes allégées (cbf9c38) | Plantage de la carte graphique au 1er lancement | Abandonnée |

## En cours
- Le tour de la map (téléportations et préchargement à chaque endroit) : 1 h 30 au maximum.
- Ensuite, la version « préchargement + sondes » avec un cache neuf. Si les murs sont bien là, elle passe sur le raccourci, et la version actuelle reste dans `_work` (« Jouer sans sondes (plus rapide) »).

## Ce qui n'a pas été repris d'upstream, et pourquoi
- **9c58a1c** (pré-construction des pipelines) : en conflit avec notre file de pipelines. Notre préchargement fait le même travail.
- **86a5ed6** (vues décalées dans un tampon) : notre gestion de la mémoire du GPU est trop différente de celle d'upstream.
- **77053a0, 13091f6, e0034c2, a7fc47a** (précision des shaders) : ils dépendent de parties du traducteur d'upstream qu'on n'a pas. Les tests de la PR #11 ont permis de corriger les vrais écarts.
