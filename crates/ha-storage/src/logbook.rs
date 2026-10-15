//! Le journal de bord : ce qui s'est passé dans la maison, en phrases.
//!
//! Les phrases sont écrites par la tâche du journal (crate `ha-logbook`) ;
//! ici, on les range et on les relit.

use std::time::SystemTime;

use ha_core::EntityId;

use crate::audit::clean_field;
use crate::error::StorageError;
use crate::store::{Store, from_micros, to_micros};

/// Nombre maximal de lignes renvoyées d'un coup.
pub const MAX_LOGBOOK_LIMIT: u32 = 2000;

/// Une ligne du journal de bord.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogbookEntry {
    pub id: i64,
    pub at: SystemTime,
    pub entity_id: Option<String>,
    pub name: String,
    pub message: String,
    pub cause: Option<String>,
}

/// Une ligne à écrire.
#[derive(Debug, Clone, Copy)]
pub struct NewLogbookEntry<'a> {
    pub entity_id: Option<&'a EntityId>,
    pub name: &'a str,
    pub message: &'a str,
    pub cause: Option<&'a str>,
}

type LogbookRow = (i64, i64, Option<String>, String, String, Option<String>);

impl Store {
    /// Ajoute une ligne, datée de maintenant. Les textes sont nettoyés
    /// (caractères de contrôle remplacés, 200 caractères au plus) : un nom
    /// d'appareil venu de l'extérieur ne peut pas abîmer l'affichage.
    pub async fn add_logbook(&self, entry: NewLogbookEntry<'_>) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO logbook (at, entity_id, name, message, cause)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(to_micros(SystemTime::now())?)
        .bind(entry.entity_id.map(EntityId::as_str))
        .bind(clean_field(entry.name))
        .bind(clean_field(entry.message))
        .bind(entry.cause.map(clean_field))
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Les lignes entre `since` et `until`, de la plus récente à la plus
    /// ancienne, pour une entité ou pour toutes.
    pub async fn logbook(
        &self,
        since: SystemTime,
        until: SystemTime,
        entity_id: Option<&EntityId>,
        limit: u32,
    ) -> Result<Vec<LogbookEntry>, StorageError> {
        let rows: Vec<LogbookRow> = sqlx::query_as(
            "SELECT id, at, entity_id, name, message, cause FROM logbook
             WHERE at >= ?1 AND at <= ?2 AND (?3 IS NULL OR entity_id = ?3)
             ORDER BY at DESC, id DESC LIMIT ?4",
        )
        .bind(to_micros(since)?)
        .bind(to_micros(until)?)
        .bind(entity_id.map(EntityId::as_str))
        .bind(limit.min(MAX_LOGBOOK_LIMIT))
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|(id, at, entity_id, name, message, cause)| {
                Ok(LogbookEntry {
                    id,
                    at: from_micros(at)?,
                    entity_id,
                    name,
                    message,
                    cause,
                })
            })
            .collect()
    }

    /// Supprime les lignes plus anciennes que `older_than`. Renvoie leur nombre.
    pub async fn purge_logbook(&self, older_than: SystemTime) -> Result<u64, StorageError> {
        let deleted = sqlx::query("DELETE FROM logbook WHERE at < ?1")
            .bind(to_micros(older_than)?)
            .execute(self.pool())
            .await?
            .rows_affected();
        Ok(deleted)
    }
}
