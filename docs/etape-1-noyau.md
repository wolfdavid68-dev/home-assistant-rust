# Étape 1 : le noyau, expliqué

## Ce que fait le noyau

```
 appareil / simulation                         abonnés
        │                                ┌──► affichage des logs
        ▼                                │
  StateMachine.set("light.salon", "on")  ├──► (étape 2) historique SQLite
        │                                │
        │ publie Event::StateChanged     ├──► (étape 5) automatisations
        ▼                                │
     EventBus ───────────────────────────┘
```

1. Un appareil (ici simulé) annonce son état à la **machine à états** : `light.salon` vaut `on`.
2. La machine à états compare avec l'état précédent. Si rien n'a changé, elle ne fait rien.
3. Sinon, elle enregistre le nouvel état et publie un événement `StateChanged` sur le **bus**.
4. Tous les abonnés reçoivent l'événement, chacun de son côté.

C'est exactement le principe de Home Assistant : tout passe par le bus, et les fonctionnalités
s'y branchent sans se connaître entre elles.

## Les fichiers

| Fichier | Contenu |
|---------|---------|
| [entity_id.rs](../crates/ha-core/src/entity_id.rs) | `EntityId` : un identifiant `domaine.nom` forcément valide |
| [state.rs](../crates/ha-core/src/state.rs) | `State` : valeur, attributs, dates de changement |
| [event.rs](../crates/ha-core/src/event.rs) | `Event` et `EventBus` |
| [state_machine.rs](../crates/ha-core/src/state_machine.rs) | `StateMachine` : les états actuels, protégés par un verrou |
| [error.rs](../crates/ha-core/src/error.rs) | `CoreError` : les erreurs possibles |
| [tests/state_machine.rs](../crates/ha-core/tests/state_machine.rs) | Les tests |
| [ha-server/main.rs](../crates/ha-server/src/main.rs) | La démo |

## Les concepts Rust rencontrés

**Propriété et emprunt (`&`).** En Rust, chaque valeur a un seul propriétaire. Une fonction qui
reçoit `&EntityId` *emprunte* l'identifiant pour le lire, sans le prendre. Une fonction qui reçoit
`EntityId` (sans `&`) en devient propriétaire. C'est pour cela qu'on voit parfois `.clone()` :
on fait une copie quand on a besoin de garder l'original. Ce système remplace le ramasse-miettes
(garbage collector) et empêche à la compilation toute une famille de failles mémoire.

**`Result` et `?`.** Une fonction qui peut échouer renvoie `Result<T, Erreur>`. L'opérateur `?`
veut dire « si c'est une erreur, la renvoyer tout de suite à l'appelant ». Pas d'exception cachée :
on voit dans la signature qu'une fonction peut échouer.

**`Option` au lieu de `null`.** `get` renvoie `Option<State>` : `Some(état)` ou `None`. Impossible
d'oublier le cas « l'entité n'existe pas », le compilateur refuse.

**Newtype.** `EntityId` enveloppe une `String`, mais on ne peut le créer qu'en passant par la
validation. Une fois qu'on a un `EntityId`, il est valide : c'est une règle de sécurité appliquée
par le compilateur.

**Champs privés.** Les champs de `State` ne sont pas `pub` : le reste du programme peut lire un
état, mais seule la machine à états peut le modifier.

**`enum` avec données.** `Event::StateChanged { old_state, new_state, .. }` porte ses propres
données. Quand on ajoutera d'autres événements, chaque `match` qui les oublie ne compilera plus.

**Traits.** `Display`, `FromStr`, `Clone`, `Debug`… sont des comportements qu'un type peut
implémenter. `#[derive(...)]` les génère automatiquement.

**Concurrence sûre : `Arc` et `RwLock`.** Plusieurs tâches partagent le noyau. `Arc` permet le
partage (compteur de références), `RwLock` empêche deux écritures simultanées. Si on oublie le
verrou, **le programme ne compile pas** : c'est la garantie « fearless concurrency » de Rust.

**`async` / `.await` avec tokio.** La démo lance plusieurs tâches (`tokio::spawn`) qui attendent
sans bloquer : l'abonné attend un événement, la simulation attend la seconde suivante.
`tokio::select!` attend « Ctrl+C ou la fin de la simulation, le premier qui arrive ».

## Choix de sécurité de cette étape

- Identifiants validés (caractères autorisés, longueur maximale 255).
- Valeur d'état limitée à 255 caractères : une entité ne peut pas remplir la mémoire.
- Aucun `unwrap()` hors des tests : les erreurs remontent proprement.
- Un abonné lent ne bloque jamais le système : il est prévenu qu'il a manqué des événements.

## Essayer

```bash
cargo run -p ha-server
```

```bash
cargo test
```
