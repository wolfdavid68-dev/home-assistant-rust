# Étape 30 : les assistants du quotidien, expliqué

![Compteur, texte, réveil et plage horaire](images/assistants.png)

## Le but

Home Assistant appelle « helpers » de petites entités pratiques qu'on crée soi-même. Il en manquait
plusieurs, trouvées en comparant avec son code (`homeassistant/components/`). Elles sont de deux
sortes.

**Des entrées qu'on règle** (fichier `inputs.yaml`, comme à l'étape 20) :

| Entrée            | Exemple                    | Services                                              |
| ----------------- | -------------------------- | ----------------------------------------------------- |
| `counter`         | lessives du mois           | `increment`, `decrement`, `reset`, `set_value`        |
| `input_text`      | message d'un écran         | `set_value` (mode `password` : jamais affiché)        |
| `input_datetime`  | heure du réveil            | `set_datetime` (`time`, `date` ou `datetime`)         |

**Des assistants calculés** à partir d'autres appareils (nouveau fichier `derived.yaml`) :

| Assistant        | Entité créée                          | Ce qu'il fait                                          |
| ---------------- | ------------------------------------- | ------------------------------------------------------ |
| `group`          | `light.lumieres_du_bas`               | allumé si un membre l'est ; une commande va à tous     |
| `schedule`       | `schedule.chauffage_matin`            | `on` pendant les plages de la semaine                  |
| `min_max`        | `sensor.temperature_moyenne`          | min, max, moyenne, médiane, somme, écart, dernier      |
| `threshold`      | `binary_sensor.salle_de_bain_humide`  | au-dessus / au-dessous d'un seuil, avec une marge       |
| `utility_meter`  | `sensor.electricite_du_jour`          | consommation depuis minuit, lundi, le 1er du mois…      |
| `history_stats`  | `sensor.chauffage_du_jour`            | combien de temps, ou de fois, une entité a été dans un état (étape 59) |

Les exemples commentés sont dans `inputs.example.yaml` et `derived.example.yaml`. Pour vérifier un
fichier avant de redémarrer :

```bash
ha-server inputs check
ha-server derived check
```

Avec Docker, le fichier se met dans `config/derived.yaml` (`HA_DERIVED_PATH`).

## L'heure du réveil, réglable depuis le tableau de bord

Le déclencheur `time` accepte maintenant une entrée `input_datetime`, comme dans Home Assistant :

```yaml
- id: reveil
  alias: Ouvrir les volets au réveil
  triggers:
    - trigger: time
      at: input_datetime.reveil
  actions:
    - action: cover.open_cover
      target: {entity_id: cover.chambre}
```

Change l'heure sur la tuile « Réveil » : l'automatisation suit, sans rien modifier. Avec une date et
une heure (`has_date` et `has_time`), elle ne se lance qu'une fois, ce jour-là.

## Les groupes

```yaml
group:
  lumieres_du_bas:
    name: Lumières du bas
    type: light               # light, switch, fan, cover ou binary_sensor
    entities: [light.salon, light.cuisine]
    all: false                # true : allumé seulement si tous le sont
```

- L'état suit les membres ; un groupe de volets donne aussi la **position moyenne**.
- « Allumer », « éteindre », « ouvrir »… sur le groupe sont **transmis à chaque membre**, au nom de
  la même personne : le journal de bord dit « par david » sur chaque lampe.
- **Pas de groupe dans un groupe** : « A commande B qui commande A » tournerait sans fin. Le fichier
  est refusé.
- **Les droits passent à travers le groupe.** Le serveur vérifie chaque membre : un invité ne peut
  pas ouvrir la porte du garage en passant par un groupe « Tous les volets » qui la contient. Le
  bouton ne lui est même pas montré.

## Les plages horaires

```yaml
schedule:
  chauffage_matin:
    monday: [{from: "06:30", to: "08:00"}, {from: "18:00", to: "22:00"}]
    saturday: [{from: "08:00", to: "24:00"}]   # 24:00 : jusqu'à minuit
```

`schedule.chauffage_matin` est `on` pendant les plages ; l'attribut `next_event` dit quand il change.
Une automatisation s'en sert avec un simple déclencheur `state` (`to: "on"`), ou une condition.
Deux plages qui se chevauchent sont refusées.

