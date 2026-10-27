# Étape 22 : répéter, attendre, toutes les N minutes, expliqué

## Le but

Trois choses manquaient encore par rapport à Home Assistant pour écrire les automatisations du
quotidien :

- **« toutes les 15 minutes »** : le déclencheur `time_pattern` ;
- **« recommence tant que… »** : l'action `repeat` ;
- **« attends que la porte se ferme, 10 minutes au plus »** : l'action `wait_for_trigger`.

Elles s'écrivent exactement comme dans Home Assistant, et se construisent aussi dans l'éditeur du
tableau de bord.

## Toutes les N minutes : `time_pattern`

```yaml
triggers:
  - trigger: time_pattern
    minutes: "/15"      # à 0, 15, 30 et 45 minutes de chaque heure
```

| Écrit | Se déclenche |
|-------|--------------|
| `minutes: "/15"` | toutes les 15 minutes (h:00, h:15, h:30, h:45) |
| `hours: "/2"` | toutes les 2 heures, à l'heure pile (0 h, 2 h, 4 h…) |
| `hours: 7` | chaque jour à 7 h 00 |
| `minutes: 30` | chaque heure à la demie |
| `hours: "*", minutes: 5, seconds: 30` | chaque heure à h:05:30 |

La règle (celle de Home Assistant) : ce que tu n'écris pas et qui est **plus petit** que ce que tu
as écrit vaut 0, ce qui est **plus grand** vaut « toutes ». Mets `"/15"` entre guillemets : la barre
oblique doit rester du texte.

**Au plus une fois par minute** : `seconds` doit être une valeur fixe (`seconds: "/5"` est refusé).
Une automatisation lancée plus de 10 fois par minute serait de toute façon arrêtée par la
protection contre les boucles.

## Répéter : `repeat`

```yaml
# 3 fois : faire clignoter la lumière.
- repeat:
    count: 3
    sequence:
      - action: light.toggle
        target: {entity_id: light.entree}
      - delay: 1s

# Tant que la porte du garage est ouverte : un message toutes les 10 minutes.
- repeat:
    while:
      - condition: state
        entity_id: binary_sensor.porte_garage
        state: "on"
    sequence:
      - action: notify.send_message
        target: {entity_id: notify.telephone}
        data: {message: La porte du garage est ouverte.}
      - delay: 10min
```

- `count` : un nombre de fois (1 à 100) ;
- `while` : **tant que** les conditions sont vraies, vérifiées **avant** chaque tour (zéro tour si
  elles sont fausses dès le départ) ;
- `until` : **jusqu'à ce que** les conditions soient vraies, vérifiées **après** chaque tour (au
  moins un tour).

**Garde-fou** : 100 tours au plus, même avec `while` ou `until`. Une condition qui ne devient
jamais vraie ne fait pas tourner le serveur à l'infini (le journal du serveur le signale). Pense au
`delay` dans la `sequence`, sinon les 100 tours passent en un instant.

## Attendre : `wait_for_trigger`

```yaml
# La porte du garage s'ouvre : lumière allumée, puis éteinte quand la porte se ferme.
actions:
  - action: light.turn_on
    target: {entity_id: light.garage}
  - wait_for_trigger:
      - trigger: state
        entity_id: binary_sensor.porte_garage
        to: "off"
    timeout: 10min
    continue_on_timeout: false
  - action: light.turn_off
    target: {entity_id: light.garage}
```

- `wait_for_trigger` : un ou plusieurs déclencheurs, écrits **comme les déclencheurs habituels**
  (`state` avec `for:`, `numeric_state`, `time`, `time_pattern`, `event`). L'un d'eux suffit.
- `timeout` : l'attente maximale (24 h au plus). Sans `timeout`, on attend aussi longtemps qu'il
  faut.
- `continue_on_timeout` : après l'attente dépassée, `true` (par défaut) continue avec les actions
  suivantes, `false` s'arrête là.

