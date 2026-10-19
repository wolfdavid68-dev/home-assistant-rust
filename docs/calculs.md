# Étape 64 : des modèles qui calculent, la condition `template` et les capteurs modèles, expliqué

## Le but

Depuis l'[étape 34](modeles.md), un modèle (`{{ … }}`) sait **lire** un état et le mettre en forme.
Il ne savait pas **calculer** : impossible d'écrire « l'écart entre le salon et dehors », ou « vrai
si l'humidité dépasse 60 % ». Cette étape ajoute trois choses, comme dans Home Assistant :

1. des **calculs et des comparaisons** dans les modèles ;
2. la **condition `template`** dans les automatisations ;
3. les **capteurs modèles** : un capteur dont la valeur est un calcul, recalculé dès que ce qu'il
   lit change.

## Ce qu'un modèle sait faire maintenant

| Écriture | Exemple | Résultat |
| --- | --- | --- |
| `+ - * /` | `{{ states('sensor.salon') \| float - states('sensor.dehors') \| float }}` | l'écart, en nombre |
| `//` et `%` | `{{ 7 // 2 }}`, `{{ 7 % 2 }}` | division entière (3), reste (1) |
| `~` | `{{ 'Il fait ' ~ states('sensor.dehors') ~ ' °C' }}` | colle des textes |
| `== != < <= > >=` | `{{ states('sensor.dehors') \| float < 5 }}` | `True` ou `False` |
| `in`, `not in` | `{{ 'david' in state_attr('zone.home', 'persons') }}` | dans une liste, ou un morceau de texte |
| `and`, `or`, `not` | `{{ is_state('light.salon', 'on') and … }}` | comme en Python |
| `x if … else y` | `{{ 'froid' if states('sensor.dehors') \| float < 5 else 'doux' }}` | l'un ou l'autre |
| `( … )`, `[ … ]` | `{{ (a + b) * 2 }}`, `{{ states('light.salon') in ['on', 'off'] }}` | parenthèses, listes |
| filtre `abs` | `{{ (a - b) \| abs }}` | valeur absolue (nouveau) |

Quelques règles, comme dans Home Assistant :
- **Un état est un texte** : pour calculer, il faut `| float` (ou `| int`).
  `states('sensor.dehors') + 1` ne donne rien, `states('sensor.dehors') | float + 1` donne le nombre.
- **Les filtres passent avant les opérations** : `a | float - b | float` calcule bien
  `(a | float) - (b | float)`.
- **Un calcul impossible** (diviser par zéro, un capteur indisponible) donne une valeur vide, que
  `default(…)` peut remplacer. Jamais d'erreur qui arrête l'automatisation.
- **Une seule comparaison à la fois** : `1 < x < 5` est refusé, écris `1 < x and x < 5`.

### Ce qui a déclenché un événement

Avec un déclencheur `event`, le modèle lit maintenant ses détails : `trigger.event.event_type` et
`trigger.event.data.…` (ou `trigger.event.data['…']`). Par exemple, pour le délestage
(étape 62) : `{{ trigger.event.data.entity_id }} coupé à {{ trigger.event.data.mesure }} A`.

## La condition `template`

```yaml
- id: humidite_sdb
  alias: Humidité de la salle de bain
  triggers:
    - {trigger: state, entity_id: sensor.humidite_sdb}
  conditions:
    # Seulement si l'humidité vient de monter de plus de 5 points.
    - condition: template
      value_template: >-
        {{ trigger.to_state.state | float(0) - trigger.from_state.state | float(0) > 5 }}
  actions:
    - action: notify.send_message
      target: {entity_id: notify.appli}
      data:
        title: Salle de bain
        message: "Humidité {{ trigger.to_state.state }} % : pense à aérer."
```

- **Quand la condition est vraie :** le modèle donne `True`, un nombre non nul, ou l'un des textes
  `on`, `yes`, `true`, `open`, `home`.
- **Ce que le modèle peut lire :** tout (états, attributs) et, comme ici, ce qui a déclenché
  l'automatisation.
- **Où l'utiliser :** dans `conditions:`, et aussi dans `if:`, `choose:`, `repeat` (`while`,
  `until`) et « continuer seulement si ».
- **Dans l'éditeur du tableau de bord :** « Un calcul est vrai (modèle) ». L'historique des
  exécutions montre le modèle, avec vrai ou faux.

