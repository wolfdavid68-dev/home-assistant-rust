//! Le compte Iglou de la maison (étape 81) : le projet Firebase de l'appli,
//! l'adresse, le mot de passe (chiffré dans la base) et la maison suivie.

use std::time::SystemTime;

use zeroize::Zeroizing;

use crate::{StorageError, Store, to_micros};

/// Usage du mot de passe, pour son chiffrement.
const PASSWORD_CONTEXT: &str = "iglou_account.password";

/// Le compte.
#[derive(Clone)]
pub struct IglouAccount {
    /// Le projet Firebase de l'appli (`mon-projet-1a2b3`).
    pub project_id: String,
    /// Sa clé « web », publique.
    pub api_key: String,
    pub email: String,
    pub password: Zeroizing<String>,
    /// La maison Iglou dont la liste de courses est lue.
    pub maison_id: String,
}

impl std::fmt::Debug for IglouAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IglouAccount")
            .field("project_id", &self.project_id)
            .field("email", &self.email)
            .field("password", &"<masqué>")
            .field("maison_id", &self.maison_id)
            .finish_non_exhaustive()
    }
}

impl Store {
    /// Le compte Iglou, s'il y en a un.
    pub async fn iglou_account(&self) -> Result<Option<IglouAccount>, StorageError> {
        let row: Option<(String, String, String, String, String)> = sqlx::query_as(
            "SELECT project_id, api_key, email, password, maison_id FROM iglou_account WHERE id = 1",
        )
        .fetch_optional(self.pool())
        .await?;
        row.map(|(project_id, api_key, email, sealed, maison_id)| {
            Ok(IglouAccount {
                project_id,
                api_key,
                email,
                password: self.unseal(PASSWORD_CONTEXT, &sealed)?,
                maison_id,
            })
        })
        .transpose()
    }

    /// Enregistre le compte (en remplaçant le précédent).
    pub async fn save_iglou_account(&self, account: &IglouAccount) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO iglou_account (id, project_id, api_key, email, password, maison_id, paired_at)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (id) DO UPDATE SET
                 project_id = excluded.project_id, api_key = excluded.api_key,
                 email = excluded.email, password = excluded.password,
                 maison_id = excluded.maison_id, paired_at = excluded.paired_at",
        )
        .bind(&account.project_id)
        .bind(&account.api_key)
        .bind(&account.email)
        .bind(self.seal(PASSWORD_CONTEXT, &account.password)?)
        .bind(&account.maison_id)
        .bind(to_micros(SystemTime::now())?)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Oublie le compte. Renvoie `false` s'il n'y en avait pas.
    pub async fn forget_iglou_account(&self) -> Result<bool, StorageError> {
        let deleted = sqlx::query("DELETE FROM iglou_account WHERE id = 1")
            .execute(self.pool())
            .await?
            .rows_affected();
        Ok(deleted > 0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[tokio::test]
    async fn the_password_is_sealed() {
        let store = Store::open_in_memory().await.unwrap();
        assert!(store.iglou_account().await.unwrap().is_none());
        let account = IglouAccount {
            project_id: "projet-1a2b3".into(),
            api_key: "cle-web".into(),
            email: "maison@exemple.fr".into(),
            password: Zeroizing::new("un mot de passe".into()),
            maison_id: "m1".into(),
        };
        store.save_iglou_account(&account).await.unwrap();
        let raw: String = sqlx::query_scalar("SELECT password FROM iglou_account")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert!(crate::is_sealed(&raw), "jamais en clair dans la base");
        assert!(!raw.contains("mot de passe"));
        let read = store.iglou_account().await.unwrap().unwrap();
        assert_eq!(read.email, "maison@exemple.fr");
        assert_eq!(read.maison_id, "m1");
        assert_eq!(read.password.as_str(), "un mot de passe");
        assert!(!format!("{read:?}").contains("mot de passe"));
        assert!(store.forget_iglou_account().await.unwrap());
        assert!(!store.forget_iglou_account().await.unwrap());
    }
}
