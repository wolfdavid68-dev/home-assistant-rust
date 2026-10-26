# Étape 100 : une couleur par sujet, expliqué

## Le but

Sur l'écran, une température, une puissance, une machine et une alerte se ressemblaient : la même
icône grise. Pour savoir de quoi parle une valeur, il fallait lire son nom.

Maintenant, chaque **famille** a sa teinte, la même partout :

| Famille | Teinte | Exemples |
| --- | --- | --- |
| météo | bleu | température, humidité, pluie, vent, pression, lumière |
| énergie | jaune | puissance (W, VA), consommation (kWh), coût (€), Linky, Tempo |
| machines | vert | lave-linge, sèche-linge, lave-vaisselle, fin de cycle |
| alertes | ambre | fumée, fuite d'eau, gaz, problème, appareil injoignable |

## Où on la voit

- **Les tuiles de capteurs** : l'icône prend un fond léger de sa teinte. Une alerte en cours
  (fumée détectée, fuite) a l'icône pleine, en ambre.
- **Les cartes en verre** (étape 99) : un point de couleur devant la source, un reflet teinté et
  les pastilles d'une liste.
- **La carte de la machine** sur le dessin de la maison (étape 86) : verte pendant le cycle (elle
  était de la couleur d'accent). En pause et à la fin, elle garde ses couleurs (jaune, puis vert
  « fini »).

Les lampes, prises, volets et lecteurs gardent leurs couleurs à eux : seuls les capteurs ont une
famille.

## Comment la famille est choisie

Une seule fonction, `familyOf` (dans `utils.js`), sert aux tuiles et aux cartes. Elle regarde, dans
l'ordre :

1. le **type de capteur** donné par l'intégration (`device_class` : `temperature`, `power`,
   `smoke`…) ;
2. l'**unité** (°C, W, kWh, VA, €, mm, km/h…) ;
3. le **nom** ou la phrase (« Lave-linge », « Conso », « Il fait… »).

Les alertes passent en premier : un capteur de fumée reste une alerte même si son nom parle d'une
pièce. Rien ne trouve de famille ? La tuile reste comme avant.

## Ce qui ne change pas

- Les teintes sont des jetons de la feuille de style (`--tint-meteo`, `--tint-energie`,
  `--tint-machines`, `--tint-alertes`), avec une version pour le thème sombre ; le thème « plan »
  garde ses traits à l'encre.
- La couleur n'est jamais la seule information : le nom, l'icône et l'unité restent affichés.

L'idée vient de l'assistant vocal Cornelius (une couleur par sujet) ; le code est écrit pour ce
projet, rien n'en est copié.
