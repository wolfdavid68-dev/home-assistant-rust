# Étape 5 : les automatisations, expliqué

## Le principe

Une automatisation, c'est « **quand** X se produit, **si** Y est vrai, **alors** fais Z » :

```yaml
- id: salon_au_retour
  alias: Lumière du salon au retour le soir
  triggers:                      # QUAND la porte s'ouvre…
    - trigger: state
      entity_id: binary_sensor.porte_entree
      to: "on"
  conditions:                    # SI c'est le soir et que la lampe est éteinte…
    - condition: time
      after: "18:00"
      before: "23:30"
    - condition: state
      entity_id: light.salon
      state: "off"
  actions:                       # ALORS allumer, attendre 10 min, éteindre.
    - action: light.turn_on
      target: {entity_id: light.salon}
      data: {brightness: 180}
    - delay: 10min
    - action: light.turn_off
      target: {entity_id: light.salon}
```

C'est la **même syntaxe que Home Assistant** : la plupart des exemples trouvés en ligne
s'adaptent facilement. Le fichier [automations.example.yaml](../automations.example.yaml) en
contient quatre, commentés.

```
          bus d'événements ──┐            ┌── horloge (chaque seconde)
                             ▼            ▼
                     ┌───────────────────────────┐
                     │ Evaluator : quels triggers ?│
                     │            conditions OK ?  │
                     └─────────────┬─────────────┘
                                   ▼
                     actions → Core::call_service → MQTT / Hue → appareil
```

## Ce qui est pris en charge

| Déclencheurs (`triggers`) | |
|---|---|
| `state` | une entité change de valeur (`from`, `to` facultatifs), éventuellement depuis un certain temps (`for: 5min`) |
| `numeric_state` | une valeur passe sous `below` ou au-dessus de `above` |
| `time` | tous les jours à une heure (`at: "07:30"` ou une liste) |

| Conditions (`conditions`, toutes doivent être vraies) | |
|---|---|
| `state` | une entité a une valeur donnée (ou l'une d'une liste) |
| `numeric_state` | une valeur est dans un intervalle |
| `time` | plage horaire (`after`, `before`, même à cheval sur minuit) et jours (`weekday: [lun, mar]` ou `[mon, tue]`) |

