# Étape 89 : un tableau sans doublons et un écran de veille sans cadre, expliqués

## Le but

Trois choses faisaient « beaucoup » sur la tablette, avec la mise en page **tableau** (thème
Maquette) :
- **« Toute la maison »** montrait chaque pièce deux fois : en cartes compactes en haut, puis en
  grandes sections en dessous. Les volets apparaissaient même deux fois de suite : dans la bande
  « Volets », puis dans leur pièce.
- **L'écran de veille** posait l'image de la maison en rectangle aux bords nets sur un fond
  presque noir, avec beaucoup de vide autour.
- **L'agenda** de la vue hub répétait le nom de l'agenda (« demain · Famille ») sur chaque ligne,
  alors que la couleur de la barre le dit déjà.

## Ce qui a changé

### « Toute la maison » : chaque chose une fois

Avec la mise en page tableau, les pièces ne sont plus redessinées en grandes sections sous la
maison. Elles restent en **cartes compactes** en haut (le nom, l'icône, les lumières allumées, la
température) ; un toucher ouvre la pièce **en fenêtre**, avec tous ses appareils, comme avant.

Les **volets** n'apparaissent donc plus qu'une fois, dans la bande « Volets ». Sous la maison, il
ne reste que les appareils rangés dans **aucune** pièce (météo, capteurs à part…), par type.

Les autres mises en page (liste, plan) ne changent pas : les pièces y restent en sections.

Au passage, un changement d'état d'un appareil qui n'a pas de tuile à l'écran (une lampe rangée
dans une pièce, une scène) ne fait plus redessiner toute la page : seules ses cartes (pièce,
volets, maison) suivent. Seul un appareil qui apparaît ou disparaît redessine tout.

### L'écran de veille : la maison se fond dans l'écran

- Derrière, **la même image, très floutée et assombrie**, remplit tout l'écran : ses couleurs vont
  jusqu'aux bords, il n'y a plus de grand vide noir.
- Les bords de l'image de la maison **s'effacent** en douceur dans ce fond : plus de rectangle.
  Une maquette déjà détourée (voir [la vue hub](hub.md)) flotte avec son ombre, sans fondu.
- Un voile assombrit les bords et le côté de l'heure, pour qu'elle se lise sur n'importe quelle
  image ; l'heure, la date et l'état ont une ombre légère.
- Tout le reste est gardé : plus sombre la nuit, décalé de quelques pixels chaque minute, l'heure
  sous la maison sur un téléphone ou une tablette en portrait. Sans dessin de la maison, l'écran
  reste noir avec l'heure au centre, comme avant.

### L'agenda : sans le nom de l'agenda

Dans la vue hub et dans l'agenda du jour, une ligne montre l'heure, le titre et, s'il y en a un,
le lieu (« demain · Mairie »). Le nom de l'agenda reste dans la fenêtre du rendez-vous (« agenda
Famille · TimeTree »).

## Les fichiers

| Fichier | Rôle |
|---|---|
| [web/app.js](../crates/ha-api/web/app.js) | `renderAll` : pas de sections de pièces en mise en page tableau ; `onStateChanged` : tout redessiner seulement pour un appareil ajouté ou supprimé |
| [web/start.js](../crates/ha-api/web/start.js) | `showSaver` : l'image de la maison passe aussi en fond (`--saver-photo`) |
| [web/style.css](../crates/ha-api/web/style.css) | l'écran de veille : le fond flouté, le voile, le fondu des bords |
| [web/hub.js](../crates/ha-api/web/hub.js), [web/agenda.js](../crates/ha-api/web/agenda.js) | les lignes de l'agenda sans le nom de l'agenda |
