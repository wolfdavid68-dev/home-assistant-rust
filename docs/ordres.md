# Étape 46 : des ordres avec réponse, expliqués

![« MQTT ne répond plus depuis 08:48 : l'ordre n'a pas été envoyé »](images/ordre-refuse.png)

## Le but

Toucher une lampe envoyait un ordre sur le bus, puis… rien. Si le pont Hue était débranché, la tuile
clignotait cinq secondes et revenait comme avant, sans un mot. Pire : l'ordre restait en file, et
l'intégration pouvait l'exécuter **en se reconnectant**, deux minutes plus tard. Pour une lampe,
c'est gênant ; pour une porte de garage, ça ne l'est pas du tout.

C'est la deuxième idée reprise de [Moli OS](https://github.com/rudy-io/moli-os) après la
[supervision des intégrations](supervision.md) : « un ordre que personne n'attend plus n'est jamais
exécuté », et on sait s'il a abouti.

## Ce qui change

| Situation | Réponse du serveur | Ce que dit le tableau de bord |
| --------- | ------------------ | ----------------------------- |
| Hue ou Somfy accepte l'ordre | `200`, `"result": "done"` | rien (l'appareil change d'état) |
| Hue ou Somfy le refuse | `502` | « Le pont Hue n'a pas accepté l'ordre. » |
| Pas de réponse en 8 s | `504` | « Hue n'a pas répondu à temps. » |
| L'intégration est en panne (appareils indisponibles) | `503`, **rien n'est envoyé** | « MQTT ne répond plus depuis 08:48 : l'ordre n'a pas été envoyé. » |
| Une intégration qui ne sait pas répondre (MQTT, entrées, scripts…) | `200`, `"result": "sent"` | rien : l'appareil confirmera en changeant d'état |

Et pour toutes les commandes :

- **un ordre périmé n'est jamais exécuté** : chaque ordre porte sa date ; reçu plus de **5 secondes**
  après avoir été donné (pendant une coupure, par exemple), Hue, Somfy et MQTT l'ignorent. La porte du
  garage ne s'ouvre pas toute seule au retour du réseau ;
- les **messages ne citent jamais d'adresse** : le détail de l'erreur (qui peut contenir l'adresse
  du pont) reste dans les logs du serveur.

## Pourquoi seulement Hue et Somfy répondent

Hue et Somfy reçoivent l'ordre par une requête HTTP : le pont ou la box répond « d'accord » ou
« non ». MQTT, lui, dépose un message chez le broker, sans savoir si l'appareil l'a reçu : la seule
confirmation est l'état qui change. Les entrées, scripts et scènes sont dans le serveur lui-même.
Pour eux, l'ordre est simplement envoyé (`"sent"`) ; mais un ordre périmé reste ignoré, et une
intégration en panne est refusée d'office (`503`).

## Sécurité

- Le refus d'office (`503`) évite qu'un ordre attende en file et s'exécute à un moment où plus
  personne ne le veut.
- Les ordres en attente de réponse sont bornés (256 au plus) : une page qui insiste ne remplit pas
  la mémoire.
- Les droits ne changent pas : on vérifie toujours le compte avant tout (`403`), et le mot de passe
  pour une serrure.

## Étape 53 : vérifié avant l'envoi

Avant de partir vers l'appareil, et avant d'être retenu par la [garde](garde.md), un ordre est
**vérifié par le serveur** : l'appareil sait-il le faire, et la valeur est-elle dans sa plage ?
Sinon, il est refusé tout de suite, avec la raison :

> Volet chambre : position entre 0 et 100 (reçu 150).
> Chauffage du salon : consigne entre 7 et 25 (reçu 40).
> Mode maison : choix « Fête » inconnu : Jour, Nuit.

Avant, une valeur hors plage partait quand même : certaines intégrations l'ignoraient sans rien
dire, d'autres la ramenaient dans les bornes (40 °C devenait 25 °C). C'est l'idée de Moli OS :
« vérifier la valeur avant d'envoyer ».

| Appareil | Ce qui est vérifié |
|----------|--------------------|
| Lampe (`light.turn_on`) | luminosité 0 – 255 (ou 0 – 100 %), température de couleur, transition ≤ 300 s |
| Volet, vanne | position et inclinaison 0 – 100 |
| Thermostat, chauffe-eau | consigne entre `min_temp` et `max_temp` ; mode parmi `hvac_modes` ; préréglage parmi `preset_modes` |
| Ventilateur | vitesse 0 – 100 % ; préréglage parmi `preset_modes` |
| Humidificateur | humidité entre `min_humidity` et `max_humidity` |
| Nombre, curseur | valeur entre `min` et `max` |
| Liste | choix parmi `options` |
| Texte | longueur entre `min` et `max` |

Les limites sont celles que l'appareil annonce, avec les mêmes noms que Home Assistant. Un
service que le serveur ne connaît pas n'est pas bloqué : l'intégration garde ses propres
vérifications.

La vérification est faite dans le noyau : elle vaut pour le tableau de bord, l'API, les
automatisations (l'erreur apparaît dans l'historique de l'exécution), Alexa et les ordres validés
depuis la cloche.

## Comment c'est construit

- `crates/ha-core/src/commands.rs` (nouveau) : un numéro par ordre (`Core::call_service_tracked`),
  la réponse de l'intégration (`Core::command_done`), et `is_stale()` (plus de `COMMAND_TTL`, 5 s).
- `Event::CallService` porte maintenant `command` (le numéro, si quelqu'un attend) et `issued_at`
  (sa date).
- `Core::answers_commands("hue")` : les intégrations qui savent répondre ;
  `Core::owner_down(entité)` : l'intégration de l'appareil est-elle en panne ?
- `crates/ha-api/src/rest.rs` (`POST /api/services/…`) : refuse si l'intégration est en panne,
  attend la réponse au plus 8 s (5 s de durée de vie, plus 3 s pour joindre l'appareil) quand elle
  sait répondre.
- `crates/ha-api/web/app.js` : affiche le message en bas de l'écran et arrête le clignotement.
- `crates/ha-core/src/checks.rs` (étape 53) : les vérifications avant l'envoi, par domaine et
  service ; `Core::check_call`, appelé avant chaque envoi et avant qu'un ordre soit retenu.
