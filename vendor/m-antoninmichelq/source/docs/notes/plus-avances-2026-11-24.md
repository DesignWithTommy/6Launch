# Qui est le plus avancé avec VIctory (recherche 24/11/2026 ~20:30 Paris)

## 1. fxceless/victory-gta6 — le plus avancé (licence GPL-3, réutilisable)
- https://github.com/fxceless/victory-gta6 (README "Status of this fork")
- Dit afficher : cinématique d'ouverture éclairée du début à la fin, et la 1re scène jouable (prologue) avec
  personnages, textures, ciel, extérieur au soleil, HUD et manette. 2 à 4 images/s. Ce qui est à l'ombre est
  noir (éclairage par lancer de rayons pas encore fait). Testé sur NVIDIA RTX 4070 uniquement.
- Confirmé par quelqu'un d'autre : issue VIctory #1528 (RTX 4070, Linux) « le monde s'affiche (la ville au
  crépuscule) avec le fork victory-gta6 ». https://github.com/Marvisak/VIctory/issues/1528
- Version stable selon l'auteur : commit 578d459 (sur VIctory main ffe310e). Les commits plus récents
  perdent la carte graphique chez lui. Copié sur notre fork : branche import/victory-gta6-578d459.
- Points clés de leurs corrections (README) : le tri des meshlets lisait ses tampons avec le mauvais type
  de vue (le monde n'affichait presque rien) ; la lumière calculée par les computes était relue depuis des
  copies vides (ciel, sondes, brouillard) ; tables de descripteurs par draw épuisées ; échantillonnage dans
  les computes (730 programmes) ; constructeur BVH (ray tracing) désactivé par défaut (VICTORY_BVH_BUILDER)
  — cohérent avec nos 2 computes qui faisaient planter le jeu (caa78b8c, 28918b71).
- Un seul gros commit de 32 000 lignes : on ne peut pas le fusionner morceau par morceau dans notre version
  sans gros travail. Le plus rapide : le compiler tel quel pour la RX 7600 et le tester.

## 2. Autres pistes
- PR #1533 (Xvania7) : l'instruction de ray tracing traitée comme « rien touché » pour que les computes
  d'éclairage se traduisent. https://github.com/Marvisak/VIctory/pull/1533
- Issue #841 : prologue rendu dans les cibles 2560x1440 (RTX 3090). Issue #940 : maillages du monde dans le
  G-buffer (RTX 5080). Issue #752 : en jeu sur AMD 9070 XT (Linux).
- PortWatch (4lwayz/portwatch-vi) : CGBJ (privé) en jeu en couleurs plates sans lumière ; personne d'autre
  n'a de monde éclairé. victory-gta6 n'y est pas encore listé.
- AnyXboxX : image 3D encore noire.
