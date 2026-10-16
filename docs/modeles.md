# Étape 34 : les modèles et les déclencheurs plus fins, expliqués

## Le but

Deux choses manquaient pour écrire des automatisations utiles :

- **des valeurs dans les messages** : « Il fait 31 °C dehors », « Vigilance orange : orages » ;
- **des déclencheurs plus fins** : sur un attribut (la luminosité d'une lampe, le vent dans la
  vigilance), sur plusieurs valeurs à la fois (« orange **ou** rouge »), ou en excluant des valeurs.

Les deux s'écrivent comme dans Home Assistant : une automatisation copiée depuis Home Assistant a
de bonnes chances de marcher telle quelle.

## Des déclencheurs plus fins

```yaml
triggers:
  # Plusieurs valeurs : l'une d'elles suffit.
  - trigger: state
    entity_id: sensor.vigilance_meteo
    to: [Orange, Rouge]

  # Un attribut plutôt que l'état : seul son changement compte.
  - trigger: state
    entity_id: light.salon
    attribute: brightness
    to: 255

  # Toute nouvelle valeur, sauf en sortie de panne et sauf « Vert ».
  - trigger: state
    entity_id: sensor.vigilance_meteo
    not_from: [unavailable, unknown]
    not_to: Vert
```

| Clé                 | Rôle                                                                 |
| ------------------- | -------------------------------------------------------------------- |
| `from`, `to`        | l'ancienne, la nouvelle valeur : une valeur ou une liste             |
| `not_from`, `not_to`| les valeurs **exclues**                                              |
| `attribute`         | surveiller un attribut au lieu de l'état                             |
| `for`               | rester ainsi au moins ce temps (comme avant)                         |

La condition `state` accepte aussi `attribute` :

```yaml
conditions:
  - condition: state
    entity_id: sensor.vigilance_meteo
    attribute: niveau_actuel
    state: [Orange, Rouge]
```

Un attribut dont le nom contient des espaces s'écrit tel quel : `attribute: Vent violent`.

**Correction au passage** : un déclencheur avec `for:` (« porte ouverte depuis 5 minutes ») était
annulé quand **un autre attribut** de l'appareil changeait (le niveau de batterie d'un capteur, par
exemple). Seul un changement de la valeur surveillée l'annule désormais.

## Des valeurs dans les messages : les modèles

Dans les données d'une action (`data:`), un texte peut contenir des `{{ … }}`, calculés au moment
où l'action s'exécute :

```yaml
- id: vigilance_telephone
  alias: Vigilance orange ou rouge
  triggers:
    - trigger: state
      entity_id: sensor.vigilance_meteo
      to: [Orange, Rouge]
  actions:
    - action: notify.send_message
      target: {entity_id: notify.telephone}
      data:
        title: "Vigilance {{ trigger.to_state.state | lower }}"
        message: >-
          {{ trigger.to_state.name }} : {{ trigger.from_state.state | lower }} →
          {{ trigger.to_state.state | lower }}. Dehors :
          {{ states('sensor.meteo_temperature') | round(1) }} °C.
```

Ce qui donne sur le téléphone : « Vigilance orange — Vigilance météo : jaune → orange. Dehors :
18.5 °C. »

### Ce qu'on peut lire

| Écriture                                     | Valeur                                                 |
| -------------------------------------------- | ------------------------------------------------------ |
| `states('sensor.salon')`                     | l'état (`unknown` si l'entité n'existe pas)            |
| `state_attr('light.salon', 'brightness')`    | un attribut (rien s'il n'existe pas)                   |
| `is_state('light.salon', 'on')`              | `True` ou `False`                                      |
| `trigger.to_state.state`                     | la nouvelle valeur de l'entité qui a déclenché         |
| `trigger.from_state.state`                   | son ancienne valeur                                    |
| `trigger.to_state.name`                      | son nom (« Vigilance météo »)                          |
| `trigger.to_state.attributes.brightness`     | un attribut ; avec des espaces : `attributes['Vent violent']` |
| `trigger.entity_id`, `trigger.id`            | l'entité, l'identifiant du déclencheur                 |

`trigger.…` vient d'un déclencheur `state` ou `numeric_state`. Avec un autre déclencheur (une heure,
le soleil…), il est vide.

### Les filtres

