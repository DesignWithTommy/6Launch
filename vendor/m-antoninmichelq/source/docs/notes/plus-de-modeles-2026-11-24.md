# Afficher le plus de modèles 3D possible (2026-11-24, ~15:00 Paris)

## Ce qu'on sait de CGBJ (public)
Seule source lisible : PortWatch (fan tracker), plus un tweet et une vidéo de 77 s sur X (non lisibles d'ici).
- Code privé, basé sur VIctory + un peu de xcl. Aucune explication technique publiée, aucune citation du groupe.
- Leur image publique est une **vue de debug des normales** (« no textures or lighting »), exactement comme notre raccourci « vue 3D ». La vidéo « couleurs plates » est sans doute une vue de debug du même genre (normales ou albedo du G-buffer).
- Quand le joueur marchait, l'écran du jeu normal était noir.
Conclusion : CGBJ n'a pas de méthode secrète connue pour afficher les modèles ; ils affichent une cible du G-buffer au lieu de l'image finale, comme nous. La différence vient de quels shaders passent.

## Pourquoi des modèles manquent chez nous (D13s, diag)
Les draws eux-mêmes ont presque tous leur pipeline (« being made » retombe à 1-10/frame après quelques minutes). Ce qui manque, ce sont des **compute non traduits** : 236 + 205 + 135 + 110 (indirect dispatch) + … par image. Ce sont en grande partie les passes de culling / préparation des draws indirects : si elles ne tournent pas, les arguments des draws indirects ne sont pas écrits et les objets n'apparaissent pas. Familles en tête : « typed root view », « write through address in root constants » (f6e47efa), « addr not from root param », « addr from phi ».

## Plan pour en afficher le plus possible
1. **Vue couleurs** (sans code) : `VICTORY_PRESENT_TARGET` sur la cible albedo (format 28 ou 29, n = 0..2, même taille que la vue 3D). → Sync.
2. **Vue « éclairage de secours »** (code) : à la présentation, combiner albedo × (ambiant + N·L avec une lumière fixe) à partir des deux cibles du G-buffer. On obtient une image lisible, ombrée, de tout ce qui est dessiné, même si l'éclairage du jeu reste noir. Option `VICTORY_PRESENT_SHADE=albedo_spec;normal_spec`. → fil shaders.
3. **Classer les compute manquants par « objets cachés »** (diag) : marquer un dispatch abandonné dont un UAV est lu plus tard comme arguments indirects (ou buffer d'index/visibilité). Ceux-là cachent des modèles : à corriger en premier. → fil shaders.
4. **Passer la jambe `VICTORY_ROOT_UAVS=1`** (déjà prévu) : supprime « typed root view » + f6e47efa, les plus gros abandons. → Sync.
5. **Shader pixel de secours** : si le pixel shader d'un draw échoue ou si l'hôte refuse la paire (E_INVALIDARG mesh f5e50eb2 + pixel ce3f0432 ; made=false fe660f2d + b0e293b1), faire la pipeline avec un pixel shader générique (couleur unie ou normale) au lieu de jeter le draw. Gain modeste. → fil shaders.
6. **Attendre les pipelines graphiques au début** (option `VICTORY_DRAW_WAIT=1`) : 620 draws/frame « being made » pendant les premières minutes ; attendre plutôt que sauter fait apparaître les modèles dès les premières images (au prix de la fluidité). → fil fps.

Licences : rien de CGBJ (privé), idées seulement ; tout le reste est notre code GPL-3.
