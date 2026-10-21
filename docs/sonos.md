# Étape 73 : les enceintes Sonos et les annonces vocales, expliquées

## Le but

Tes enceintes Sonos apparaissent dans le tableau de bord. Pour chacune, on voit ce qu'elle joue, et on
peut lancer, mettre en pause, passer au morceau suivant et régler le volume.

Surtout, **la maison peut parler** : « Le linge est fini », « Vigilance orange orages demain »,
« Le repas est prêt ». La phrase est lue par une voix française qui tourne **sur ton PC** (Piper, la
voix locale de Home Assistant) : rien ne part sur Internet.

| Avant | Pendant l'annonce | Après |
| --- | --- | --- |
| Salon et cuisine jouent *Aline*, morceau 3, à 1 min 23, volumes 22 et 35 % | ding-dong, puis « Le repas est prêt », à 50 % | *Aline* reprend à 1 min 23, volumes 22 et 35 % |

Tout passe par l'API locale des Sonos (UPnP, port 1400) : ni compte Sonos ni cloud.

**Et Alexa ?** Un Echo ne sait pas jouer un son qu'on lui envoie sans passer par le cloud d'Amazon
(il faudrait une API non officielle, avec les identifiants du compte Amazon, qui casse souvent). Les
annonces se font donc sur les Sonos. Une Sonos avec Alexa intégrée (One, Era 100, Beam…) marche comme
les autres.

## Mettre en place

### 1. Les enceintes

Ajoute `HA_SONOS` aux réglages :

| Réglage | Exemple | Rôle |
| --- | --- | --- |
| `HA_SONOS` | `auto` | les chercher sur le réseau ; sinon, l'adresse d'**une** enceinte (`192.168.1.30`) : elle connaît les autres |
| `HA_SONOS_AUDIO` | `192.168.1.20` | l'adresse de ce PC vue par les enceintes (devinée si absente ; obligatoire avec Docker) ; `:9000` pour changer le port 8124 |
| `HA_PIPER` | `C:\ha-maison\piper\piper.exe` ou `tcp://piper:10200` | la voix : le programme Piper, ou le service Wyoming |
| `HA_PIPER_VOIX` | `fr_FR-siwis-medium` | le fichier `.onnx` de la voix (programme), ou son nom (service) ; par défaut `fr_FR-siwis-medium` |

Pour voir ce que le serveur trouve :

```bash
ha-server sonos
```

```text
recherche des enceintes Sonos sur le réseau local (3 s)…
- groupe : Salon (media_player.salon, 192.168.1.30:1400) + Cuisine (media_player.cuisine, 192.168.1.31:1400)
- Chambre (media_player.chambre, 192.168.1.32:1400)
3 enceinte(s). Pour le serveur : HA_SONOS=192.168.1.30
```

Sans voix, les annonces se limitent au **carillon** (« ding-dong ») : c'est déjà une sonnette.

### 2. La voix, sous Windows

1. Télécharge **Piper** : `piper_windows_amd64.zip` sur
   <https://github.com/rhasspy/piper/releases/tag/2023.11.14-2>. Décompresse-le dans le dossier du
   serveur : tu obtiens un dossier `piper` avec `piper.exe` dedans.
2. Télécharge **la voix**, les deux fichiers `fr_FR-siwis-medium.onnx` (63 Mo) et
   `fr_FR-siwis-medium.onnx.json`, sur
   <https://huggingface.co/rhasspy/piper-voices/tree/v1.0.0/fr/fr_FR/siwis/medium>. Range-les dans
   le dossier `piper`, à côté de `piper.exe`.
3. Dans `demarrer.bat` :
   ```bat
   set HA_SONOS=auto
   set HA_PIPER=%~dp0piper\piper.exe
   ```
4. Essaie la voix, sans les enceintes :
   ```bat
   ha-server.exe voix "Bonjour, ceci est un essai."
   ```
   Le fichier `essai-voix.wav` apparaît : double-clique pour l'écouter.
5. Au premier démarrage du serveur, Windows demande l'accès au réseau pour `ha-server.exe` : coche
   **réseaux privés** seulement. Les enceintes viennent chercher l'annonce sur le port 8124.

D'autres voix françaises existent (`fr_FR-upmc-medium`, `fr_FR-tom-medium`…) : même dossier, et
`set HA_PIPER_VOIX=%~dp0piper\fr_FR-tom-medium.onnx`.

