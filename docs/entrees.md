# Étape 20 : les entrées, expliqué

## Le but

Certaines choses ne sont pas des appareils, mais la maison doit s'en souvenir : « on est en
vacances », « la consigne de la chambre », « le mode de la maison (normal, nuit, absent) »,
« la lessive tourne encore 40 minutes ». Home Assistant les appelle des **entrées** (*helpers*) :
des appareils **virtuels**, réglés depuis le tableau de bord, que les automatisations lisent et
commandent.

| Sorte | Exemple | État | Commandes |
|-------|---------|------|-----------|
| `input_boolean` | `input_boolean.mode_vacances` | `on` / `off` | `turn_on`, `turn_off`, `toggle` |
| `input_number` | `input_number.consigne_chambre` | `19.5` | `set_value`, `increment`, `decrement` |
| `input_select` | `input_select.mode_maison` | `Nuit` | `select_option`, `select_next`, `select_previous`, `select_first`, `select_last` |
| `input_button` | `input_button.bonne_nuit` | date du dernier appui | `press` |
| `timer` | `timer.lessive` | `idle` / `active` / `paused` | `start`, `pause`, `cancel`, `finish` |

## Mettre en place

1. Copie [inputs.example.yaml](../inputs.example.yaml) en `inputs.yaml` (avec Docker :
   `config/inputs.yaml`), et garde ce dont tu as besoin :

   ```yaml
   input_boolean:
     mode_vacances:
       name: Mode vacances

   input_number:
     consigne_chambre:
       name: Consigne chambre
       min: 15
       max: 23
       step: 0.5
       unit_of_measurement: "°C"   # mode: box pour des boutons + et − au lieu du curseur

   input_select:
     mode_maison:
       name: Mode de la maison
       options: [Normal, Nuit, Absent]

   input_button:
     bonne_nuit:
       name: Bonne nuit

   timer:
     lessive:
       name: Lessive
       duration: "01:30:00"          # ou 90min, 30s, 2h, ou un nombre de secondes
   ```

2. Vérifie et redémarre :

   ```powershell
   cargo run -p ha-server -- inputs check
   ```

Les entrées apparaissent dans la section **« Entrées »** du tableau de bord : un interrupteur, un
curseur (ou − / +), une liste déroulante, un bouton « Appuyer », et un minuteur avec son
**compte à rebours** et les boutons Démarrer / Pause / Reprendre / Annuler.

## Dans les automatisations

```yaml
# Le mode vacances met le chauffage hors gel.
- id: vacances
  triggers:
    - trigger: state
      entity_id: input_boolean.mode_vacances
      to: "on"
  actions:
    - action: climate.set_preset_mode
      target: {entity_id: climate.salon}
      data: {preset_mode: hors_gel}

# Le bouton « Bonne nuit » : tout éteindre et passer en mode Nuit.
- id: bonne_nuit
  triggers:
    - trigger: state
      entity_id: input_button.bonne_nuit
  actions:
    - action: light.turn_off
      target: {entity_id: [light.salon, light.entree]}
    - action: input_select.select_option
      target: {entity_id: input_select.mode_maison}
      data: {option: Nuit}

# La lessive est finie : prévenir le téléphone.
- id: lessive_finie
  triggers:
    - trigger: event
      event_type: timer.finished
      event_data: {entity_id: timer.lessive}
  actions:
    - action: notify.send_message
      target: {entity_id: notify.telephone}
      data: {title: Lessive, message: La machine est finie.}
```

Une entrée sert aussi de **condition** : « seulement si `input_boolean.mode_vacances` est `off` ».

### Le nouveau déclencheur `event`

Un minuteur qui passe de `active` à `idle` peut avoir **fini**… ou avoir été **annulé**. Pour faire
la différence, il publie un **événement** : `timer.started`, `timer.restarted`, `timer.paused`,
`timer.cancelled` ou `timer.finished`, avec `{entity_id: timer.lessive}`. Le déclencheur `event`
s'y abonne. `event_data` est facultatif : sans lui, n'importe quel minuteur convient. Il est aussi
dans l'éditeur du tableau de bord (« Un événement (minuteur fini…) »).

`timer.start` accepte une autre durée : `data: {duration: 10min}`. Sur un minuteur en marche, il
repart de zéro (`timer.restarted`) ; en pause, sans durée, il reprend où il en était.

## Après un redémarrage

- Les valeurs reviennent comme avant l'arrêt (sauf si tu as mis `initial:` dans le fichier : la
  valeur `initial` gagne alors à chaque démarrage).
