# Étape 91 : les favoris Sonos d'un toucher, expliqués

## Le but

Lancer ses playlists (YouTube Music, Spotify, une radio…) sur une enceinte Sonos en touchant un
bouton du tableau de bord, sans ouvrir l'application Sonos.

## Ce qui a changé

### Les favoris de la maison Sonos

Chaque minute, le serveur lit les **favoris Sonos** (le dossier « Mes Sonos »
de l'application) sur une des enceintes. Chaque enceinte les montre dans l'attribut
`source_list` : la liste des favoris qu'on peut lancer.

- Les raccourcis que Sonos ne sait pas rejouer de l'extérieur (certaines stations
  « Sonos Radio ») sont ignorés.
- Une radio jouée depuis un favori s'affiche dans `source`.

### Lancer un favori

Le nouveau service `media_player.select_source`, avec `source` = le nom du favori, lance le
favori sur l'enceinte (ou sur tout son groupe, si elle est groupée) :

- une radio est jouée tout de suite ;
- une playlist ou un album remplace la file d'attente, puis la lecture démarre au début.

Un nom de favori inconnu est refusé avant d'être envoyé, avec le message
« favori « … » inconnu ».

Le service se règle aussi dans l'éditeur d'automatisations : action **Lancer un favori**, puis le
nom du favori. Par exemple, une playlist douce chaque soir à 19 h.

### Dans le tableau de bord

La carte de l'enceinte montre un bouton par favori, sous le volume. Le favori qui joue est mis en
avant. Au-delà de huit favoris, les boutons laissent place à une liste « Lancer un favori… ».

## Ajouter ses playlists YouTube Music

Le tableau de bord montre les favoris enregistrés dans l'application Sonos :

1. Ouvrir l'application **Sonos** sur le téléphone.
2. Aller dans **YouTube Music** (il doit être ajouté dans Paramètres, Services et voix).
3. Ouvrir une playlist, toucher **…**, puis **Ajouter aux favoris Sonos**.

Le bouton apparaît sur la tablette moins d'une minute plus tard. Pour le retirer, on retire le
favori dans l'application Sonos.

## Les fichiers

| Fichier | Rôle |
|---|---|
| [ha-sonos/src/favorites.rs](../crates/ha-sonos/src/favorites.rs) | `parse` : lire les favoris envoyés par l'enceinte ; `describe` : les attributs `source_list` et `source` |
| [ha-sonos/src/api.rs](../crates/ha-sonos/src/api.rs) | `favorites` : demander les favoris ; `clear_queue` et `add_to_queue` : remplir la file d'attente |
| [ha-sonos/src/lib.rs](../crates/ha-sonos/src/lib.rs) | `refresh_favorites` : garder la liste à jour ; `select_source` : lancer un favori sur le groupe |
| [ha-core/src/checks.rs](../crates/ha-core/src/checks.rs) | refuser un favori inconnu avant l'envoi |
| [web/tiles.js](../crates/ha-api/web/tiles.js) | `mediaFavorites` : les boutons des favoris sur la carte de l'enceinte |
| [web/editor.js](../crates/ha-api/web/editor.js) | l'action « Lancer un favori » dans l'éditeur d'automatisations |
| [web/style.css](../crates/ha-api/web/style.css) | les boutons des favoris |
| [ha-sonos/tests/fausses_enceintes.rs](../crates/ha-sonos/tests/fausses_enceintes.rs) | les favoris lus et lancés sur de fausses enceintes |
