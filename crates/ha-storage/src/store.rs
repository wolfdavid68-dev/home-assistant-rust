//! Accès à la base SQLite : ouverture, écriture des changements, lecture.

use std::path::Path;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ha_core::{Attributes, EntityId, State, StateMachine};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{ConnectOptions, Connection, SqlitePool};
use tracing::warn;
use zeroize::Zeroizing;

use crate::error::StorageError;
use crate::secrets::SecretKey;

/// Nombre maximal de lignes renvoyées par [`Store::history`].
pub const MAX_HISTORY_LIMIT: u32 = 10_000;

/// Concept Rust : la macro `migrate!` lit les fichiers `.sql`
/// du dossier `migrations/` **à la compilation** et les intègre dans le
/// binaire. Pas de fichier à copier à côté du programme, et personne ne peut
/// les modifier après coup.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Un état relu depuis la base.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredState {
    pub entity_id: EntityId,
    pub state: String,
    pub attributes: Attributes,
    pub last_changed: SystemTime,
    pub last_updated: SystemTime,
}

/// Un point d'une courbe : moyenne, minimum et maximum d'une valeur
/// numérique sur une tranche de temps.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesPoint {
    /// Début de la tranche (premier relevé de la tranche).
    pub at: SystemTime,
    pub mean: f64,
    pub min: f64,
    pub max: f64,
}

/// Un changement de valeur. `state` vaut `None` si l'entité a été supprimée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateChange {
    pub at: SystemTime,
    pub state: Option<String>,
}

/// L'historique d'une entité sur une période, prêt à dessiner.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HistorySeries {
    /// Pour un capteur numérique : une courbe d'au plus `buckets` points.
    pub numeric: Vec<SeriesPoint>,
    /// Les changements de valeur (au plus [`MAX_SERIES_CHANGES`]), en
    /// commençant par la valeur au début de la période.
    pub changes: Vec<StateChange>,
}

/// Nombre maximal de changements renvoyés par [`Store::history_series`].
pub const MAX_SERIES_CHANGES: u32 = 2_000;
/// Nombre maximal de points d'une courbe.
pub const MAX_SERIES_BUCKETS: u32 = 1_000;

/// Une ligne de l'historique. `state` vaut `None` si l'entité a été supprimée.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    pub entity_id: EntityId,
    pub state: Option<StoredState>,
    pub recorded_at: SystemTime,
}

/// La base de données.
///
/// Concept Rust : `SqlitePool` est un « pool » de connexions, partagé en
/// interne avec un `Arc`. Cloner un `Store` est donc peu coûteux et donne une
/// deuxième poignée vers la même base.
#[derive(Debug, Clone)]
pub struct Store {
    pool: SqlitePool,
    secrets: SecretKey,
}

/// Ligne brute de la table `states` : (entity_id, state, attributes, last_changed, last_updated).
type StateRow = (String, String, String, i64, i64);

/// Ligne brute de la table `state_history`, avec les colonnes qui peuvent être NULL.
type HistoryRow = (
    String,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    i64,
);