## Les capteurs modèles

Copie [modeles.example.yaml](../modeles.example.yaml) en `modeles.yaml` (ou indique son chemin dans
`HA_TEMPLATES_PATH`) :

```yaml
sensor:
  - name: Écart de température
    unit_of_measurement: °C
    device_class: temperature
    state: >-
      {{ (states('sensor.temperature_salon') | float
          - states('sensor.meteo_temperature') | float) | round(1) }}

binary_sensor:
  - name: Quelqu'un en bas
    id: en_bas                 # facultatif : sinon, tiré du nom
    device_class: occupancy
    state: >-
      {{ is_state('binary_sensor.mouvement_salon', 'on')
         or is_state('binary_sensor.mouvement_cuisine', 'on') }}
```

Vérifie le fichier avec `ha-server modeles check` : la commande dit ce que chaque capteur lit.

- **Les entités créées :** `sensor.ecart_de_temperature` et `binary_sensor.en_bas`. Elles sont
  utilisables partout : tableau de bord, automatisations, historique, Alexa…
- **Le recalcul :** à chaque changement d'une entité lue, et chaque minute par sécurité.
  Un capteur modèle peut en lire un autre.
- **Un détecteur** (`binary_sensor`) vaut `on` quand son modèle est vrai, `off` sinon.
- **Un capteur** vaut le résultat, ou `unknown` quand il est vide (une mesure indisponible).
- **Les boucles sont refusées au chargement :** un capteur qui se lit lui-même, directement ou par
  un autre, se recalculerait sans fin.
- **Limites :** 100 capteurs modèles au plus. Le nom, l'unité, la classe et l'icône sont vérifiés.

Essayé sur la démo :
- **Le capteur d'écart :** 23,6 °C dans la salle de bain et 11,2 °C dehors donnent « 12,4 °C ».
  La valeur suit dès qu'une des deux températures change.
- **Le détecteur « Aérer » :** il passe à « off » quand il fait moins de 5 °C dehors.
- **L'automatisation ci-dessus :** elle part quand l'humidité monte de 61,4 à 72,4 %.
  L'historique affiche : « le modèle … est vrai ».

## Étape 68 : le temps dans les modèles

Un modèle connaît maintenant l'heure, et le moment où un appareil a changé, comme dans Home
Assistant.

| Écriture | Exemple | Résultat |
| --- | --- | --- |
| `now()` | `{{ now() }}` | « 2026-10-16T22:31:00+02:00 », l'heure locale |
| `now().hour`, `.minute`, `.second` | `{{ now().hour >= 22 }}` | l'heure, en nombre |
| `now().day`, `.month`, `.year` | `{{ now().month in [12, 1, 2] }}` | « c'est l'hiver » |
| `now().weekday()` | `{{ now().weekday() >= 5 }}` | 0 le lundi… 6 le dimanche : vrai le week-end |
| `now().isoweekday()` | | 1 le lundi… 7 le dimanche |
| `states.cover.garage.state` | | l'état, comme `states('cover.garage')` |
| `states.cover.garage.last_changed` | | quand l'état a changé (`last_updated` : l'état ou un attribut) |
| `states.cover.garage.attributes.x` | `states.light.salon.attributes['brightness']` | un attribut |
| `as_timestamp(…)` ou `… \| as_timestamp` | `{{ as_timestamp(now()) }}` | une date en secondes depuis 1970, pour compter des durées |

`last_changed` existe aussi sur `trigger.to_state` et `trigger.from_state`.

```yaml
# modeles.yaml
sensor:
  # Depuis combien de minutes le garage est ouvert (0 s'il est fermé).
  - name: Garage ouvert depuis
    unit_of_measurement: min
    state: >-
      {{ ((as_timestamp(now()) - as_timestamp(states.cover.garage.last_changed)) / 60) | round(0)
         if is_state('cover.garage', 'open') else 0 }}
  - name: Moment de la journée
    state: >-
      {{ 'nuit' if now().hour < 6 or now().hour >= 22
         else 'matin' if now().hour < 12
         else 'après-midi' if now().hour < 18 else 'soirée' }}
binary_sensor:
  - name: Week-end
    state: "{{ now().weekday() >= 5 }}"
```

