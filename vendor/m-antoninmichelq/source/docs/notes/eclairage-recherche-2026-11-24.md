# Textures + lumière (calculs GPU du visibility buffer) — recherche 2026-11-24 ~19:30 Paris

Chaîne: G-buffer = visibility buffer (ID objet + UV) -> computes matériaux/lumière -> composite. Ces computes
(3b90165a, caa78b8c, 28918b71, f6e47efa, famille "typed root view" ~20-26) ne se traduisent qu'avec
VICTORY_ROOT_UAVS=1, et ce mode fait planter le jeu au chargement du monde (le CPU du jeu lit 0xffff... ;
patterns/d13z4-crash.txt). Donc la porte d'entrée = rendre le mode root-UAV stable.

## Ce que font les autres
- AnyXboxX (LooperSalty, GPL-3 → code réutilisable): s'appuie sur VIctory (fork LooperSalty/victory). Ce soir: image 3D
  encore noire chez eux aussi (teinte uniforme). Branches utiles: shaders/first-lane-uniform-code-table b00573d,
  shaders/uniform-phi-loop-counter 8996aeb (irradiance SH computes traduits chez eux). Notre stack a déjà
  through_firstlane (lit la source de v_readfirstlane sans test d'uniformité), donc b00573d est surtout couvert;
  8996aeb à mesurer avec pedtool si besoin. anyxboxx/essai: ed28424 (root UAV bornés: déjà chez nous),
  a31baa2 (tampon GPU seul pour plages réservées), b22510c (shadow::find au-delà de 64 voisins).
- Upstream VIctory: #1024 (Dufftwt) copy-back n'écrit que les octets du GPU + marque une page à jour seulement si
  elle correspond: correspond exactement au plantage (« fence list read as -1 »). #1077 root UAVs (3b90165a ne
  tombe plus, 5,5 min sans erreur sur RTX 3080) et #1175: notre stack a son propre équivalent. #180 quad gradients
  compute: déjà présent. #308 whole-wave: le nôtre.
- xcl (GPL-2, idées seules) et Leonida: personne n'affiche encore un monde éclairé (portwatch).

## Fait
- fork fix/copy-back-runs 379f21a (sur d13z6 f668795): copy-back par runs d'octets exacts + vérif page à jour.
  Envoyé au fil Sync pour un test ROOT_UAVS=1 avec copy-back actif.

## Ensuite (si le mode root-UAV tient)
- relever la liste des computes encore refusés entre le visibility buffer et le composite (diag), puis
  ScreenProbes 39f0e9ae (compile driver), pointer chase (memory table déjà en d13z6).