impl Store {
    /// Ouvre (ou crée) la base dans le fichier `path` et met le schéma à jour.
    ///
    /// La clé qui chiffre les secrets est dans un fichier voisin, avec
    /// l'extension `.key` (`ha.db` → `ha.key`). Elle est créée si besoin.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref();
        Self::open_with_key(path, &path.with_extension("key")).await
    }

    /// Comme [`Store::open`], avec la clé de chiffrement dans `key_path`.
    pub async fn open_with_key(
        path: impl AsRef<Path>,
        key_path: &Path,
    ) -> Result<Self, StorageError> {
        let path = path.as_ref();
        let secrets = SecretKey::load_or_create(key_path)?;
        create_private_file(path)?;

        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            // WAL : les lectures ne bloquent pas les écritures, et un arrêt
            // brutal ne corrompt pas la base.
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            // Efface vraiment les données supprimées au lieu de les laisser
            // traîner dans le fichier.
            .pragma("secure_delete", "ON")
            .busy_timeout(Duration::from_secs(5));
        // Une première connexion, seule, crée le fichier et le passe en mode
        // WAL. Ce changement demande un accès exclusif : si plusieurs
        // connexions le tentaient en même temps sur une base neuve, l'une
        // d'elles pouvait échouer (« database is locked »). Le mode WAL reste
        // ensuite inscrit dans le fichier, et les connexions suivantes n'ont
        // plus rien à changer.
        options.clone().connect().await?.close().await?;
        Self::connect(options, 4, secrets).await
    }

    /// Base temporaire en mémoire, perdue à la fermeture. Utile pour les tests.
    pub async fn open_in_memory() -> Result<Self, StorageError> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
        // Chaque connexion à `:memory:` aurait sa propre base vide : on n'en
        // garde donc qu'une seule, ouverte en permanence.
        Self::connect(options, 1, SecretKey::random()?).await
    }

    async fn connect(
        options: SqliteConnectOptions,
        max_connections: u32,
        secrets: SecretKey,
    ) -> Result<Self, StorageError> {
        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .min_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            .connect_with(options)
            .await?;
        MIGRATOR.run(&pool).await?;
        Ok(Self { pool, secrets })
    }

    pub(crate) fn secrets(&self) -> &SecretKey {
        &self.secrets
    }

    /// Accès direct aux connexions, pour les crates qui ont leurs propres
    /// tables dans la même base (comptes et jetons dans `ha-auth`).
    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Chiffre un secret avant de le ranger dans la base. `context` dit à
    /// quoi il sert (par exemple `"somfy_box.token"`) : le secret ne pourra
    /// être déchiffré que pour ce même usage.
    pub fn seal(&self, context: &str, plaintext: &str) -> Result<String, StorageError> {
        self.secrets.seal(context, plaintext)
    }

    /// Déchiffre un secret lu dans la base (voir [`Store::seal`]).
    pub fn unseal(&self, context: &str, sealed: &str) -> Result<Zeroizing<String>, StorageError> {
        self.secrets.unseal(context, sealed)
    }

    /// Ferme proprement toutes les connexions.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Enregistre un changement d'état : met à jour le dernier état connu et
    /// ajoute une ligne à l'historique, dans une seule transaction.
    ///
    /// `new_state` vaut `None` quand l'entité a été supprimée.
    pub async fn record_change(
        &self,
        entity_id: &EntityId,
        new_state: Option<&State>,
    ) -> Result<(), StorageError> {
        let recorded_at = to_micros(SystemTime::now())?;

        // Concept : une transaction applique tout ou rien. Si une requête
        // échoue, `tx` est abandonnée sans `commit` et la base reste intacte.
        let mut tx = self.pool.begin().await?;
        if let Some(state) = new_state {
            let row = encode(state)?;
            // Sécurité : les valeurs passent TOUJOURS par des paramètres
            // (`?1`, `?2`…) et jamais par du texte collé dans la requête.
            // SQLite les traite comme des données, pas comme du SQL : une
            // valeur comme `'); DROP TABLE states; --` est stockée telle quelle.
            sqlx::query(
                "INSERT INTO states (entity_id, state, attributes, last_changed, last_updated)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (entity_id) DO UPDATE SET
                     state = excluded.state,
                     attributes = excluded.attributes,
                     last_changed = excluded.last_changed,
                     last_updated = excluded.last_updated",
            )
            .bind(&row.0)
            .bind(&row.1)
            .bind(&row.2)
            .bind(row.3)
            .bind(row.4)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO state_history
                     (entity_id, state, attributes, last_changed, last_updated, recorded_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .bind(&row.0)
            .bind(&row.1)
            .bind(&row.2)
            .bind(row.3)
            .bind(row.4)
            .bind(recorded_at)
            .execute(&mut *tx)
            .await?;
        } else {
            sqlx::query("DELETE FROM states WHERE entity_id = ?1")
                .bind(entity_id.as_str())
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO state_history (entity_id, recorded_at) VALUES (?1, ?2)")
                .bind(entity_id.as_str())
                .bind(recorded_at)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Remplace tout le contenu de la table `states` par `states`.
    ///
    /// Sert à resynchroniser la base quand des événements ont été manqués.
    /// L'historique n'est pas touché.
    pub async fn replace_all_states(&self, states: &[State]) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM states").execute(&mut *tx).await?;
        for state in states {
            let row = encode(state)?;
            sqlx::query(
                "INSERT INTO states (entity_id, state, attributes, last_changed, last_updated)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind(row.0)
            .bind(row.1)
            .bind(row.2)
            .bind(row.3)
            .bind(row.4)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Relit le dernier état connu de chaque entité, trié par identifiant.
    ///
    /// Une ligne illisible (fichier modifié à la main, par exemple) est ignorée
    /// avec un avertissement plutôt que d'empêcher le démarrage.
    pub async fn load_states(&self) -> Result<Vec<StoredState>, StorageError> {
        let rows: Vec<StateRow> = sqlx::query_as(
            "SELECT entity_id, state, attributes, last_changed, last_updated
             FROM states ORDER BY entity_id",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut states = Vec::with_capacity(rows.len());
        for (entity_id, state, attributes, last_changed, last_updated) in rows {
            match decode(&entity_id, state, &attributes, last_changed, last_updated) {
                Ok(stored) => states.push(stored),
                Err(error) => warn!("état sauvegardé ignoré pour {entity_id:?} : {error}"),
            }
        }
        Ok(states)
    }

    /// Recharge les états sauvegardés dans la machine à états, sans publier
    /// d'événement. Renvoie le nombre d'états restaurés.
    pub async fn restore_into(&self, machine: &StateMachine) -> Result<usize, StorageError> {
        let mut restored = 0;
        for stored in self.load_states().await? {
            let entity_id = stored.entity_id.clone();
            match machine.restore(
                stored.entity_id,
                stored.state,
                stored.attributes,
                stored.last_changed,
                stored.last_updated,
            ) {
                Ok(true) => restored += 1,
                Ok(false) => {}
                Err(error) => warn!("état sauvegardé ignoré pour {entity_id} : {error}"),
            }
        }
        Ok(restored)
    }

    /// Historique d'une entité, du plus récent au plus ancien.
    /// `limit` est plafonné à [`MAX_HISTORY_LIMIT`].
    pub async fn history(
        &self,
        entity_id: &EntityId,
        limit: u32,
    ) -> Result<Vec<HistoryEntry>, StorageError> {
        let rows: Vec<HistoryRow> = sqlx::query_as(
            "SELECT entity_id, state, attributes, last_changed, last_updated, recorded_at
             FROM state_history WHERE entity_id = ?1
             ORDER BY id DESC LIMIT ?2",
        )
        .bind(entity_id.as_str())
        .bind(limit.min(MAX_HISTORY_LIMIT))
        .fetch_all(&self.pool)
        .await?;

        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            match decode_history(row) {
                Ok(entry) => entries.push(entry),
                Err(error) => warn!("ligne d'historique ignorée pour {entity_id} : {error}"),
            }
        }
        Ok(entries)
    }

    /// L'historique de `entity_id` entre `since` et `until`, résumé pour un
    /// graphique : au plus `buckets` points pour une valeur numérique, et la
    /// liste des changements de valeur.
    ///
    /// Concept : le résumé est calculé **par SQLite** (`GROUP BY`, `LAG`),
    /// sans charger toutes les lignes en mémoire. Un capteur qui envoie une
    /// mesure par seconde produit 86 400 lignes par jour : on n'en renvoie
    /// que quelques centaines.
    pub async fn history_series(
        &self,
        entity_id: &EntityId,
        since: SystemTime,
        until: SystemTime,
        buckets: u32,
    ) -> Result<HistorySeries, StorageError> {
        let since = to_micros(since)?;
        let until = to_micros(until)?;
        if until <= since {
            return Ok(HistorySeries::default());
        }
        let buckets = i64::from(buckets.clamp(1, MAX_SERIES_BUCKETS));
        let width = ((until - since) / buckets).max(1);

        // Valeurs numériques uniquement (« 21.5 », « -3 ») : « unavailable »
        // ou « on » ne doivent pas devenir des zéros sur la courbe.
        let points: Vec<(i64, f64, f64, f64)> = sqlx::query_as(
            "SELECT MIN(recorded_at),
                    AVG(CAST(state AS REAL)), MIN(CAST(state AS REAL)), MAX(CAST(state AS REAL))
             FROM state_history
             WHERE entity_id = ?1 AND recorded_at >= ?2 AND recorded_at <= ?3
               AND state GLOB '*[0-9]*' AND trim(state, '0123456789.-+eE') = ''
             GROUP BY (recorded_at - ?2) / ?4
             ORDER BY 1",
        )
        .bind(entity_id.as_str())
        .bind(since)
        .bind(until)
        .bind(width)
        .fetch_all(&self.pool)
        .await?;

        // La valeur au début de la période (dernier relevé avant `since`)…
        let initial: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT state FROM state_history
             WHERE entity_id = ?1 AND recorded_at < ?2
             ORDER BY recorded_at DESC, id DESC LIMIT 1",
        )
        .bind(entity_id.as_str())
        .bind(since)
        .fetch_optional(&self.pool)
        .await?;
        // … puis seulement les vrais changements de valeur (pas les mises à
        // jour d'attributs) : `LAG` donne la valeur de la ligne précédente.
        let rows: Vec<(i64, Option<String>)> = sqlx::query_as(
            "SELECT recorded_at, state FROM (
                 SELECT recorded_at, state,
                        LAG(state, 1, char(1)) OVER (ORDER BY recorded_at, id) AS previous
                 FROM state_history
                 WHERE entity_id = ?1 AND recorded_at >= ?2 AND recorded_at <= ?3
             )
             WHERE previous IS NOT state
             ORDER BY recorded_at
             LIMIT ?4",
        )
        .bind(entity_id.as_str())
        .bind(since)
        .bind(until)
        .bind(MAX_SERIES_CHANGES)
        .fetch_all(&self.pool)
        .await?;

        let mut changes = Vec::with_capacity(rows.len() + 1);
        if let Some((state,)) = initial {
            changes.push(StateChange {
                at: from_micros(since)?,
                state,
            });
        }
        for (at, state) in rows {
            // Même valeur que celle du début de période : pas un changement.
            if changes.last().is_some_and(|last| last.state == state) {
                continue;
            }
            changes.push(StateChange {
                at: from_micros(at)?,
                state,
            });
        }
        let numeric = points
            .into_iter()
            .map(|(at, mean, min, max)| {
                Ok(SeriesPoint {
                    at: from_micros(at)?,
                    mean,
                    min,
                    max,
                })
            })
            .collect::<Result<_, StorageError>>()?;
        Ok(HistorySeries { numeric, changes })
    }

    /// Supprime l'historique enregistré avant `older_than`.
    /// Renvoie le nombre de lignes supprimées.
    pub async fn purge_history(&self, older_than: SystemTime) -> Result<u64, StorageError> {
        let result = sqlx::query("DELETE FROM state_history WHERE recorded_at < ?1")
            .bind(to_micros(older_than)?)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}

/// Convertit un `State` en valeurs prêtes à être écrites.
fn encode(state: &State) -> Result<StateRow, StorageError> {
    Ok((
        state.entity_id().as_str().to_owned(),
        state.state().to_owned(),
        serde_json::to_string(state.attributes())?,
        to_micros(state.last_changed())?,
        to_micros(state.last_updated())?,
    ))
}

/// Vérifie et convertit une ligne lue dans la base.
///
/// Sécurité : on ne fait pas confiance au contenu du fichier. L'identifiant
/// repasse par la validation d'`EntityId`, et les attributs doivent être un
/// objet JSON.
fn decode(
    entity_id: &str,
    state: String,
    attributes: &str,
    last_changed: i64,
    last_updated: i64,
) -> Result<StoredState, Box<dyn std::error::Error>> {
    let serde_json::Value::Object(attributes) = serde_json::from_str(attributes)? else {
        return Err("les attributs ne sont pas un objet JSON".into());
    };
    Ok(StoredState {
        entity_id: EntityId::new(entity_id)?,
        state,
        attributes,
        last_changed: from_micros(last_changed)?,
        last_updated: from_micros(last_updated)?,
    })
}

fn decode_history(row: HistoryRow) -> Result<HistoryEntry, Box<dyn std::error::Error>> {
    let (entity_id, state, attributes, last_changed, last_updated, recorded_at) = row;
    let state =
        match (state, attributes, last_changed, last_updated) {
            (Some(state), Some(attributes), Some(last_changed), Some(last_updated)) => Some(
                decode(&entity_id, state, &attributes, last_changed, last_updated)?,
            ),
            (None, None, None, None) => None,
            _ => return Err("ligne incomplète".into()),
        };
    Ok(HistoryEntry {
        entity_id: EntityId::new(&entity_id)?,
        state,
        recorded_at: from_micros(recorded_at)?,
    })
}

/// Date → microsecondes depuis 1970.
pub fn to_micros(time: SystemTime) -> Result<i64, StorageError> {
    let since_epoch = time
        .duration_since(UNIX_EPOCH)
        .map_err(|_| StorageError::Timestamp("date antérieure à 1970"))?;
    i64::try_from(since_epoch.as_micros())
        .map_err(|_| StorageError::Timestamp("date trop lointaine"))
}

/// Microsecondes depuis 1970 → date.
pub fn from_micros(micros: i64) -> Result<SystemTime, StorageError> {
    let micros = u64::try_from(micros).map_err(|_| StorageError::Timestamp("date négative"))?;
    UNIX_EPOCH
        .checked_add(Duration::from_micros(micros))
        .ok_or(StorageError::Timestamp("date trop lointaine"))
}

/// Crée le fichier de base vide, lisible uniquement par l'utilisateur courant.
///
/// Sécurité : l'historique révèle quand tu es chez toi, quand tu dors…
/// Sous Linux et macOS, on crée donc le fichier avec les droits `600`
/// (lecture/écriture pour le propriétaire seulement). SQLite donne ensuite les
/// mêmes droits à ses fichiers annexes (`-wal`, `-shm`). Sous Windows, le
/// fichier hérite des droits du dossier : place-le dans ton dossier utilisateur.
fn create_private_file(path: &Path) -> Result<(), StorageError> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(_) => Ok(()),
        // Le fichier existe déjà : on le garde tel quel.
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_round_trip_to_the_microsecond() {
        let time = UNIX_EPOCH + Duration::from_micros(1_700_000_000_123_456);
        assert_eq!(from_micros(to_micros(time).unwrap()).unwrap(), time);
    }

    #[test]
    fn invalid_timestamps_are_rejected() {
        assert!(from_micros(-1).is_err());
        assert!(to_micros(UNIX_EPOCH - Duration::from_secs(1)).is_err());
    }

    #[test]
    fn decode_rejects_bad_rows() {
        assert!(decode("pas un id", "on".into(), "{}", 0, 0).is_err());
        assert!(decode("light.salon", "on".into(), "[1, 2]", 0, 0).is_err());
        assert!(decode("light.salon", "on".into(), "{pas du json", 0, 0).is_err());
        assert!(decode("light.salon", "on".into(), "{}", 0, 0).is_ok());
    }
}
