# Étape 2 : le stockage SQLite, expliqué

## Ce que fait le stockage

```
  StateMachine.set(...)                     ha.db (SQLite)
        │                                ┌──────────────────────────┐
        │ publie Event::StateChanged     │ states        (1 ligne   │
        ▼                                │   par entité : le dernier│
     EventBus ──► Recorder ── écrit ───► │   état connu)            │
                                         │ state_history (1 ligne   │
                                         │   par changement)        │
                                         └──────────────────────────┘
  Au démarrage :  ha.db ── Store::restore_into ──► StateMachine
```

1. Le **Recorder** (l'enregistreur) est un abonné du bus comme un autre. Pour chaque
   `StateChanged`, il met à jour la table `states` et ajoute une ligne dans `state_history`,
   dans une **transaction** (tout ou rien). Les changements déjà en attente (une rafale de
   capteurs) partent ensemble, jusqu'à 256 par transaction, dans le même ordre.
2. Au démarrage, `Store::restore_into` relit la table `states` et remet chaque entité dans la
   machine à états, avec ses dates d'origine. Aucun événement n'est publié : rien n'a vraiment
   changé, on se souvient simplement de ce qu'on savait avant l'arrêt.
3. Au démarrage, puis une fois par jour, l'historique plus vieux que `HA_HISTORY_DAYS` jours
   (10 par défaut) est supprimé, pour que la base ne grossisse pas sans fin, même si le serveur
   reste allumé des mois. Après la purge quotidienne, `PRAGMA optimize` laisse SQLite mettre à
   jour ses statistiques sur les index.
4. À l'arrêt (Ctrl+C), l'enregistreur écrit les derniers événements en attente avant de fermer
   la base.

Comme pour l'étape 1, le noyau ne sait pas que le stockage existe : il publie sur le bus, et
l'enregistreur s'y branche. C'est le même principe que le « recorder » de Home Assistant.

## Les fichiers

| Fichier | Contenu |
|---------|---------|
| [migrations/0001_states_and_history.sql](../crates/ha-storage/migrations/0001_states_and_history.sql) | Le schéma : tables `states` et `state_history` |
| [store.rs](../crates/ha-storage/src/store.rs) | `Store` : ouverture, écriture, lecture, purge |
| [recorder.rs](../crates/ha-storage/src/recorder.rs) | `Recorder` : la tâche abonnée au bus |
| [error.rs](../crates/ha-storage/src/error.rs) | `StorageError` : les erreurs possibles |
| [tests/storage.rs](../crates/ha-storage/tests/storage.rs) | Les tests (base en mémoire ou fichier temporaire) |
| [state_machine.rs](../crates/ha-core/src/state_machine.rs) | Nouvelle méthode `restore` |
| [ha-server/main.rs](../crates/ha-server/src/main.rs) | La démo, avec stockage |

## Le schéma de la base

- `states` : une ligne par entité (`entity_id` est la clé primaire), avec la valeur, les attributs
  en JSON et les deux dates.
- `state_history` : une ligne par changement. Quand une entité est supprimée, on écrit une ligne
  dont `state` vaut `NULL`.

Les dates sont stockées en **microsecondes depuis le 1er janvier 1970** (un simple entier) : c'est
compact, facile à comparer et indépendant des fuseaux horaires. Les tables sont déclarées `STRICT` :
SQLite refuse alors un texte dans une colonne `INTEGER` (par défaut, il l'accepterait).

**Migrations.** Le schéma est décrit dans des fichiers `.sql` numérotés. Au démarrage, `sqlx`
regarde lesquels ont déjà été appliqués (il le note dans une table `_sqlx_migrations`) et applique
les nouveaux. Pour changer le schéma plus tard, on n'édite **jamais** un fichier existant : on en
ajoute un nouveau (`0002_...sql`). `sqlx` vérifie d'ailleurs que les anciens n'ont pas été modifiés.

## Les concepts Rust rencontrés

**`async` et la base de données.** Lire ou écrire sur le disque prend du temps. Avec `.await`,
la tâche de l'enregistreur se met en pause pendant l'écriture, et les autres tâches (la simulation,
l'affichage) continuent pendant ce temps.

**Le « pool » de connexions.** `SqlitePool` garde quelques connexions ouvertes et les prête à qui en
a besoin. Il est partagé en interne avec un `Arc` : cloner un `Store` donne une deuxième poignée
vers la même base, pas une deuxième base.

**Conversion d'erreurs avec `#[from]` et `?`.** `StorageError` déclare
`Database(#[from] sqlx::Error)`. Grâce à ça, dans une fonction qui renvoie
`Result<_, StorageError>`, `requete.await?` convertit automatiquement une erreur `sqlx` en
`StorageError::Database`.

**Les transactions.** `self.pool.begin()` démarre une transaction, `tx.commit()` la valide. Si une
requête échoue entre les deux, l'opérateur `?` fait sortir de la fonction, `tx` est détruite sans
`commit`, et SQLite annule tout. Rust garantit qu'on ne peut pas oublier ce nettoyage : c'est le
principe du « drop » (destruction automatique en fin de portée).

**Les macros qui travaillent à la compilation.** `sqlx::migrate!("./migrations")` lit les fichiers
SQL **pendant la compilation** et les met dans le programme. Il n'y a aucun fichier à livrer à côté,
et personne ne peut les modifier ensuite.

**Les tuples comme lignes.** `query_as::<_, (String, String, String, i64, i64)>` lit chaque ligne
comme un tuple : le type de chaque colonne est vérifié à la lecture. Une colonne qui peut être
`NULL` se lit en `Option<String>` : le compilateur oblige à traiter le cas vide.

**Arrêt propre.** `Recorder::stop` envoie un signal d'arrêt (un `CancellationToken`, voir
[la modernisation](modernisation.md#2-un-seul-outil-pour-arrêter-les-tâches-de-fond)), puis
attend la fin de la tâche. Dans la boucle, `tokio::select!` attend « un événement **ou** l'ordre
d'arrêt ». Après l'ordre d'arrêt, on vide les événements restants avec `try_recv`.

**`#[cfg(unix)]`.** Ce bloc n'est compilé que sous Linux/macOS. Il sert à régler les droits du
fichier, une notion qui n'existe pas de la même façon sous Windows.

## Choix de sécurité de cette étape

- **Requêtes paramétrées uniquement.** Toutes les valeurs passent par `.bind(...)` (`?1`, `?2`…),
  jamais par du texte collé dans la requête. SQLite les traite comme des données, pas comme du
  SQL : la valeur `'); DROP TABLE states; --` est stockée telle quelle (un test le vérifie).
  En prime, `sqlx` 0.9 **refuse de compiler** une requête construite avec `format!` : seules les
  chaînes écrites en dur dans le code sont acceptées (sauf à le contourner explicitement avec
  `AssertSqlSafe`, ce qu'on s'interdit). Une injection SQL ne peut donc pas arriver par simple
  inattention.
- **Fichier privé.** La base révèle tes habitudes (quand tu rentres, quand tu dors). Sous
  Linux/macOS, elle est créée avec les droits `600` : seul ton utilisateur peut la lire.
  Sous Windows, garde-la dans ton dossier utilisateur. Les fichiers `*.db` sont ignorés par Git :
  la base ne peut pas partir sur GitHub par accident.
- **Effacement réel.** L'option SQLite `secure_delete` écrase les données supprimées (purge de
  l'historique) au lieu de les laisser traîner dans le fichier.
- **On ne fait pas confiance au fichier.** Ce qui est relu de la base repasse par les mêmes
  validations que le reste (`EntityId::new`, longueur de l'état, attributs en objet JSON). Une ligne
  invalide est ignorée avec un avertissement, sans empêcher le démarrage.
- **Durée de conservation limitée.** L'historique est purgé après 10 jours par défaut
  (`HA_HISTORY_DAYS`, entre 1 et 3650). Garder moins de données, c'est moins de données à voler.
- **Pas de dépendance superflue.** `sqlx` est compilé avec SQLite seulement : sans MySQL (qui tire
  la crate `rsa`, concernée par une faille connue) ni PostgreSQL. `cargo deny` le vérifie.
- **Robustesse.** Une erreur d'écriture (disque plein…) est signalée dans les logs mais n'arrête
  pas le système. Si l'enregistreur prend du retard et perd des événements, il réécrit le dernier
  état de toutes les entités pour que la restauration reste juste.
- Toujours aucun `unwrap()` hors des tests et aucun code `unsafe`.

## Essayer

```bash
cargo run -p ha-server
```

Laisse tourner quelques secondes, arrête avec Ctrl+C, puis relance : les premières lignes
affichent les états restaurés depuis `ha.db`.

Autre fichier ou autre durée de conservation (PowerShell) :

```powershell
$env:HA_DB_PATH = "C:\Users\moi\domotique\ha.db"; $env:HA_HISTORY_DAYS = "30"; cargo run -p ha-server
```

Pour regarder dans la base, n'importe quel outil SQLite convient (par exemple
[DB Browser for SQLite](https://sqlitebrowser.org/)) :

```sql
SELECT entity_id, state, datetime(last_changed / 1000000, 'unixepoch') FROM states;
```

```bash
cargo test
```