| Actions (`actions`, dans l'ordre) | |
|---|---|
| `action: domaine.service` | appel de service sur une ou plusieurs entités (`target: entity_id:`), avec `data` facultatif |
| `delay` | attente : `30s`, `5min`, `2h` ou `00:05:00` |

Les heures sont en **heure locale** (fuseau horaire de la machine, heure d'été comprise).
Mettre `enabled: false` désactive une automatisation sans l'effacer.

## Les fichiers

| Fichier | Contenu |
|---------|---------|
| [config.rs](../crates/ha-automation/src/config.rs) | Lecture du YAML et vérification de chaque valeur |
| [evaluator.rs](../crates/ha-automation/src/evaluator.rs) | Quels déclencheurs se produisent, quelles conditions sont vraies |
| [engine.rs](../crates/ha-automation/src/engine.rs) | La tâche qui écoute le bus et l'horloge, et exécute les actions |
| [tests/automations.rs](../crates/ha-automation/tests/automations.rs) | Les tests, avec une horloge simulée |
| [automations.example.yaml](../automations.example.yaml) | Exemples à copier |

## Les concepts Rust rencontrés

**Lire puis vérifier : deux familles de types.** `config.rs` contient d'abord des types qui
ressemblent au YAML (`TriggerConfig` avec des `String`), puis des types « vérifiés » (`Trigger` avec
des `EntityId`, des `Time`, des `Duration`). La fonction `check` passe des uns aux autres. Une fois
qu'on a un `Automation`, plus rien ne peut être invalide : le compilateur le garantit, comme pour
`EntityId` à l'étape 1. On appelle ce principe « parse, don't validate ».

**Les enums serde.** `#[serde(tag = "trigger")]` choisit la variante d'après le champ
`trigger: state`. `#[serde(untagged)]` essaie chaque variante : c'est ainsi que `entity_id` accepte
une valeur seule **ou** une liste.

**`collect::<Result<Vec<_>, _>>()`.** Transforme une liste de résultats en résultat de liste :
on obtient tous les éléments si tout va bien, sinon la **première** erreur.

**Tester le temps sans attendre.** `#[tokio::test(start_paused = true)]` met l'horloge de tokio en
pause et `tokio::time::advance` la fait avancer instantanément. Le test « porte ouverte depuis
5 minutes » s'exécute donc en quelques millisecondes. L'heure de la journée, elle, est passée en
paramètre à l'`Evaluator`, d'où des tests comme « de 23:59:59 à 00:00:01 ».

**`BTreeSet`.** Un ensemble trié et sans doublon : si deux déclencheurs d'une même automatisation
se produisent ensemble, elle ne part qu'une fois.

**`std::mem::take` et `partition`.** Pour trier les attentes `for:` en « échues » et « encore en
attente » sans copier : on prend le vecteur (en laissant un vecteur vide à sa place), puis on le
coupe en deux.

## Choix de sécurité de cette étape

- **Une automatisation ne peut qu'appeler des services et attendre.** Pas de commande système
  (Home Assistant a `shell_command`), pas de modèle Jinja, pas de code : le fichier ne peut rien
  faire d'autre que ce que permet déjà l'API.
- **Lecture stricte** : tout champ inconnu est refusé (`too:` au lieu de `to:` donne une erreur
  avec la ligne et la colonne), chaque entité, heure, durée et nom de service est vérifié.
  Un fichier invalide **empêche le démarrage**, avec un message clair, plutôt que de tourner à
  moitié.
- **Bibliothèque YAML sûre** : `serde-saphyr`, en Rust pur et conçue pour ne jamais planter. Elle
  bloque les « bombes YAML » (des alias qui se multiplient jusqu'à saturer la mémoire), et un test
  le vérifie. L'ancienne bibliothèque de référence, `serde_yaml`, n'est plus maintenue, et sa
  remplaçante `serde_yaml_ng` dépend d'une brique abandonnée : `cargo deny` l'a refusée.
- **Limites** : fichier ≤ 1 Mio, 200 automatisations, 20 déclencheurs, conditions ou actions
  chacune, attentes ≤ 24 h.
- **Protection contre les boucles** : une automatisation qui se déclenche plus de 10 fois en une
  minute est ignorée (avec un avertissement). Sans cela, deux automatisations qui se répondent
  pourraient faire clignoter une lampe à l'infini. Un test simule cette boucle.
- **Une seule exécution à la fois** par automatisation : si elle est encore dans un `delay`, un
  nouveau déclenchement est ignoré (comme le mode `single` de Home Assistant).
- `automations.yaml` est ignoré par Git : tes automatisations décrivent ta maison et tes horaires.

## Essayer

```powershell
Copy-Item automations.example.yaml automations.yaml
# adapte les noms d'entités (GET /api/states), puis :
cargo run -p ha-server -- automations check
cargo run -p ha-server
```

`automations check` affiche la liste des automatisations comprises, ou l'erreur exacte. Dans les
logs du serveur, chaque déclenchement apparaît (`automatisation « … » déclenchée`). Après une
modification du fichier, relance le serveur.

Avec les appareils simulés (sans MQTT ni Hue), la porte `binary_sensor.porte_entree` s'ouvre
toutes les 6 secondes : pratique pour tester un déclencheur. Les appareils simulés n'obéissent
pas aux ordres, mais on voit les appels de service dans les logs.

## Pas encore pris en charge

Déclencheurs d'événements, conditions « ou », actions conditionnelles (`choose`, `if`), modes
`restart`/`queued`. (Le soleil est arrivé à l'étape 8, et l'édition depuis le tableau de bord,
sans redémarrer, à l'étape 13 : voir [l'éditeur](editeur-automatisations.md).)

```bash
cargo test -p ha-automation
```
