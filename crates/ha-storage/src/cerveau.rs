//! Les préférences de la maison, lues par le cerveau avant de répondre
//! (docs/cerveau.md). L'API vérifie le texte avant de l'enregistrer.

use std::time::SystemTime;

use crate::error::StorageError;
use crate::store::{Store, from_micros, to_micros};

/// Le texte des préférences, et qui l'a écrit en dernier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CerveauPreferences {
    pub text: String,
    pub updated_at: SystemTime,
    pub updated_by: String,
}

impl Store {
    /// Les préférences, si elles ont été écrites.
    pub async fn cerveau_preferences(&self) -> Result<Option<CerveauPreferences>, StorageError> {
        let row: Option<(String, i64, String)> = sqlx::query_as(
            "SELECT text, updated_at, updated_by FROM cerveau_preferences WHERE id = 1",
        )
        .fetch_optional(self.pool())
        .await?;
        row.map(|(text, updated_at, updated_by)| {
            Ok(CerveauPreferences {
                text,
                updated_at: from_micros(updated_at)?,
                updated_by,
            })
        })
        .transpose()
    }

    /// Enregistre les préférences.
    pub async fn save_cerveau_preferences(
        &self,
        text: &str,
        updated_by: &str,
    ) -> Result<(), StorageError> {
        let now = to_micros(SystemTime::now())?;
        sqlx::query(
            "INSERT INTO cerveau_preferences (id, text, updated_at, updated_by) VALUES (1, ?1, ?2, ?3)
             ON CONFLICT (id) DO UPDATE SET
                 text = excluded.text, updated_at = excluded.updated_at,
                 updated_by = excluded.updated_by",
        )
        .bind(text)
        .bind(now)
        .bind(updated_by)
        .execute(self.pool())
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn preferences_are_saved_and_replaced() {
        let store = Store::open_in_memory().await.unwrap();
        assert_eq!(store.cerveau_preferences().await.unwrap(), None);
        store
            .save_cerveau_preferences("cosy = 30 %", "alice")
            .await
            .unwrap();
        store
            .save_cerveau_preferences("cosy = 20 %", "bob")
            .await
            .unwrap();
        let saved = store.cerveau_preferences().await.unwrap().unwrap();
        assert_eq!(saved.text, "cosy = 20 %");
        assert_eq!(saved.updated_by, "bob");
    }
}