## Moyenne et seuil

```yaml
min_max:
  temperature_moyenne: {type: mean, entities: [sensor.salon, sensor.chambre], round_digits: 1}
threshold:
  salle_de_bain_humide: {entity_id: sensor.humidite_sdb, upper: 70, hysteresis: 3}
```

Le seuil a une **hystérésis** (une marge) : avec `upper: 70` et `hysteresis: 3`, il s'allume au-dessus
de 73 % et ne s'éteint qu'en dessous de 67 %. Sans elle, un ventilateur branché dessus
s'allumerait et s'éteindrait sans arrêt quand l'humidité hésite autour de 70 %.

Un capteur qui ne répond pas (`unavailable`) est simplement laissé de côté dans la moyenne.

## La consommation par période

```yaml
utility_meter:
  electricite_du_jour: {source: sensor.compteur_electricite, cycle: daily}
```

La source doit être un **compteur qui ne fait que monter** (un index en kWh, en m³). L'assistant
additionne ce qui est compté depuis le début de la période, et garde le total de la période précédente
(`last_period`). Si le compteur **baisse**, c'est qu'il a été remis à zéro (compteur remplacé) : ce
qu'il affiche alors est compté comme nouveau, comme dans Home Assistant. Ne le branche donc pas sur une
température ou une puissance instantanée.

Après un redémarrage, il reprend là où il en était (la valeur et le dernier index lu sont gardés avec
l'état). Si le serveur était arrêté au moment du changement de période, la nouvelle période commence
au démarrage.

## Les statistiques d'historique (étape 59)

Combien de temps le chauffage a tourné aujourd'hui, combien de fois la porte du garage s'est
ouverte cette semaine, quelle part de la journée la lumière du salon est restée allumée : comme
`history_stats` dans Home Assistant.

```yaml
history_stats:
  chauffage_du_jour:
    name: Chauffage aujourd'hui
    entity_id: switch.radiateur_salon
    state: "on"              # un état, ou une liste : [heat, on]
    type: time               # time (heures), count (nombre de fois), ratio (% de la période)
    cycle: daily             # hourly, daily, weekly, monthly ou yearly
  ouvertures_du_garage:
    name: Ouvertures du garage cette semaine
    entity_id: cover.garage
    state: [open, opening]
    type: count
    cycle: weekly
```

- `time` donne des heures (« 2,35 h ») ; `count`, le nombre de fois où l'entité est **entrée**
  dans l'état pendant la période (l'état à minuit ne compte pas) ; `ratio`, la part de la période
  écoulée, en %.
- Le calcul part de l'**historique enregistré** : il survit à un redémarrage. Il est refait
  chaque minute, par la base (sans limite de lignes).
- La période recommence à minuit, le lundi, le 1er du mois… comme la consommation par période.
- L'historique est gardé 10 jours par défaut (`HA_HISTORY_DAYS`) : pour `monthly` ou `yearly`,
  augmente-le, sinon le début de la période est oublié.

Dans une automatisation : « si le chauffage a déjà tourné 6 heures aujourd'hui, baisse la
consigne » (`numeric_state` sur `sensor.chauffage_du_jour`, `above: 6`).

## L'intégrale et la dérivée (étape 65)

Deux assistants de Home Assistant (`integration` et `derivative`) qui travaillent sur le temps.

### L'intégrale : une puissance devient une énergie

Une prise qui ne donne que sa **puissance** (en W) ne dit pas combien elle a consommé. Son
intégrale, si : 1 000 W pendant une demi-heure font 0,5 kWh.

```yaml
integration:
  lave_linge_energie:
    name: Lave-linge, énergie
    source: sensor.prise_lave_linge_puissance
    unit_prefix: k          # k : kWh (÷ 1 000) ; M : MWh ; sans : Wh
    unit_time: h            # s, min, h (par défaut) ou d
    method: left            # left (par défaut) ou trapezoidal
    round: 3
```

- **L'unité :** elle se compose toute seule, à partir de celle de la source (W donne kWh).
  Une source de classe `power` donne un capteur de classe `energy`.
- **Le bilan énergétique :** mets `sensor.lave_linge_energie` dans `energy.yaml`, avec un prix, pour
  avoir les euros dans le bilan (étape 32).