### 3. La voix, avec Docker

La voix tourne dans son propre conteneur, `wyoming-piper`. Dans `compose.yaml`, enlève les `#` devant le
service `piper`, devant le port `8124` et devant les lignes Sonos :

```yaml
      HA_SONOS: 192.168.1.30        # une enceinte : la recherche ne sort pas d'un conteneur
      HA_SONOS_AUDIO: 192.168.1.20  # l'adresse de ce PC
      HA_PIPER: tcp://piper:10200
```

Puis `docker compose up -d`. Au premier démarrage, `piper` télécharge la voix (une seule fois), puis
tout reste sur le PC. Pour l'essayer : `docker compose run --rm ha-server voix "Bonjour"`.

## Dans le tableau de bord

La section **Musique et annonces** :
- **Chaque enceinte** :
  - ce qu'elle joue (« Lecture · Aline — Christophe · avec Cuisine »), la radio ou la TV ;
  - morceau précédent, lecture ou pause, morceau suivant ;
  - le volume, et couper le son.
- **Annonces** : écris la phrase, choisis l'enceinte (ou « Partout ») et le volume, puis **Annoncer**.
  **Carillon seul** fait juste « ding-dong ».

Dans un groupe, une commande de lecture va à l'enceinte qui coordonne. Une annonce sur une enceinte
groupée est donc entendue par **tout le groupe**, comme la musique. Le volume, lui, se règle enceinte
par enceinte.

## Dans les automatisations

```yaml
- id: linge_annonce
  alias: Le linge est fini
  triggers:
    - trigger: event
      event_type: appareil_termine
      event_data: { entity_id: sensor.lave_linge }
  actions:
    - action: tts.speak
      target: { entity_id: tts.annonces }
      data:
        media_player_entity_id: [media_player.salon, media_player.cuisine]
        message: "Le linge est fini, en {{ trigger.event.data.duree_minutes }} minutes."
        volume: 45          # % ; sans lui, le volume actuel
        carillon: true      # par défaut
```

L'éditeur propose « Faire une annonce » sur `tts.annonces`, et les commandes des enceintes
(`media_player.media_pause`, `volume_set`…) comme pour les autres appareils.

| Entité | État | Attributs utiles |
| --- | --- | --- |
| `media_player.<pièce>` | `playing`, `paused`, `idle`, `unavailable` | `media_title`, `media_artist`, `media_channel` (radio), `volume_level` (0 à 1), `is_volume_muted`, `group_members` |
| `tts.annonces` | l'heure de la dernière annonce | `dernier_message`, `voix`, `enceintes` |

Le journal de bord note « Salon joue », « Chambre s'est arrêtée » et « Annonces : « Le repas est
prêt » », avec qui l'a demandé.

## Ce qui se passe pendant une annonce

1. Le texte devient un son, sur place (Piper), précédé du carillon.
2. Le serveur **note** ce que joue le groupe : la file d'attente et l'endroit du morceau, ou la radio,
   ou la TV ; s'il jouait ; le volume et la sourdine de chaque enceinte.
3. Il met le volume de l'annonce, enlève la sourdine, et donne à l'enceinte l'adresse du son.
4. L'enceinte télécharge le son et le joue (« Annonce » dans l'appli Sonos).
5. À la fin, tout est **remis comme avant** : la même musique au même endroit, les mêmes volumes, et la
   lecture si ça jouait.

Deux annonces sur le même groupe passent l'une après l'autre. Si quelqu'un lance autre chose pendant
l'annonce, l'ancienne musique n'est pas remise par-dessus : seulement les volumes.

**Limite :** la musique lancée **depuis le téléphone** (Spotify Connect, AirPlay) ne peut pas être
reprise, comme dans Home Assistant. Elle s'arrête après l'annonce : relance-la depuis le téléphone.
La file d'attente Sonos, les radios et la TV reprennent toutes seules.

## Sécurité

- **Le serveur des annonces** (port 8124) est séparé du tableau de bord, et ne sait faire qu'une chose :
  - il ne sert que les annonces en cours, chacune sous un nom aléatoire de 128 bits, oubliée à la fin ou
    après 10 minutes ;
  - il ne répond qu'aux adresses des enceintes Sonos connues : pour toute autre machine, rien n'existe ;
  - il ne donne accès à rien d'autre : ni le tableau de bord, ni l'API.