- Un minuteur en marche **continue** : il sonne à la même heure qu'avant l'arrêt.
- Un minuteur qui **aurait dû sonner pendant l'arrêt** sonne au démarrage : ton automatisation
  « lessive finie » est quand même lancée.

## Bon à savoir

- Une valeur hors limites est **ramenée** dans l'intervalle et sur le pas : `set_value 99` sur une
  consigne de 15 à 23 donne 23 ; `19.3` avec un pas de 0,5 donne 19,5.
- Une commande invalide (option inconnue, valeur qui n'est pas un nombre) est **refusée** et notée
  dans le journal du serveur : rien ne change.
- Le compte à rebours est calculé par le navigateur d'après l'heure de fin : si l'horloge du
  téléphone est fausse, il l'est aussi (le minuteur, lui, sonne à l'heure du serveur).
- Limites : 50 entrées de chaque sorte, 50 options par liste, minuteurs de 24 h au plus.

## Comment c'est construit

| Fichier | Contenu |
|---------|---------|
| [ha-input/src/config.rs](../crates/ha-input/src/config.rs) | Lecture et vérification de `inputs.yaml` |
| [ha-input/src/helper.rs](../crates/ha-input/src/helper.rs) | Ce que fait chaque entrée, sans horloge : facile à tester |
| [ha-input/src/lib.rs](../crates/ha-input/src/lib.rs) | La tâche : appels de service, minuteurs qui sonnent |
| [ha-core/src/event.rs](../crates/ha-core/src/event.rs) | Le nouvel événement nommé `Event::Fired` |
| [evaluator.rs](../crates/ha-automation/src/evaluator.rs) | Le déclencheur `event` |
| [app.js](../crates/ha-api/web/app.js) | Les tuiles, le compte à rebours, l'éditeur |

**Le piège du premier démarrage.** Créer une entrée, c'est passer de « rien » à une valeur. Pour le
moteur, c'est un changement : au tout premier démarrage, le bouton « Bonne nuit » (de « rien » à
« jamais appuyé ») aurait lancé son automatisation tout seul ! Les entrées sont donc démarrées en
**deux temps** : créées **avant** le moteur d'automatisations (il ne voit pas leur création), puis
mises en marche **après** (pour qu'il reçoive le `timer.finished` d'un minuteur qui a sonné pendant
l'arrêt). Un test le vérifie.

**Concepts Rust rencontrés :**

- **Ajouter une variante à un `enum`** (`Event::Fired`) : le compilateur a refusé de compiler
  tant que chaque `match` sur les événements (historique, journal, WebSocket, thermostat,
  automatisations) ne disait pas quoi en faire. Aucun oubli possible.
- **Un état par variante** : `TimerState::Active { deadline, … }` et `Paused { remaining, … }`.
  Un minuteur arrêté n'a pas d'heure de fin : ce n'est pas un champ vide qu'on pourrait oublier de
  vérifier, il n'existe tout simplement pas.
- **`tokio::time::sleep_until`** dans un `select!` : la tâche dort jusqu'au prochain minuteur, ou
  indéfiniment (`std::future::pending`) s'il n'y en a aucun. Pas de vérification chaque seconde.
- **L'heure passée en paramètre** (`now`, `wall`) : les tests vérifient « 4 minutes plus tard » sans
  attendre 4 minutes.

## Testé

- Le fichier : chaque sorte, valeurs par défaut, et refus des fautes (section inconnue, champ
  inconnu, min ≥ max, pas nul, option en double, durée de 25 h…).
- Chaque entrée : bascule, nombre ramené dans l'intervalle et sur le pas, options dans l'ordre (en
  boucle ou non), minuteur démarré, mis en pause (le temps restant ne bouge plus), repris, annulé,
  relancé avec une autre durée ; valeurs restaurées, `initial` prioritaire.
- Avec le vrai moteur (temps simulé) : la lessive finie lance l'automatisation, une lessive annulée
  non ; « Bonne nuit » ne fait rien en mode vacances ; un minuteur qui a sonné pendant l'arrêt lance
  son automatisation au démarrage.
- Sur un serveur en marche, dans Chromium : interrupteur, curseur à 21,5, + / −, liste, bouton qui
  passe la maison en mode Nuit par automatisation, compte à rebours qui avance, pause figée, et
  automatisation lancée à la fin du minuteur. Aucune erreur, pas de débordement sur téléphone.
- **Redémarrage** : serveur arrêté pendant 12 s avec deux minuteurs en marche. Celui de 8 s a sonné
  au démarrage (et son automatisation a fonctionné) ; celui de 2 min a gardé son heure de fin.