Le soleil n'est pas possible dans une attente : utilise plutôt une condition `sun`.

**Bon à savoir** : l'attente commence au moment où l'action est atteinte. Si la porte s'est déjà
fermée juste avant, l'attente ne le voit pas (comme dans Home Assistant) : ajoute une condition
avant si besoin. Avec `mode: restart`, une nouvelle ouverture de la porte recommence tout (et
abandonne l'attente en cours).

## Dans l'éditeur du tableau de bord

- Déclencheur **« Régulièrement (toutes les N minutes) »** : heures, minutes, seconde.
- Action **« Répéter »** : « un nombre de fois », « tant que… » ou « jusqu'à ce que… », et les
  actions à répéter.
- Action **« Attendre un événement »** : les déclencheurs attendus, la durée maximale, et que faire
  si elle est dépassée.

La liste des automatisations les décrit en français : « Quand toutes les 15 minutes · alors répéter
2 fois : allumer salon, puis attendre : porte entrée passe à « off » (au plus 5min, sinon on
s'arrête) ».

## Comment c'est construit

| Fichier | Contenu |
|---------|---------|
| [config/](../crates/ha-automation/src/config/) | Lecture et vérification (`check_time_pattern`, `repeat`, `wait_for_trigger`) |
| [evaluator.rs](../crates/ha-automation/src/evaluator.rs) | `pattern_crossed` : un moment du motif est-il passé depuis la dernière seconde ? |
| [engine.rs](../crates/ha-automation/src/engine.rs) | L'exécution de `repeat`, et `wait_for_trigger` |
| [app.js](../crates/ha-api/web/app.js) | L'éditeur |

**Attendre, en réutilisant l'évaluateur.** Pour `wait_for_trigger`, le moteur fabrique une
automatisation « fantôme » qui n'a que les déclencheurs attendus, et la donne à l'évaluateur
habituel. `for:`, `numeric_state`, `time`… marchent donc exactement comme dans les déclencheurs
normaux, sans une ligne de code en double.

**Concepts Rust rencontrés :**

- **`let … else`** : `let Pattern::At(second) = … else { return Err(…) };` : « ce doit être une
  valeur fixe, sinon on refuse », en une instruction.
- **`Iterator::rposition`** : trouver le **dernier** morceau indiqué (heures, minutes ou secondes)
  pour appliquer la règle « plus petit : 0, plus grand : toutes ».
- **`tokio::select!` à trois branches** : un événement du bus, une seconde qui passe, ou l'attente
  dépassée ; la première arrivée gagne.
- **`tokio::task::yield_now`** : entre deux tours d'une répétition, la tâche laisse passer les autres.
  Même 100 tours sans `delay` ne bloquent pas le serveur.

## Testé

- La lecture : les exemples du tableau ci-dessus, et le refus des valeurs invalides (`/0`, 60
  minutes, 24 h, texte, secondes répétées, champ inconnu, `repeat` sans ou avec deux modes, `count`
  de 0 ou 101, `sequence` vide, attente sans déclencheur, avec le soleil, durée illisible).
- `time_pattern` heure par heure : 10:15 oui, 10:15:01 non (une seule fois), une seconde sautée
  rattrapée, 13 h non et 14 h oui pour « toutes les 2 heures », le passage de minuit.
- Avec le vrai moteur (temps simulé) : `count: 3` → 3 fois ; `while` → 3 messages en 25 s, puis plus
  rien une fois l'alarme arrêtée ; `until` jamais vrai → arrêté à 100 tours ; la porte du garage
  refermée au bout d'une minute → lumière éteinte ; restée ouverte 10 minutes → on s'arrête sans
  éteindre.
- Dans Chromium : l'automatisation « Garage » construite dans le formulaire (toutes les 15 minutes,
  répéter 2 fois, attendre la porte 5 min sinon s'arrêter), passage « tant que » puis retour au
  nombre, enregistrée avec confirmation du mot de passe. Aucune erreur.
