# Étape 70 : la protection solaire des volets, expliquée

## Le but

En été, le soleil qui tape sur une baie vitrée chauffe la pièce bien plus vite que n'importe quel
radiateur. Ici, la protection baisse automatiquement les volets de la façade concernée. Il faut
trois conditions :
- le soleil est **sur la façade** ;
- il fait **chaud** ;
- il y a **du soleil**.

Quand le soleil est passé, chaque volet **remonte là où il était**. Si tu bouges un volet à la main,
la protection le laisse tranquille un moment : c'est toi qui décides.

Exemple pour une baie plein sud, un jour à 28 °C :

| Heure | Le soleil | Le volet |
| --- | --- | --- |
| 9 h | à l'est, la façade sud est encore à l'ombre | ouvert |
| 10 h 30 | passe devant la façade | descend à 25 % |
| 13 h | quelqu'un le remonte à la main | laissé tranquille 2 h |
| 15 h | toujours sur la façade | redescend à 25 % |
| 18 h 30 | passé à l'ouest | remonte à sa position d'avant |

## Mettre en place

Copie `protection-solaire.example.yaml` en `protection-solaire.yaml` (avec Docker :
`config/protection-solaire.yaml`), adapte-le, puis vérifie :

```bash
ha-server protection-solaire check
```

```yaml
# protection-solaire.yaml
temperature: sensor.meteo_temperature  # la chaleur qui décide (dehors, ou une pièce)
seuil: 24                              # °C : protéger au-dessus
ecart: 2                               # °C : arrêter sous 22 °C
meteo: weather.maison                  # seulement par soleil ou temps voilé
pause_manuelle: 2                      # heures sans toucher un volet bougé à la main
volets:
  - entity_id: cover.salon
    orientation: sud                   # où regarde la façade
    position: 25                       # où le descendre (0 fermé, 100 ouvert)
  - entity_id: cover.chambre
    orientation: ouest
    ouverture: 70                      # le soleil compte à ±70° de la façade
    hauteur_min: 15                    # et au moins à 15° au-dessus de l'horizon
    position: 0
```

