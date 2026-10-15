//! Le dessin de la maison (style « plan d'architecte » du tableau de bord) :
//! des images, et la façon de les assembler.
//!
//! Le stockage range ce qu'on lui donne : c'est l'API qui vérifie que les
//! images sont bien des PNG, JPEG ou WebP, et l'assemblage, **avant** de les
//! enregistrer.

use std::time::SystemTime;

use crate::error::StorageError;
use crate::store::{Store, from_micros, to_micros};

/// Une image rangée dans la base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeImage {
    pub content_type: String,
    pub data: Vec<u8>,
}

/// Ce qu'on sait d'une image sans la lire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeImageInfo {
    pub name: String,
    pub content_type: String,
    pub size: u64,
    pub updated_at: SystemTime,
}

impl Store {
    /// La liste des images, par nom.
    pub async fn home_images(&self) -> Result<Vec<HomeImageInfo>, StorageError> {
        let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
            "SELECT name, content_type, length(data), updated_at FROM home_images ORDER BY name",
        )
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|(name, content_type, size, updated_at)| {
                Ok(HomeImageInfo {
                    name,
                    content_type,
                    size: u64::try_from(size).unwrap_or_default(),
                    updated_at: from_micros(updated_at)?,
                })
            })
            .collect()
    }

    /// Une image.
    pub async fn home_image(&self, name: &str) -> Result<Option<HomeImage>, StorageError> {
        let row: Option<(String, Vec<u8>)> =
            sqlx::query_as("SELECT content_type, data FROM home_images WHERE name = ?1")
                .bind(name)
                .fetch_optional(self.pool())
                .await?;
        Ok(row.map(|(content_type, data)| HomeImage { content_type, data }))
    }

    /// Ajoute ou remplace une image.
    pub async fn save_home_image(
        &self,
        name: &str,
        content_type: &str,
        data: &[u8],
        updated_by: &str,
    ) -> Result<(), StorageError> {
        let now = to_micros(SystemTime::now())?;
        sqlx::query(
            "INSERT INTO home_images (name, content_type, data, updated_at, updated_by)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (name) DO UPDATE SET
                 content_type = excluded.content_type, data = excluded.data,
                 updated_at = excluded.updated_at, updated_by = excluded.updated_by",
        )
        .bind(name)
        .bind(content_type)
        .bind(data)
        .bind(now)
        .bind(updated_by)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Retire une image. Renvoie `false` si elle n'existait pas.
    pub async fn delete_home_image(&self, name: &str) -> Result<bool, StorageError> {
        let done = sqlx::query("DELETE FROM home_images WHERE name = ?1")
            .bind(name)
            .execute(self.pool())
            .await?;
        Ok(done.rows_affected() > 0)
    }

    /// L'assemblage du dessin (JSON), s'il a été réglé.
    pub async fn home_drawing(&self) -> Result<Option<String>, StorageError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT config FROM home_drawing WHERE id = 1")
            .fetch_optional(self.pool())
            .await?;
        Ok(row.map(|(config,)| config))
    }

    /// Enregistre l'assemblage du dessin.
    pub async fn save_home_drawing(
        &self,
        config: &str,
        updated_by: &str,
    ) -> Result<(), StorageError> {
        let now = to_micros(SystemTime::now())?;
        sqlx::query(
            "INSERT INTO home_drawing (id, config, updated_at, updated_by) VALUES (1, ?1, ?2, ?3)
             ON CONFLICT (id) DO UPDATE SET
                 config = excluded.config, updated_at = excluded.updated_at,
                 updated_by = excluded.updated_by",
        )
        .bind(config)
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
    async fn images_are_saved_replaced_and_removed() {
        let store = Store::open_in_memory().await.unwrap();
        assert_eq!(store.home_images().await.unwrap(), Vec::new());

        store
            .save_home_image("jour", "image/png", b"premier", "alice")
            .await
            .unwrap();
        store
            .save_home_image("jour", "image/webp", b"second", "alice")
            .await
            .unwrap();
        let image = store.home_image("jour").await.unwrap().unwrap();
        assert_eq!(image.content_type, "image/webp");
        assert_eq!(image.data, b"second");
        let list = store.home_images().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].size, 6);

        assert!(store.delete_home_image("jour").await.unwrap());
        assert!(!store.delete_home_image("jour").await.unwrap());
        assert_eq!(store.home_image("jour").await.unwrap(), None);
    }

    #[tokio::test]
    async fn the_drawing_is_one_row() {
        let store = Store::open_in_memory().await.unwrap();
        assert_eq!(store.home_drawing().await.unwrap(), None);
        store.save_home_drawing("{\"a\":1}", "alice").await.unwrap();
        store.save_home_drawing("{\"a\":2}", "alice").await.unwrap();
        assert_eq!(
            store.home_drawing().await.unwrap().as_deref(),
            Some("{\"a\":2}")
        );
    }
}
