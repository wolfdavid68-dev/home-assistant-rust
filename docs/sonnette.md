# Étape 82 : la sonnette Ring, expliquée

![Sur la tablette murale : « On sonne : Porte d'entrée, à 08:42 », la photo de la porte en grand, et « Fermer »](images/sonnette.png)

## Le but

Quelqu'un sonne à la porte. Sur la **tablette murale**, la photo de la porte s'affiche aussitôt en
grand, par-dessus tout, même pendant l'[écran de veille](veille.md). Les **enceintes** annoncent
« On sonne à la porte », et les **téléphones** reçoivent une notification. Le
[journal de bord](journal-de-bord.md) note « Porte d'entrée a sonné ».

Alexa continue de marcher comme avant : rien ne change de son côté.

## Pourquoi pas par Alexa

Alexa reçoit bien les coups de sonnette, mais elle ne les transmet à aucun autre logiciel de la
maison. Ring non plus n'a pas d'accès prévu pour cela. La solution éprouvée, utilisée par des
dizaines de milliers de maisons avec Home Assistant, est **ring-mqtt** : un petit programme libre
qui se connecte à ton compte Ring (comme l'appli Ring sur ton téléphone) et transmet, sur le réseau
de la maison :

- le **coup de sonnette** (`binary_sensor.porte_d_entree_sonnette`, « on » quelques secondes) ;
- le **mouvement** devant la porte (`binary_sensor.porte_d_entree_mouvement`) ;
- la **photo** (`camera.porte_d_entree_photo`), prise à chaque coup de sonnette ;
- la **batterie** (`sensor.porte_d_entree_batterie`), et quelques réglages.

Ces noms viennent de ring-mqtt, en anglais (« Ding », « Snapshot »…) : le serveur les traduit.
Le début (`porte_d_entree`) est le nom de ta sonnette dans l'appli Ring.

**Une sonnette sur batterie** ne peut pas prendre de photo quand on le lui demande : elle dort pour
économiser sa batterie. Avec l'abonnement **Ring Protect**, chaque coup de sonnette est enregistré,
et ring-mqtt récupère la photo de cet enregistrement. C'est ce qui permet d'avoir la photo. Elle
arrive quelques secondes après le coup de sonnette : la fenêtre de la tablette montre d'abord la
photo précédente avec « La nouvelle photo arrive… », puis la remplace.

## Mettre en place, sur le mini-PC

### 1. Le facteur MQTT (Mosquitto)

ring-mqtt et le serveur se parlent par [MQTT](etape-4-mqtt.md). Si Mosquitto n'est pas encore
installé :

```bash
sudo apt install mosquitto
sudo nano /etc/mosquitto/conf.d/maison.conf
```

```text
listener 1883 127.0.0.1
allow_anonymous false
password_file /etc/mosquitto/passwd
```

`127.0.0.1` : seul le mini-PC lui-même peut s'y connecter (ring-mqtt et le serveur y tournent tous
les deux). Deux comptes, un pour chacun, avec chacun son mot de passe :

```bash
sudo mosquitto_passwd -c /etc/mosquitto/passwd ha-server
sudo mosquitto_passwd /etc/mosquitto/passwd ring
sudo chown root:mosquitto /etc/mosquitto/passwd && sudo chmod 640 /etc/mosquitto/passwd
sudo systemctl restart mosquitto
```

Puis le serveur : le mot de passe de `ha-server` dans un fichier secret, et trois lignes dans
`/etc/ha-maison/ha-maison.env` :

```bash
sudo nano /etc/ha-maison/secrets/mqtt_password    # le mot de passe, sur une ligne
sudo chgrp ha-maison /etc/ha-maison/secrets/mqtt_password
sudo chmod 640 /etc/ha-maison/secrets/mqtt_password
sudo nano /etc/ha-maison/ha-maison.env
```

```text
HA_MQTT_HOST=127.0.0.1
HA_MQTT_USER=ha-server
HA_MQTT_PASSWORD_FILE=/etc/ha-maison/secrets/mqtt_password
```

```bash
sudo systemctl restart ha-maison
```

### 2. ring-mqtt

Le plus simple est de le faire tourner dans un conteneur Docker, mis à part du reste :

```bash
sudo apt install docker.io
sudo mkdir -p /var/lib/ring-mqtt
sudo nano /var/lib/ring-mqtt/config.json
```

```json
{
  "mqtt_url": "mqtt://ring:LE-MOT-DE-PASSE-DE-ring@127.0.0.1:1883",
  "enable_cameras": true,
  "enable_modes": false,
  "enable_panic": false,
  "livestream_user": "",
  "livestream_pass": "",
  "disarm_code": "",
  "hass_topic": "homeassistant/status",
  "ring_topic": "ring",
  "location_ids": []
}
```

```bash
sudo chmod 600 /var/lib/ring-mqtt/config.json
sudo docker run -d --name ring-mqtt --restart unless-stopped --network host \
  -v /var/lib/ring-mqtt:/data tsightler/ring-mqtt:5.9.3
```

`--restart unless-stopped` : il redémarre tout seul avec le mini-PC. Le numéro de version est
fixé : une mise à jour se fait à la main (voir plus bas), pas en cachette.

### 3. Le relier à ton compte Ring

Sur ton téléphone, ouvre **`http://192.168.1.20:55123`** (l'adresse du mini-PC) : connecte-toi avec
le compte Ring, puis tape le **code reçu par SMS**. C'est fait une seule fois : ring-mqtt garde
ensuite un jeton (dans `/var/lib/ring-mqtt`), et cette page s'arrête d'elle-même.

**De préférence, un compte Ring à part** : dans l'appli Ring, Réglages → Utilisateurs partagés →
invite une adresse à toi réservée à la maison, puis connecte ring-mqtt avec ce compte. Tu pourras
lui retirer l'accès depuis l'appli Ring, sans toucher à ton propre compte.

Quelques secondes plus tard, le journal du serveur dit
`entité MQTT découverte : binary_sensor.porte_d_entree_sonnette`, et la tuile de la photo apparaît
sur le tableau de bord.

### 4. Quand prendre une photo

Sur le tableau de bord, la liste **Porte d'entrée Mode photo** : choisis **Ding** (une photo à chaque
coup de sonnette), ou **Motion + Ding** (aussi quand quelqu'un passe devant la porte, mais la batterie
de la sonnette s'use plus vite).

### 5. L'annonce et la notification

Dans `/etc/ha-maison/automations.yaml` (ou avec l'[éditeur](editeur-automatisations.md)) :

```yaml
- id: sonnette
  alias: On sonne à la porte
  triggers:
    - trigger: state
      entity_id: binary_sensor.porte_d_entree_sonnette
      to: "on"
  actions:
    - action: tts.speak
      target: { entity_id: tts.annonces }
      data:
        media_player_entity_id: media_player.salon
        message: On sonne à la porte.
    - action: notify.send_message
      target: { entity_id: notify.appli }
      data:
        title: Sonnette
        message: On sonne à la porte.
        priority: 5
        tag: sonnette
```

La notification ouvre le tableau de bord, où la tuile montre la photo.

## Sur la tablette

La fenêtre « On sonne » s'ouvre seulement en **mode tablette murale** (sur un téléphone, c'est la
notification qui prévient). Elle réveille l'écran de veille, se met à jour quand la nouvelle photo
arrive, et se ferme d'elle-même au bout de **3 minutes** (ou avec **Fermer**). Un deuxième coup de
sonnette la rouvre.

![La tuile de la photo sur le téléphone : « Porte d'entrée Photo », la photo, « Photo de 08:42 »](images/sonnette-tuile.png)

## Sécurité

- **La photo ne quitte jamais le mini-PC** autrement que vers le tableau de bord d'un compte
  connecté : elle est gardée en **mémoire seulement** (jamais dans la base, ni dans l'historique,
  ni dans les sauvegardes), et servie avec `Cache-Control: no-store` (le navigateur ne la garde pas).
- **Les invités ne la voient pas** : ni la photo, ni la caméra dans la liste des appareils (comme
  les serrures et la présence de la famille).
- **Seulement des images** : un message qui n'est ni un JPEG ni un PNG est refusé ; 2 Mio au plus ;
  32 caméras au plus.
- **MQTT seulement sur le mini-PC** (`127.0.0.1`), un compte par programme, chacun son mot de passe.
- **ring-mqtt** a le droit de voir tes appareils Ring, comme l'appli Ring : d'où le compte partagé,
  retirable à tout moment. Son jeton reste dans `/var/lib/ring-mqtt`, lisible par l'administrateur
  seul. Il ne parle qu'aux serveurs de Ring (en HTTPS) et au Mosquitto du mini-PC.
- **Alexa** n'est pas touchée : rien ne change pour elle.

**Mettre à jour ring-mqtt** (quand Ring change quelque chose, ring-mqtt sort une nouvelle version) :
regarde le numéro sur [la page du projet](https://github.com/tsightler/ring-mqtt/releases), puis :

```bash
sudo docker pull tsightler/ring-mqtt:<nouveau numéro>
sudo docker rm -f ring-mqtt
sudo docker run -d --name ring-mqtt --restart unless-stopped --network host \
  -v /var/lib/ring-mqtt:/data tsightler/ring-mqtt:<nouveau numéro>
```

## Comment c'est construit

| Fichier | Rôle |
| ------- | ---- |
| [crates/ha-mqtt/src/discovery.rs](../crates/ha-mqtt/src/discovery.rs) | Le type `camera` (son sujet `topic`, l'encodage `b64` accepté) ; les noms de ring-mqtt traduits, seulement pour un appareil dont le fabricant est Ring |
| [crates/ha-mqtt/src/bridge.rs](../crates/ha-mqtt/src/bridge.rs) | L'image reçue (binaire, plus grosse qu'un état) est vérifiée et rangée ; la caméra passe à `idle` avec `image_at` (le moment de la photo) |
| [crates/ha-core/src/images.rs](../crates/ha-core/src/images.rs) | La dernière image de chaque caméra, en mémoire |
| [crates/ha-api/src/rest.rs](../crates/ha-api/src/rest.rs) | `GET /api/camera_proxy/{entité}`, comme dans Home Assistant |
| [web/sonnette.js](../crates/ha-api/web/sonnette.js) | La fenêtre « On sonne » sur la tablette, et la photo des tuiles (lue avec le jeton) |
| [crates/ha-logbook/src/describe.rs](../crates/ha-logbook/src/describe.rs) | « Porte d'entrée a sonné » dans le journal de bord |

**Tests** : ce que ring-mqtt publie, rejoué tel quel (la sonnette, puis une photo de 100 Kio, plus
qu'un état ordinaire) ; une image en base64 ; un texte refusé à la place d'une image ; la caméra
retirée avec son image ; l'API (photo servie à la famille, `no-store`, introuvable pour un invité) ;
le journal de bord.

Essayé dans le navigateur avec un vrai Mosquitto et les messages exacts de ring-mqtt (une image de
démonstration, dessinée) : la tablette en écran de veille, le coup de sonnette (la veille s'efface,
la fenêtre s'ouvre avec l'ancienne photo et « La nouvelle photo arrive… »), la nouvelle photo
(elle remplace l'ancienne), Fermer, un deuxième coup de sonnette, et le téléphone (pas de fenêtre,
la tuile avec la photo). Aucune erreur JavaScript.

### Concepts utilisés

**Une image dans MQTT** : un message MQTT peut contenir n'importe quels octets, pas seulement du
texte. ring-mqtt y envoie le fichier JPEG tel quel. Le serveur reconnaît une image à ses premiers
octets (`FF D8 FF` pour un JPEG, la « signature » du format), pas à ce que dit le message.

**`Cache-Control: no-store`** : demande au navigateur de ne rien garder sur le disque de la
tablette. La photo n'existe que le temps de l'afficher.
