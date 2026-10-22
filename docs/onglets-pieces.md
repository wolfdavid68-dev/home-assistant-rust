# Étape 90 : un onglet par pièce, et tout sur un écran, expliqués

## Le but

Avec la mise en page **tableau** (styles Plan et Maquette), l'onglet **Maison** doit garder
l'essentiel, et chaque page doit tenir sur l'écran de la tablette murale, **1920 × 1080**, sans
barre de défilement.

## Ce qui a changé

### Un onglet par pièce

La barre du haut montre, après **Maison** : un onglet par pièce (dans l'ordre de
`config/areas.yaml`), puis **Autres** s'il reste des appareils rangés dans aucune pièce, puis
**Agenda**.

- Un point jaune suit le nom d'une pièce où une lumière est allumée.
- Un onglet de pièce montre tous ses appareils, sur toute la largeur, en colonnes équilibrées
  (jusqu'à cinq) : même une pièce de 19 appareils tient sur l'écran.
- Toucher une carte de pièce, ou le dessin de la maison, ouvre aussi son onglet.
- Le bouton Maison du rail, ou l'onglet Maison, ramène à la maison.
- La vue hub garde la fenêtre de pièce, comme avant.
- S'il y a beaucoup d'onglets, ils prennent une ligne à eux, en haut, et passent à la ligne
  plutôt que de déborder.

### Maison : l'essentiel, sur une page

- À gauche : l'agenda du jour, la semaine, puis **les volets**, un par ligne.
- À droite : le dessin de la maison, sur toute la hauteur.
- En bas : la ligne « Journal d'activité ».
- Les sections par pièce et le cartouche ne s'affichent plus sur un grand écran : les pièces ont
  leurs onglets.

### L'agenda : plus petit, sans défiler

La vue semaine ou jour montre toutes les heures de la journée, **30 pixels par heure** : la
semaine tient sans défiler et la carte ne prend que la hauteur nécessaire.

Sur un téléphone, rien ne change : la page défile comme avant.

## Les fichiers

| Fichier | Rôle |
|---|---|
| [web/house.js](../crates/ha-api/web/house.js) | `renderTabs` : les onglets des pièces ; `openRoomTab` / `closeRoomTab` : la pièce en pleine page ; `--hero-h` : la hauteur de l'en-tête |
| [web/app.js](../crates/ha-api/web/app.js) | `looseStates` : les appareils rangés dans aucune pièce (l'onglet Autres) |
| [web/agenda.js](../crates/ha-api/web/agenda.js) | la semaine à 30 pixels par heure, sans défilement ; passer de l'agenda à une pièce |
| [web/tiles.js](../crates/ha-api/web/tiles.js) | le plan des pièces masqué en mise en page tableau |
| [web/index.html](../crates/ha-api/web/index.html) | `#room-view` : la place de la pièce en pleine page |
| [web/style.css](../crates/ha-api/web/style.css) | la grille de Maison sur un écran, les colonnes des pièces, la carte de l'agenda |