Le serveur doit connaître la position de la maison (`HA_LATITUDE` et `HA_LONGITUDE`, comme pour
le soleil de l'étape 8) : c'est ce qui lui dit où est le soleil à chaque instant.

| Réglage | Par défaut | Rôle |
| --- | --- | --- |
| `temperature` | aucune | un capteur en °C ; sans lui, la chaleur ne compte pas |
| `seuil` | 24 | protéger au-dessus de cette température |
| `ecart` | 2 | arrêter sous seuil − écart : pas d'aller-retour à chaque dixième de degré |
| `meteo` | aucune | `weather.maison` : seulement par temps ensoleillé ou voilé |
| `pause_manuelle` | 2 | heures sans toucher un volet bougé à la main (0 : jamais de pause) |
| `orientation` | (obligatoire) | `nord`, `nord-est`, `est`, `sud-est`, `sud`, `sud-ouest`, `ouest`, `nord-ouest`, ou des degrés (180 = sud) |
| `ouverture` | 90 | le soleil compte s'il est à moins de tant de degrés de la façade (10 à 90) |
| `hauteur_min` | 10 | hauteur minimale du soleil : un soleil rasant est souvent caché par les voisins |
| `position` | 20 | où le descendre, de 0 (fermé) à 90 |

**Trouver l'orientation d'une façade :** regarde par la fenêtre avec la boussole du téléphone.
La direction que tu vois est celle de la façade. Si elle ne tombe pas pile sur un point
cardinal, donne des degrés : 200 pour « un peu après le sud ».

## L'interrupteur `switch.protection_solaire`

- **Armée** au premier démarrage, puis comme tu l'as laissée.
- **La désarmer** remonte tout de suite les volets qu'elle avait baissés.
- **Sa tuile dit ce qu'elle fait :**
  - « Baissé : Volet séjour » ;
  - « Armée, pas assez chaud » ;
  - « Armée, pas de soleil » ;
  - « Armée, soleil couché » ;
  - et « à la main : Volet chambre » pour un volet en pause.
- **Ses attributs** pour les automatisations :
  - `soleil_hauteur`, `soleil_azimut` ;
  - `chaleur`, `ensoleille` ;
  - `soleil_sur` (les volets que le soleil vise) ;
  - `baisses` (et leur position d'avant) ;
  - `en_pause`.
- **Chaque mouvement** est annoncé par l'événement `protection_solaire`
  (`{action: baisse, entity_id: cover.salon, position: 25}`) et noté dans le journal de bord
  (« par l'automatisation « Protection solaire » »).

## Ce qui la retient

- **Une main :**
  - un volet bougé par quelqu'un, par une automatisation ou par la simulation de présence est
    laissé tranquille pendant `pause_manuelle` ;
  - elle oublie aussi de le remonter, puisqu'on l'a déjà réglé.
  - Ses propres mouvements ne comptent pas : peu après son ordre, un volet qui va vers la position
    demandée, c'est elle. Un volet remonté juste après qu'elle l'a baissé, c'est une main.
- **Un volet déjà assez bas** n'est pas touché, et ne sera pas remonté plus tard.
- **15 minutes entre deux mouvements** d'un même volet : un nuage qui passe ne le fait pas monter
  et descendre sans cesse. Seul le désarmement remonte tout de suite.
- **Un redémarrage :** les volets qu'elle avait baissés sont repris, et remontés le moment venu.
- **Un volet sans position** (seulement ouvrir et fermer) est fermé, puis rouvert.

## Sécurité

- Elle ne commande **que des volets** (`cover.…`) listés dans le fichier : pas un portail, si tu ne
  le mets pas dans la liste.
- Ses ordres passent par les mêmes vérifications que les autres : position de 0 à 100, intégration
  en panne refusée.
- **Les invités** ne voient pas son interrupteur, comme le délestage et la simulation de présence.
- Limites : 30 volets ; seuil de −20 à 50 °C ; pause de 0 à 24 h.

## Comment c'est construit

- `crates/ha-shade` (nouveau) :
  - `parse_config` lit le fichier, avec les points cardinaux en français ;
  - `CoverConfig::sun_on_facade` compare l'azimut du soleil et l'orientation de la façade, y compris
    autour du nord (350° et 10° sont à 20° l'un de l'autre) ;
  - `Heat` garde l'écart de température ;
  - `Tracker` décide pour un volet, sans horloge ni réseau ;
  - `Shades` fait tourner le tout toutes les 30 s, et tout de suite quand la température, la météo
    ou un volet change.
- `crates/ha-sun` donne la position du soleil (`position`), déjà utilisée par `sun.sun`.
- `crates/ha-server` : le fichier (`HA_PROTECTION_SOLAIRE_PATH`) et
  `ha-server protection-solaire check`.
- `crates/ha-api` : l'interrupteur caché aux invités, la tuile.

Essayé de bout en bout avec mosquitto et deux faux volets MQTT, qui descendent et remontent par
pas de 10 % comme un vrai moteur. À Paris, le 17 octobre à 9 h 15 (soleil à 24° de haut, plein
sud-est) :
- à 27 °C, le volet de la façade sud-est est descendu à 20 % ; celui de la façade ouest n'a pas
  bougé ;
- après un redémarrage, la protection savait qu'il était à 100 % avant ;
- la désarmer l'a remonté aussitôt ;
- le volet de la chambre, réglé à la main, a été mis en pause, avec une seule ligne de journal.

Ce n'est pas un vrai volet Somfy : si le tien ne réagit pas, envoie-moi ce que dit le journal.

### Concepts Rust utilisés

**`rem_euclid` pour les angles** : `(azimut − orientation + 540).rem_euclid(360) − 180` donne
l'écart entre −180° et 180°, quel que soit le côté du nord. Contrairement à `%`, `rem_euclid`
ne rend jamais de reste négatif.

**`#[serde(untagged)]`** : `orientation: sud` et `orientation: 180` sont lus par la même
énumération `Orientation { Degrees(f64), Name(String) }`.

**Une source de soleil remplaçable** : `Shades::start` reçoit une fonction
(`Box<dyn Fn() -> Option<(f64, f64)>>`). Le serveur lui donne la vraie position du soleil
(`real_sun`). Le test lui donne un soleil fixe, plein sud, pour vérifier le service en quelques
millisecondes.
