# Étape 4 : les vrais appareils avec MQTT, expliqué

## Le principe

MQTT est un « facteur » : un petit serveur, le **broker** (Mosquitto, le plus souvent), reçoit des
messages rangés par **sujet** (`zigbee2mqtt/capteur_salon`, `tasmota/prise/cmnd/POWER`…) et les
distribue à tous ceux qui sont abonnés à ce sujet.

```
 capteur Zigbee ─► Zigbee2MQTT ─┐                         ┌─► ha-server
 prise Tasmota ─────────────────┼──►  broker Mosquitto ◄──┤   (ha-mqtt)
 module ESPHome ────────────────┘                         └─► …
```

Il n'y a rien à déclarer à la main : c'est la **découverte automatique** de Home Assistant.
Chaque appareil publie sa propre description sur `homeassistant/<type>/<objet>/config` :

```json
{"name": "Prise café", "~": "tasmota/prise/",
 "stat_t": "~stat/POWER", "cmd_t": "~cmnd/POWER"}
```

`ha-server` lit ces descriptions et crée les entités tout seul (`switch.prise_cafe`). Ensuite :

1. quand la prise publie `ON` sur `tasmota/prise/stat/POWER`, l'entité passe à `on` ;
2. quand tu appelles `POST /api/services/switch/turn_on`, le noyau publie un événement
   `CallService` sur le bus ; l'intégration MQTT le reçoit et envoie `ON` sur
   `tasmota/prise/cmnd/POWER` ;
3. la prise obéit, publie son nouvel état, et on revient au point 1. L'état ne change donc
   **que quand l'appareil confirme**, comme dans Home Assistant.

Types pris en charge : `sensor`, `binary_sensor`, `switch` et `light` (marche/arrêt, et
luminosité pour les lampes Zigbee2MQTT).

## Les fichiers

| Fichier | Contenu |
|---------|---------|
| [discovery.rs](../crates/ha-mqtt/src/discovery.rs) | Lecture et vérification des descriptions de découverte |
| [template.rs](../crates/ha-mqtt/src/template.rs) | Extraction des valeurs : `{{ value_json.temperature }}` |
| [bridge.rs](../crates/ha-mqtt/src/bridge.rs) | Le pont : messages ⇄ entités, appels de service ⇄ ordres |
| [client.rs](../crates/ha-mqtt/src/client.rs) | Connexion au broker, reconnexion automatique |
| [tests/bridge.rs](../crates/ha-mqtt/tests/bridge.rs) | Tests avec de faux messages Zigbee2MQTT et Tasmota |
| [event.rs](../crates/ha-core/src/event.rs) | Nouvel événement `CallService` |
| [rest.rs](../crates/ha-api/src/rest.rs) | Nouvelle route `POST /api/services/{domaine}/{service}` |

## Les concepts Rust rencontrés

**Ajouter une variante à un `enum`.** L'étape 1 annonçait que le compilateur signalerait chaque
`match` à compléter quand on ajouterait un événement. C'est arrivé : en ajoutant
`Event::CallService`, la compilation a échoué à 4 endroits (enregistreur, WebSocket, logs du
serveur, tests). Chacun a dû décider explicitement quoi faire de ce nouvel événement. Rien n'a
pu être oublié.

**Séparer la logique du réseau.** `Bridge` ne fait aucune entrée/sortie : il reçoit un message et
renvoie une liste d'`Action` (`Subscribe`, `Publish`…). La boucle réseau exécute ces actions.
Résultat : toute la logique se teste sans broker (voir `tests/bridge.rs`), vite et de façon fiable.

**`HashMap` et compteur de références maison.** Plusieurs entités Zigbee2MQTT partagent le même
sujet (température, humidité et batterie arrivent dans un seul message). `subscriptions` compte
combien d'entités utilisent chaque sujet : on s'abonne au premier, on se désabonne après le dernier.

**Analyse de texte à la main.** `Template::parse` découpe `value_json['DS18B20'].Temperature`
avec `strip_prefix`, `find` et des tranches (`&texte[début..fin]`). Les motifs sur octets
(`[b'\'', .., b'\'']`) reconnaissent « commence et finit par une apostrophe ».

**`Option` comme filet de sécurité.** L'opérateur `?` marche aussi sur `Option` : dès qu'une étape
ne trouve rien (champ absent, JSON invalide), la fonction renvoie `None` sans planter.

**Reconnexion avec attente croissante.** En cas de coupure, on réessaie après 1 s, puis 2 s, 4 s…
jusqu'à 60 s : un broker redémarré est retrouvé vite, sans le bombarder de connexions.

**« En ligne » à chaque connexion.** Comme Home Assistant, le serveur publie `online` sur
`homeassistant/status` juste après s'être abonné. Les ponts comme ring-mqtt n'envoient leurs
descriptions qu'à leur propre démarrage, et le broker ne les garde pas : sans ce message, un
serveur redémarré après eux ne voyait plus rien de la sonnette (ni sonnerie, ni batterie) avant le
prochain redémarrage de ring-mqtt. ring-mqtt répond en renvoyant tout, 15 s plus tard.

