# Étape 92 : le lecteur de musique, expliqué

## Le but

Une vraie page de lecteur, belle et simple, pour les enceintes Sonos :

- la pochette en grand, avec le titre et l'artiste ;
- une barre de progression qu'on touche pour avancer ;
- les boutons précédent, lecture et suivant, le volume, l'aléatoire et la répétition ;
- les favoris Sonos (playlists YouTube Music, radios…) en grille, avec leur pochette.

## Ce qui a changé

### La page Musique

Elle s'ouvre de trois façons :

- le bouton **Musique** du dock (sur téléphone) ;
- l'onglet **Musique** (avec la mise en page tableau) ;
- la ligne « en cours » sur la carte de l'enceinte : une petite pochette, le titre et une flèche.

Le bouton et l'onglet n'apparaissent que s'il y a au moins une enceinte.

Sur la page :

- **Les enceintes** : s'il y en a plusieurs, des pastilles pour choisir. Celle qui joue a trois
  petites barres qui dansent. Le choix est retenu sur l'appareil.
- **En cours** : la pochette en grand. Son reflet flou colore le fond de la page.
- **La barre de progression** : le temps écoulé à gauche, le temps restant à droite. Toucher la
  barre ou la glisser fait avancer le morceau. Pour une radio, la barre reste pleine et affiche
  « En direct ».
- **Les boutons** :
  - aléatoire et répétition (tout, puis un seul morceau), avec un point sous le bouton quand ils
    sont actifs ;
  - précédent, lecture ou pause (le gros bouton), et suivant ;
  - couper le son, et le volume.
- **Les favoris** : une grille de pochettes. Un toucher lance le favori, et celui qui joue est
  entouré. S'il n'y a pas encore de favori, la carte explique comment en ajouter (voir plus bas).

Les mises en page :

- sur téléphone, tout est en colonne ;
- sur tablette, la pochette passe à gauche ;
- sur le grand écran (1920 × 1080), les favoris passent à côté du lecteur et la pochette grandit.

Le thème plan garde des angles nets, sans reflet flou.

### Côté serveur

Chaque enceinte donne de nouveaux attributs :

| Attribut | Contenu |
|---|---|
| `entity_picture` | la pochette du morceau, servie par notre serveur (`/api/media_cover/<clé>`) |
| `media_duration`, `media_position`, `media_position_updated_at` | la durée, la position et le moment où elle a été lue ; le tableau de bord fait avancer la barre entre deux lectures |
| `shuffle`, `repeat` (`off`, `all`, `one`) | l'aléatoire et la répétition, seulement quand la file d'attente joue |
| `source_pictures` | la pochette de chaque favori |

Et de nouveaux services, envoyés au coordinateur du groupe :

| Service | Champ |
|---|---|
| `media_player.media_seek` | `seek_position`, en secondes |
| `media_player.shuffle_set` | `shuffle` : `true` ou `false` |
| `media_player.repeat_set` | `repeat` : `off`, `all` ou `one` |

### Les pochettes, en sécurité

Le tableau de bord n'affiche que des images de notre propre serveur.

1. Le serveur télécharge lui-même la pochette, seulement depuis :
   - une enceinte connue (port 1400) ;
   - ou une adresse `https://` sur Internet.

   Il ne la prend jamais ailleurs sur le réseau local.
2. L'image doit être un vrai JPEG ou PNG de 1 Mo au plus.
3. Elle est gardée en mémoire seulement : 128 pochettes au plus, la plus ancienne part la
   première.
4. Si un téléchargement échoue, on ne réessaie qu'au bout de 10 minutes.
5. `/api/media_cover/<clé>` demande d'être connecté, comme le reste de l'API.

## Ajouter ses playlists YouTube Music

Elles viennent des favoris de l'application Sonos ([étape 91](favoris-sonos.md)) :

1. Ouvrir l'application **Sonos** sur le téléphone.
2. Aller dans **YouTube Music**.
3. Ouvrir une playlist, toucher **…**, puis **Ajouter aux favoris Sonos**.

Elle apparaît dans la grille, avec sa pochette, moins d'une minute plus tard.

## Les fichiers

| Fichier | Rôle |
|---|---|
| [ha-core/src/covers.rs](../crates/ha-core/src/covers.rs) | les pochettes gardées en mémoire |
| [ha-sonos/src/art.rs](../crates/ha-sonos/src/art.rs) | d'où une pochette peut venir, et son téléchargement |
| [ha-sonos/src/player.rs](../crates/ha-sonos/src/player.rs) | la position, la durée, l'aléatoire et la répétition lus sur l'enceinte |
| [ha-sonos/src/api.rs](../crates/ha-sonos/src/api.rs) | `seek` et le mode de lecture envoyés à l'enceinte |
| [ha-sonos/src/lib.rs](../crates/ha-sonos/src/lib.rs) | les nouveaux attributs et services |
| [ha-core/src/checks.rs](../crates/ha-core/src/checks.rs) | vérifier les nouveaux services avant l'envoi |
| [ha-api/src/rest.rs](../crates/ha-api/src/rest.rs) | `GET /api/media_cover/{clé}` |
| [web/musique.js](../crates/ha-api/web/musique.js) | la page Musique |
| [web/tiles.js](../crates/ha-api/web/tiles.js) | la ligne « en cours » sur la carte de l'enceinte |
| [web/house.js](../crates/ha-api/web/house.js), [web/agenda.js](../crates/ha-api/web/agenda.js), [web/hub.js](../crates/ha-api/web/hub.js) | l'onglet Musique, et les autres pages qui la referment |
| [web/style.css](../crates/ha-api/web/style.css) | le style du lecteur, et le dock à sept boutons |