- **Les enceintes :**
  - seules les adresses du réseau local sont acceptées, sur le port des Sonos (1400) : une réponse
    trafiquée ne peut pas faire appeler une machine sur Internet ;
  - réponses limitées à 512 Kio, 3 s pour répondre, 64 enceintes au plus.
- **La voix** tourne sur le PC (ou dans un conteneur à côté) : la phrase ne part jamais sur Internet.
  Elle tient sur une ligne, sans caractère de contrôle, 500 caractères au plus, et doit être prête en 30 s.
- **Les invités** peuvent mettre la musique et régler le volume, mais ne voient pas les annonces : un
  invité ne fait pas parler toutes les enceintes de la maison. Une enceinte listée dans une pièce
  protégée (étape 48) suit la garde comme les autres appareils.
- Les valeurs sont vérifiées avant l'envoi : volume de 0 à 1 (`volume_set`), de 0 à 100 % pour une
  annonce.

## Comment c'est construit

- **`crates/ha-tts` (nouveau)** : la voix.
  - `Tts::from_settings` lit `HA_PIPER` et `HA_PIPER_VOIX` ;
  - `program` lance `piper --output_raw` ;
  - `wyoming` parle au service, avec le protocole de Home Assistant ;
  - `audio` contient le carillon (calculé, aucun fichier), la conversion au format du CD
    (44 100 Hz, stéréo) et l'écriture du WAV.
- **`crates/ha-sonos` (nouveau)** : les enceintes.
  - `api` envoie les requêtes UPnP (SOAP) et `xml` en est le petit lecteur ;
  - `topology` lit les groupes, `discovery` fait la recherche SSDP, `player` traduit l'état d'une
    enceinte ;
  - `announce` note l'état, annonce, puis remet tout comme avant ;
  - `clips` est le serveur des annonces.
- **`crates/ha-server`** : les réglages, `ha-server sonos` et `ha-server voix`.
- **`crates/ha-core`** : la vérification du volume et des annonces.
- **`crates/ha-api`** : les tuiles, les droits des invités, l'éditeur.
- **`crates/ha-logbook`** : les phrases du journal.

Essayé :
- **la voix**, avec le vrai Piper, en programme et en service Wyoming : 7 s de son préparées en 1,1 s ;
- **de bout en bout**, avec le vrai serveur et de fausses enceintes Sonos qui répondent comme les
  vraies :
  - le tableau de bord en clair, en sombre et sur téléphone ;
  - une annonce sur la cuisine, groupée avec le salon : son téléchargé par l'enceinte, puis le morceau 3
    repris à 1 min 23, et les volumes remis à 22 et 35 % ;
- **un test automatique** (`crates/ha-sonos/tests/fausses_enceintes.rs`) refait tout cela à chaque
  changement, ainsi que la pause d'une radio (refusée par Sonos, donc arrêt) et une enceinte inconnue.

Ce ne sont pas tes vraies enceintes : si une annonce ne passe pas, envoie-moi ce que dit le journal
(`RUST_LOG=ha_sonos=debug` pour le détail).

### Concepts Rust utilisés

**Un verrou asynchrone par groupe** : `tokio::sync::Mutex` se garde à travers un `.await`, ce que ne
permet pas `std::sync::Mutex`. Chaque groupe a le sien (`HashMap<String, Arc<Mutex<()>>>`) : deux
annonces sur le salon passent l'une après l'autre, pendant qu'une annonce dans la chambre se joue en
même temps.

**`oneshot` pour répondre tôt** : l'ordre `tts.speak` répond dès que l'annonce **démarre** sur chaque
groupe (`oneshot::Sender`), sans attendre sa fin. La tâche continue ensuite seule : attendre la fin du
son, puis tout remettre.

**`tokio::join!`** : l'état de lecture, le morceau et ce qui est chargé sont demandés **en même temps** à
l'enceinte, comme le volume et la sourdine : trois allers-retours réseau coûtent le temps d'un seul.

**L'interpolation linéaire** : pour passer de 22 050 à 44 100 échantillons par seconde, chaque nouvel
échantillon est pris entre ses deux voisins (`a + (b − a) × fraction`). Pour une voix, la différence
avec une méthode savante ne s'entend pas.
