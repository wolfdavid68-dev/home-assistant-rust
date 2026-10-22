//! Automatisations créées depuis le tableau de bord, et leurs versions
//! précédentes.
//!
//! Le stockage ne comprend pas ce qu'il range : il garde un texte JSON par
//! automatisation. C'est `ha-automation` qui le vérifie, **avant** de
//! l'enregistrer et à chaque relecture.

use std::time::SystemTime;

use crate::error::StorageError;
use crate::store::{Store, from_micros, to_micros};

/// Nombre de versions gardées par automatisation.
pub const MAX_VERSIONS: i64 = 20;
/// Nombre total de versions gardées, toutes automatisations confondues.
const MAX_ALL_VERSIONS: i64 = 5_000;

/// Une automatisation rangée dans la base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAutomation {
    pub id: String,
    /// Configuration en JSON.
    pub config: String,
    pub updated_at: SystemTime,
    pub updated_by: String,
}

/// Une version d'une automatisation. `config` vaut `None` pour une suppression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationVersion {
    pub version: i64,
    pub config: Option<String>,
    pub changed_at: SystemTime,
    pub changed_by: String,
}

impl Store {
    /// Toutes les automatisations, dans l'ordre de leur création.
    pub async fn automations(&self) -> Result<Vec<StoredAutomation>, StorageError> {
        let rows: Vec<(String, String, i64, String)> = sqlx::query_as(
            "SELECT id, config, updated_at, updated_by FROM automations ORDER BY position, id",
        )
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|(id, config, updated_at, updated_by)| {
                Ok(StoredAutomation {
                    id,
                    config,
                    updated_at: from_micros(updated_at)?,
                    updated_by,
                })
            })
            .collect()
    }

    /// Crée ou remplace une automatisation, et garde la nouvelle version
    /// dans l'historique. Tout est fait dans une seule transaction.
    pub async fn save_automation(
        &self,
        id: &str,
        config: &str,
        actor: &str,
    ) -> Result<(), StorageError> {
        let now = to_micros(SystemTime::now())?;
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        // Concept SQL : « INSERT … ON CONFLICT DO UPDATE » (un « upsert ») crée
        // la ligne, ou la modifie si l'identifiant existe déjà. La position
        // d'origine est gardée : l'automatisation ne change pas de place.
        sqlx::query(
            "INSERT INTO automations (id, position, config, updated_at, updated_by)
             VALUES (?1, (SELECT COALESCE(MAX(position), 0) + 1 FROM automations), ?2, ?3, ?4)
             ON CONFLICT (id) DO UPDATE SET
                 config = excluded.config,
                 updated_at = excluded.updated_at,
                 updated_by = excluded.updated_by",
        )
        .bind(id)
        .bind(config)
        .bind(now)
        .bind(actor)
        .execute(&mut *tx)
        .await?;
        add_version(&mut tx, id, Some(config), now, actor).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Supprime une automatisation. Ses versions sont gardées : elle pourra
    /// être recréée. Renvoie `false` si elle n'existait pas.
    pub async fn delete_automation(&self, id: &str, actor: &str) -> Result<bool, StorageError> {
        let now = to_micros(SystemTime::now())?;
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let deleted = sqlx::query("DELETE FROM automations WHERE id = ?1")
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if deleted > 0 {
            add_version(&mut tx, id, None, now, actor).await?;
        }
        tx.commit().await?;
        Ok(deleted > 0)
    }

    /// La place des blocs dans le schéma, pour chaque automatisation qui en a
    /// une : (identifiant, JSON).
    pub async fn automation_layouts(&self) -> Result<Vec<(String, String)>, StorageError> {
        Ok(
            sqlx::query_as("SELECT automation_id, layout FROM automation_layouts")
                .fetch_all(self.pool())
                .await?,
        )
    }

    /// Enregistre la place des blocs (sans nouvelle version de
    /// l'automatisation : rien ne change dans ce qu'elle fait).
    pub async fn save_automation_layout(&self, id: &str, layout: &str) -> Result<(), StorageError> {
        let now = to_micros(SystemTime::now())?;
        sqlx::query(
            "INSERT INTO automation_layouts (automation_id, layout, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (automation_id) DO UPDATE SET
                 layout = excluded.layout, updated_at = excluded.updated_at",
        )
        .bind(id)
        .bind(layout)
        .bind(now)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Les réglages des automatisations toutes prêtes (étape 85) :
    /// (nom, JSON).
    pub async fn automation_presets(&self) -> Result<Vec<(String, String)>, StorageError> {
        Ok(
            sqlx::query_as("SELECT name, options FROM automation_presets ORDER BY name")
                .fetch_all(self.pool())
                .await?,
        )
    }

    /// Enregistre les réglages d'une automatisation toute prête.
    pub async fn save_automation_preset(
        &self,
        name: &str,
        options: &str,
        actor: &str,
    ) -> Result<(), StorageError> {
        let now = to_micros(SystemTime::now())?;
        sqlx::query(
            "INSERT INTO automation_presets (name, options, updated_at, updated_by)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (name) DO UPDATE SET
                 options = excluded.options, updated_at = excluded.updated_at,
                 updated_by = excluded.updated_by",
        )
        .bind(name)
        .bind(options)
        .bind(now)
        .bind(actor)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Les versions d'une automatisation, de la plus récente à la plus ancienne.
    pub async fn automation_versions(
        &self,
        id: &str,
    ) -> Result<Vec<AutomationVersion>, StorageError> {
        let rows: Vec<(i64, Option<String>, i64, String)> = sqlx::query_as(
            "SELECT version, config, changed_at, changed_by FROM automation_versions
             WHERE automation_id = ?1 ORDER BY version DESC LIMIT ?2",
        )
        .bind(id)
        .bind(MAX_VERSIONS)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|(version, config, changed_at, changed_by)| {
                Ok(AutomationVersion {
                    version,
                    config,
                    changed_at: from_micros(changed_at)?,
                    changed_by,
                })
            })
            .collect()
    }
}

/// Ajoute une version, puis oublie les plus anciennes.
async fn add_version(
    tx: &mut sqlx::SqliteConnection,
    id: &str,
    config: Option<&str>,
    now: i64,
    actor: &str,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO automation_versions (automation_id, config, changed_at, changed_by)
         VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(id)
    .bind(config)
    .bind(now)
    .bind(actor)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM automation_versions WHERE automation_id = ?1 AND version NOT IN
             (SELECT version FROM automation_versions WHERE automation_id = ?1
              ORDER BY version DESC LIMIT ?2)",
    )
    .bind(id)
    .bind(MAX_VERSIONS)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM automation_versions WHERE version NOT IN
             (SELECT version FROM automation_versions ORDER BY version DESC LIMIT ?1)",
    )
    .bind(MAX_ALL_VERSIONS)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn preset_options_are_kept() {
        let store = Store::open_in_memory().await.unwrap();
        assert_eq!(
            store.automation_presets().await.unwrap(),
            Vec::<(String, String)>::new()
        );
        store
            .save_automation_preset("nuit", r#"{"heure":"23:00"}"#, "david")
            .await
            .unwrap();
        store
            .save_automation_preset("depart", "{}", "david")
            .await
            .unwrap();
        store
            .save_automation_preset("nuit", r#"{"heure":"22:30"}"#, "alice")
            .await
            .unwrap();
        assert_eq!(
            store.automation_presets().await.unwrap(),
            [
                ("depart".to_owned(), "{}".to_owned()),
                ("nuit".to_owned(), r#"{"heure":"22:30"}"#.to_owned())
            ]
        );
    }

    #[tokio::test]
    async fn save_update_delete_and_versions() {
        let store = Store::open_in_memory().await.unwrap();
        assert_eq!(store.automations().await.unwrap(), Vec::new());

        store
            .save_automation("b", r#"{"id":"b"}"#, "david")
            .await
            .unwrap();
        store
            .save_automation("a", r#"{"id":"a"}"#, "david")
            .await
            .unwrap();
        store
            .save_automation("b", r#"{"id":"b","alias":"B"}"#, "alice")
            .await
            .unwrap();
        let all = store.automations().await.unwrap();
        let ids: Vec<&str> = all.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["b", "a"], "l'ordre de création est gardé");
        assert_eq!(all[0].config, r#"{"id":"b","alias":"B"}"#);
        assert_eq!(all[0].updated_by, "alice");

        assert!(store.delete_automation("b", "david").await.unwrap());
        assert!(!store.delete_automation("b", "david").await.unwrap());
        assert_eq!(store.automations().await.unwrap().len(), 1);

        let versions = store.automation_versions("b").await.unwrap();
        let configs: Vec<Option<&str>> = versions.iter().map(|v| v.config.as_deref()).collect();
        assert_eq!(
            configs,
            [
                None,
                Some(r#"{"id":"b","alias":"B"}"#),
                Some(r#"{"id":"b"}"#)
            ]
        );

        // Une automatisation recréée reprend une place à la fin.
        store
            .save_automation("b", r#"{"id":"b"}"#, "david")
            .await
            .unwrap();
        let ids: Vec<String> = store
            .automations()
            .await
            .unwrap()
            .into_iter()
            .map(|a| a.id)
            .collect();
        assert_eq!(ids, ["a", "b"]);
    }

    #[tokio::test]
    async fn old_versions_are_forgotten() {
        let store = Store::open_in_memory().await.unwrap();
        for n in 0..30 {
            store
                .save_automation("a", &format!("{{\"n\":{n}}}"), "david")
                .await
                .unwrap();
        }
        let versions = store.automation_versions("a").await.unwrap();
        assert_eq!(versions.len(), 20);
        assert_eq!(versions[0].config.as_deref(), Some("{\"n\":29}"));
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM automation_versions")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(count, MAX_VERSIONS);
    }
}