| Filtre             | Effet                                                       |
| ------------------ | ----------------------------------------------------------- |
| `round(1)`         | arrondi (0 à 6 décimales)                                   |
| `int`, `float`     | en nombre ; `float(0)` donne 0 si ce n'est pas un nombre    |
| `lower`, `upper`, `capitalize` | minuscules, majuscules, majuscule au début      |
| `default('?')`     | la valeur à utiliser si rien n'a été trouvé                 |
| `replace('_', ' ')`| remplace un texte par un autre                              |
| `abs`              | valeur absolue (étape 64)                                   |

Ils s'enchaînent : `{{ states('sensor.cave') | float(0) | round(1) }}`.

### Un nombre plutôt qu'un texte

Quand le texte n'est **qu'un seul** `{{ … }}` qui donne un nombre, l'action reçoit un nombre :
utile pour une luminosité ou une consigne.

```yaml
- action: light.turn_on
  target: {entity_id: light.chambre}
  data:
    brightness: "{{ state_attr('light.salon', 'brightness') | int(128) }}"
```

### Les erreurs

Un modèle mal écrit est refusé **au chargement**, avec la place exacte :
`data.message : modèle : filtre json inconnu (round, int, float…)`. L'éditeur du tableau de bord
le signale pendant qu'on écrit.

À l'exécution, rien ne bloque : une entité ou un attribut introuvable donne un texte vide, ou la
valeur de `default(…)`.

## Ce qui n'est pas pris en charge (exprès)

Pas de `{% if %}`, pas de boucles, pas de variables, pas de `now()`. Le vrai Jinja de Home Assistant
est un petit langage de programmation : un fichier d'automatisations pourrait alors faire beaucoup
plus que commander des appareils. Ici, un modèle sait **lire** un état, **calculer** (depuis
l'[étape 64](calculs.md)) et le mettre en forme, rien d'autre.

## Sécurité

- **Lire seulement** : `states`, `state_attr`, `is_state`, `trigger.…` et huit filtres. Aucune
  fonction ne peut écrire, appeler un service, lire un fichier ou le réseau.
- **Vérifié au chargement** : fonctions et filtres connus, identifiants d'entités valides, au plus
  2 000 caractères et 20 `{{ … }}` par texte, 10 modèles par action.
- **Résultat limité** à 2 000 caractères.
- Les entités lues par un modèle comptent dans la vérification des entités citées : une entité
  introuvable déclenche l'alerte de réparation habituelle (étape 31).
- Le contenu des messages n'est pas recopié dans l'historique des exécutions (il pourrait contenir
  un code).

## Comment c'est construit

- `crates/ha-automation/src/template.rs` (nouveau) : la lecture des modèles (découpage en jetons,
  puis analyse) et leur calcul ;
- `config.rs` : `attribute`, les listes, `not_from` / `not_to`, et la recherche des modèles dans
  `data` ;
- `evaluator.rs` : les déclencheurs sur un attribut, et le changement d'état transmis à
  l'exécution (pour `trigger.to_state`) ;
- `engine.rs` : le calcul des modèles juste avant chaque action ;
- `web/app.js` : le champ « Attribut », plusieurs valeurs dans « De » et « À ».

### Concepts Rust utilisés

**Un analyseur en deux temps.** `tokenize` découpe `states('sensor.x') | round(1)` en jetons
(`Ident("states")`, `Open`, `Str("sensor.x")`, `Close`, `Pipe`…), puis `Parser` les lit de gauche à
droite. Le résultat est un **arbre** (`Expr`, `Base`, `Filter`) vérifié une fois pour toutes : à
l'exécution, il n'y a plus rien à analyser, seulement à calculer.

**`Peekable`** : `chars().peekable()` permet de regarder le caractère suivant sans l'avaler, pour
savoir où finit un nombre ou un nom.

**Une fonction récursive qui emprunte en `&mut`** : `slot_at` descend dans les données
(`data › tags › 0`) pour remplacer une valeur. Écrite en boucle, elle se heurte au vérificateur
d'emprunts (la variable change de propriétaire à chaque tour) ; écrite récursivement, chaque
appel ne tient qu'un seul emprunt, et le compilateur est satisfait.

**Garder un bon message d'erreur.** `OneOrMany` (une valeur ou une liste) est lu « à la main » avec
un `Visitor` : `#[serde(untagged)]` marcherait, mais remplacerait le message « `on` sans guillemets :
écris-le entre guillemets » par un vague « ne correspond à aucune variante ».