- **Le recalcul :** un capteur modèle qui lit l'heure se recalcule **chaque minute**, et aussi dès
  qu'une entité qu'il lit change (ici `cover.garage`).
- **Dans une condition** (`condition: template`) : l'heure est celle du moment où la condition est
  vérifiée, par exemple `{{ now().hour >= 22 or now().hour < 6 }}`.
- **Les dates :** `as_timestamp` lit une date avec son décalage (« …+02:00 », « …Z ») ou en heure
  locale (« 2026-10-16 22:31:00 »). Une date illisible donne une valeur vide, que
  `default(…)` remplace.

Essayé sur la démo :
- « Moment de la journée » a donné « nuit » à 22 h 42, et « Week-end » « off » un vendredi ;
- « Garage ouvert depuis » est revenu à 0 minute en refermant puis rouvrant le garage. Il comptait
  avant les minutes depuis la dernière ouverture, l'avant-veille.

## Sécurité

- Un modèle reste un **calcul sans effet** : pas de boucles, pas de variables, pas de fichiers,
  pas de réseau. Il ne peut que lire des états et produire une valeur.
- **Taille bornée :**
  - 2 000 caractères ;
  - 20 `{{ … }}` par texte ;
  - 200 morceaux (valeurs, opérations, filtres) par expression ;
  - 20 niveaux d'imbrication ;
  - 50 éléments par liste.

  Une expression géante, ou des milliers de parenthèses, est refusée au chargement. Elle ne peut
  donc pas épuiser la mémoire ni la pile.
- **Pas de calcul qui s'emballe :**
  - un résultat infini (débordement) devient vide ;
  - un texte collé reste limité à 2 000 caractères ;
  - multiplier un texte (`'a' * 1000000`) n'est pas permis.
- **Le capteur modèle :** il publie sous le nom de propriétaire `template`. Il ne commande rien.

## Comment c'est construit

- `crates/ha-automation/src/template.rs` : la lecture est maintenant un **analyseur par
  priorités** :
  - `if … else`, puis `or`, `and`, `not` ;
  - puis les comparaisons, `+ - ~`, `* / // %` et le `-` devant une valeur ;
  - puis les filtres, et enfin les valeurs.

  Le résultat est un arbre (`Node`), calculé à chaque exécution. `is_true` dit si un résultat vaut
  « vrai ».
- **La condition et les événements :**
  - `config.rs` ajoute `Condition::Template` ;
  - `evaluator.rs` passe ce qui a déclenché l'automatisation aux conditions, et remplit
    `trigger.event` pour un déclencheur `event`.
- **Étape 68 :** `template.rs` ajoute `Base::Now`, `Base::Entity` (`states.x.y.champ`) et le
  filtre `AsTimestamp`. `now()` lit l'horloge au moment du calcul (`jiff::Zoned::now()`).
- `crates/ha-automation/src/entities.rs` (nouveau) : `modeles.yaml`, la recherche de boucles, et la
  tâche qui recalcule les capteurs.
- **Le serveur et l'éditeur :**
  - `crates/ha-server` : `HA_TEMPLATES_PATH` et `ha-server modeles check` ;
  - `web/app.js` : la condition dans l'éditeur.

### Concepts Rust utilisés

**Une descente récursive par priorités** : une fonction par niveau (`or` appelle `and`, qui appelle
`not`, puis `comparison`…). Chaque fonction lit ses propres opérateurs et laisse le reste au niveau
du dessous. `1 + 2 * 3` donne donc bien `1 + (2 * 3)`, sans table de priorités à part.

**`Box<Node>` pour un arbre** : un `enum` ne peut pas se contenir lui-même directement, il serait de
taille infinie. `Box` range l'enfant ailleurs en mémoire, et le nœud ne garde qu'un pointeur.

**Évaluation paresseuse de `and` et `or`** : ils sont traités à part dans `eval`, pour ne calculer la
partie droite que si elle sert. `a or b` rend `a` s'il est vrai, sans même regarder `b`, comme en
Python.

**Chercher une boucle dans un graphe** : chaque capteur lit d'autres entités. Pour chacun, on suit
ce qu'il lit avec une pile (`todo`) et un ensemble des entités déjà vues (`seen`). Si on retombe sur
le capteur de départ, il y a une boucle.
