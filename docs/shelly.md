# Étape 66 : les prises et relais Shelly récents, sans réglage, expliqué

## Le but

Les Shelly récents (Gen2, Gen3 et Gen4 : Plus, Pro, Plug S, Mini…) parlent MQTT, mais pas la
« découverte » de Home Assistant. Jusqu'ici, il fallait un script dans chaque Shelly pour qu'il
apparaisse. Maintenant, il suffit de lui donner l'adresse du broker : le serveur le reconnaît à ses
messages et crée ses entités tout seul. C'est le moyen le plus simple d'avoir les **vrais watts**
d'une prise, ce que la prise Hue ne sait pas faire.

## Mettre en place

Dans la page web du Shelly (son adresse IP dans un navigateur), ou dans l'appli Shelly :

1. **Settings → MQTT :**
   - coche *Enable* ;
   - indique le serveur, `192.168.1.10:1883` (la machine du broker) ;
   - indique l'utilisateur et le mot de passe du broker.
2. Coche **« Generic status update over MQTT »**. C'est ce qui fait publier l'état de chaque sortie.
3. Laisse le préfixe d'usine (`shellyplugsg3-a1b2c3d4e5f6`…), ou choisis-en un sans `/`.
4. Donne-lui un nom (*Device name*, par exemple « Lave-linge ») : le serveur le reprend.

C'est tout. Au démarrage du serveur, ou dès que le Shelly se connecte, il apparaît.

## Les entités créées

Pour une **Shelly Plug S Gen3** nommée « Lave-linge » :

| Entité | Exemple | Remarque |
| --- | --- | --- |
| `switch.shellyplugsg3_a1b2c3d4e5f6` | Lave-linge : allumée | marche, arrêt, inverser ; classe `outlet` pour une prise |
| `sensor.shellyplugsg3_a1b2c3d4e5f6_puissance` | 812,4 W | puissance en direct |
| `sensor.shellyplugsg3_a1b2c3d4e5f6_energie` | 12,346 kWh | tout ce qu'elle a compté : à mettre dans `energy.yaml` |
| `sensor.shellyplugsg3_a1b2c3d4e5f6_tension` | 231,2 V | tension du secteur |

- **Plusieurs sorties :** un relais à plusieurs sorties (Plus 2PM, Pro 4PM) donne une prise par
  sortie (`_1`, `_2`…), nommée « Lave-linge (2) »…
- **Sans mesure :** un relais qui ne mesure pas (Plus 1) n'a que l'interrupteur.
- **Le nom :** il vient de l'appli Shelly. Le serveur le demande une fois (`Shelly.GetDeviceInfo`).
  Sans nom, l'entité s'appelle « Shelly <préfixe> ». On peut aussi la renommer dans `areas.yaml`,
  comme toute entité.
- **Hors ligne :** quand le Shelly se déconnecte du broker, ses entités passent « indisponible »,
  et l'alerte de réparation habituelle arrive au bout d'une heure.

Tout le reste marche comme pour une autre prise :
- tableau de bord, automatisations, Alexa et Assist (« éteins la prise du lave-linge ») ;
- délestage (étape 62) ;
- rapidité mesurée (étape 63), dans la ligne « Réponse MQTT ».

Pour le bilan énergétique :

```yaml
# energy.yaml
consumption:
  - entity: sensor.shellyplugsg3_a1b2c3d4e5f6_energie
    price: 0.2016
```

## Si tu utilisais déjà un script de découverte

Un Shelly qui publie aussi la découverte de Home Assistant (avec un script) apparaîtrait **deux
fois** : une fois par le script, une fois par cette étape. Retire alors le script de ce Shelly.

## Sécurité

- **Les commandes :** le serveur n'envoie des ordres qu'aux Shelly qu'il a déjà vus, et seulement
  `on`, `off` et `toggle` sur leurs sorties.
- **La demande d'état :** il ne l'envoie qu'aux préfixes d'usine (`shelly…`) ou déjà connus. Un
  autre appareil qui publie un message « …/online » n'est jamais contacté.
- **Les limites :** 100 appareils, 4 sorties chacun, 8 Kio par message, 60 caractères par nom (sans
  caractères de contrôle). Une valeur absurde est ignorée.
- **Le broker :** comme pour le reste de MQTT, protège-le par un mot de passe. Un appareil qui peut
  publier sur le broker peut se faire passer pour un Shelly.

## Comment c'est construit

- `crates/ha-mqtt/src/shelly.rs` (nouveau) :
  - `Shellies` lit les sujets `<préfixe>/status/switch:N` et `<préfixe>/online`, ainsi que la
    réponse à notre question, sur `ha-maison/rpc` ;
  - il renvoie des actions (publier), sans toucher au réseau ;
  - quatre tests le vérifient sans broker.
- `crates/ha-mqtt/src/client.rs` : les abonnements en plus, et chaque message passe d'abord par
  `Shellies`, puis par le pont de découverte.

Essayé avec mosquitto et un faux Shelly Plug S Gen3, qui répond comme un vrai :
- il est apparu au démarrage du serveur, sous le nom « Lave-linge » ;
- `turn_on` l'a allumé : 812,4 W, et l'énergie monte ;
- `turn_off` et `toggle` ont suivi ;
- la rapidité mesurée était de 4 ms.

Ce n'est pas un vrai Shelly : si le tien ne réagit pas, envoie-moi ce que montre l'onglet MQTT de sa
page web.

### Concepts Rust utilisés

**`Option<Vec<Action>>` pour « pas pour moi »** : `None` veut dire « ce message n'est pas un
Shelly ». `Some(vec![])` veut dire « c'est un Shelly, rien à publier ». La boucle réseau passe
chaque message aux deux décodeurs : le pont de découverte ignore de lui-même ce qu'il ne suit pas.

**`split('/')` puis un nombre exact de morceaux** : `parse_status_topic` lit trois niveaux, et
refuse un quatrième (`levels.next().is_some()`). Un sujet inattendu ne peut donc pas être pris pour
un Shelly.