- **`left`, le calcul par défaut :** la puissance est comptée telle qu'elle a été mesurée, jusqu'à
  la mesure suivante. C'est exact pour nos capteurs, qui ne publient qu'au changement.
  `trapezoidal` prend la moyenne de deux mesures, comme Home Assistant par défaut : c'est mieux
  pour une source qui varie en continu et qui est lue à intervalle régulier.
- **Une puissance stable** compte quand même : le total avance **chaque minute**, même sans
  nouvelle mesure.
- **La source indisponible :** rien n'est compté jusqu'à son retour, car on ne sait pas ce
  qu'elle a fait.
- **Après un redémarrage :** le total repart de sa valeur d'avant. Le temps où le serveur était
  arrêté n'est pas compté.

### La dérivée : à quelle vitesse une valeur change

```yaml
derivative:
  chambre_variation:
    name: Variation de température de la chambre
    source: sensor.temperature_chambre
    time_window: "00:10:00"   # la fenêtre : 10 minutes par défaut (de 1 minute à 24 heures)
    unit_time: h              # °C par heure
    round: 2
```

- **Le calcul :** la pente entre la valeur au début de la fenêtre et la valeur actuelle. Une
  température qui passe de 20 à 19,5 °C en 5 minutes donne « -6 °C/h ». Une valeur qui ne bouge plus
  revient à 0.
- **L'unité :** elle suit la source, « °C/h » ici.
- **Les cas sans valeur :** `unknown` tant qu'il n'y a pas deux mesures, et la fenêtre repart de
  zéro après une indisponibilité.

Dans une automatisation : « si la chambre perd plus de 3 °C par heure, une fenêtre est sûrement
ouverte : couper le radiateur » :

```yaml
- id: fenetre_chambre
  alias: Fenêtre ouverte dans la chambre
  triggers:
    - trigger: numeric_state
      entity_id: sensor.chambre_variation
      below: -3
  actions:
    - action: climate.set_hvac_mode
      target: {entity_id: climate.chambre}
      data: {hvac_mode: "off"}
```

La source d'une intégrale ou d'une dérivée ne peut pas être un autre assistant de `derived.yaml`
(comme pour les autres). Elle peut en revanche être un capteur modèle (étape 64).

## Comment c'est construit

- `crates/ha-input` : les trois nouvelles entrées, sur le modèle des autres.
- `crates/ha-derived` (nouveau) : `config.rs` lit et vérifie le fichier, `compute.rs` fait les calculs
  (sans attente : l'heure et les états sont passés en paramètre, ce qui les rend faciles à tester),
  `lib.rs` est la tâche qui écoute les changements d'état et les commandes.
- `crates/ha-automation` : le déclencheur `time` accepte une entrée `input_datetime`.
- Étape 65 :
  - `compute.rs` ajoute `Integral` (avec `feed`, `advance` et `resume`) et `Slope` (une fenêtre de
    mesures) ;
  - `lib.rs` les fait avancer à chaque mesure et chaque minute.

### Les concepts Rust

**`f64::total_cmp` pour trier des nombres à virgule.** Les `f64` ne sont pas « totalement ordonnés » :
`NaN` (« pas un nombre ») n'est ni plus grand ni plus petit qu'un autre. Rust refuse donc
`values.sort()` sur des `f64`. `sort_by(|a, b| a.total_cmp(b))` donne un ordre complet, utilisé pour
la médiane.

**Une étiquette sur une boucle.** Pour trouver le prochain changement d'une plage horaire, on parcourt
les jours, puis les heures de chaque jour. Quand on trouve, on sort **des deux boucles** d'un coup :

```rust
'days: for offset in 0..8_i64 {
    for moment in moments {
        if moment > now && schedule_on(week, moment) != on {
            next = Some(moment);
            break 'days;
        }
    }
}
```

**Un piège trouvé par les tests.** La bibliothèque de dates `jiff` accepte de lire une **heure** dans
« 2026-10-16 07:30:00 » (elle garde l'heure et oublie la date). Un réveil réglé pour un jour précis
aurait sonné tous les jours. Le test l'a montré ; le code vérifie maintenant qu'il n'y a pas de date
avant de lire une heure seule.