**`std::future::pending()`.** Sans MQTT, la simulation tourne ; avec MQTT, il n'y a pas de
simulation et la branche correspondante du `select!` attend indéfiniment. Seul Ctrl+C arrête alors
le serveur.

## Choix de sécurité de cette étape

- **Tout ce qui vient de MQTT est vérifié.** Descriptions limitées à 16 Kio et messages d'état à
  64 Kio, textes limités et sans caractères de contrôle, types d'entités sur liste blanche. Une
  description incomprise est **refusée** (avec un message dans les logs), jamais devinée.
- **Pas d'exécution de code.** Home Assistant accepte de vrais programmes Jinja2 dans les
  descriptions. Ici, seuls les chemins simples (`value_json.a['b'][0]`) sont acceptés : un
  message reçu ne peut que lire un champ. Seul filtre permis, à la fin : `| default("…")` (un
  texte entre guillemets ou un nombre), utilisé par ring-mqtt pour la batterie de la sonnette.
- **Sujets contrôlés.** Pas de jokers (`+`, `#`) dans les sujets d'état ou de commande, et un
  appareil ne peut pas nous faire publier sous `homeassistant/…` (sinon il pourrait créer de
  fausses entités).
- **Nombre d'entités limité** à 1 000 : un appareil défaillant ne peut pas remplir la mémoire.
- **Commandes vérifiées.** Seuls `turn_on`, `turn_off` et `toggle` existent, uniquement pour les
  entités MQTT du bon domaine. La luminosité doit être entre 0 et 255.
- **Identifiants du broker.** Le mot de passe peut être lu depuis un fichier
  (`HA_MQTT_PASSWORD_FILE`), il est effacé de la mémoire après usage et n'apparaît jamais dans les
  logs (vérifié). Un avertissement s'affiche si le broker n'a pas d'identifiants.
- **Pas encore de TLS (chiffrement).** La partie TLS de la bibliothèque `rumqttc` dépend
  aujourd'hui de crates touchées par des failles connues ou abandonnées. `cargo deny` les a
  bloquées, et c'est voulu. On l'activera quand `rumqttc` sera à jour (Dependabot nous
  préviendra). En attendant, le broker doit rester **sur la même machine ou sur ton réseau local**,
  jamais joignable depuis Internet. Un avertissement s'affiche si le broker n'est pas en `localhost`.

## Mettre en place

**1. Un broker Mosquitto**, avec un compte (sous Windows : installeur sur
[mosquitto.org](https://mosquitto.org/download/) ; sur un Raspberry Pi : `sudo apt install mosquitto`).
Dans `mosquitto.conf` :

```
listener 1883
allow_anonymous false
password_file C:\mosquitto\passwd
```

puis créer le compte : `mosquitto_passwd -c C:\mosquitto\passwd ha`.

**2. Activer la découverte sur tes appareils :**

- **Zigbee2MQTT** : dans `configuration.yaml`, `homeassistant: enabled: true` ;
- **Tasmota** : dans la console, `SetOption19 1` ;
- **ESPHome** : composant `mqtt:` (la découverte est active par défaut) ;
- **Shelly** récents (Gen2 et plus) : rien à faire, ils sont reconnus tout seuls depuis
  l'[étape 66](shelly.md) (coche « Generic status update over MQTT ») ; les anciens Shelly (Gen1)
  passent par un script de découverte.

**3. Lancer le serveur** (PowerShell) :

```powershell
Set-Content -NoNewline C:\Users\moi\domotique\mqtt-pass "le-mot-de-passe-du-broker"
$env:HA_MQTT_HOST = "127.0.0.1"          # ou l'adresse du Raspberry Pi
$env:HA_MQTT_USER = "ha"
$env:HA_MQTT_PASSWORD_FILE = "C:\Users\moi\domotique\mqtt-pass"
cargo run -p ha-server
```

Les entités apparaissent dans les logs (`entité MQTT découverte : …`) et dans `GET /api/states`.

**4. Piloter un appareil :**

```powershell
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:8123/api/services/switch/turn_on `
     -Headers $h -Body '{"entity_id":"switch.prise_cafe"}'

Invoke-RestMethod -Method Post -Uri http://127.0.0.1:8123/api/services/light/turn_on `
     -Headers $h -Body '{"entity_id":"light.lampe_salon","brightness":128}'
```

(`$h` contient le jeton : voir le [guide de l'étape 3](etape-3-auth-api.md#essayer).)

Sans `HA_MQTT_HOST`, le serveur démarre comme avant avec ses appareils simulés.

## Pas encore pris en charge

TLS (voir plus haut), disponibilité des appareils (`availability`), autres types d'entités
(volets, thermostats…), filtres Jinja (`| float`…). Ce sont des ajouts possibles plus tard.

```bash
cargo test
```
