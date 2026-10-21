//! Le compte TimeTree de la maison (étape 74) : l'adresse et le mot de
//! passe, chiffré dans la base.

use std::time::SystemTime;

use zeroize::Zeroizing;

use crate::{StorageError, Store, to_micros};

/// Usage du mot de passe, pour son chiffrement.
const PASSWORD_CONTEXT: &str = "timetree_account.password";

/// Le compte : l'adresse e-mail et le mot de passe (jamais affiché).
#[derive(Clone)]
pub struct TimeTreeAccount {
    pub email: String,
    pub password: Zeroizing<String>,
}

impl std::fmt::Debug for TimeTreeAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimeTreeAccount")
            .field("email", &self.email)
            .field("password", &"<masqué>")
            .finish()
    }
}

impl Store {
    /// Le compte TimeTree, s'il y en a un.
    pub async fn timetree_account(&self) -> Result<Option<TimeTreeAccount>, StorageError> {
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT email, password FROM timetree_account WHERE id = 1")
                .fetch_optional(self.pool())
                .await?;
        row.map(|(email, sealed)| {
            Ok(TimeTreeAccount {
                email,
                password: self.unseal(PASSWORD_CONTEXT, &sealed)?,
            })
        })
        .transpose()
    }

    /// Enregistre le compte (en remplaçant le précédent).
    pub async fn save_timetree_account(
        &self,
        account: &TimeTreeAccount,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO timetree_account (id, email, password, paired_at) VALUES (1, ?1, ?2, ?3)
             ON CONFLICT (id) DO UPDATE SET
                 email = excluded.email, password = excluded.password,
                 paired_at = excluded.paired_at",
        )
        .bind(&account.email)
        .bind(self.seal(PASSWORD_CONTEXT, &account.password)?)
        .bind(to_micros(SystemTime::now())?)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Oublie le compte. Renvoie `false` s'il n'y en avait pas.
    pub async fn forget_timetree_account(&self) -> Result<bool, StorageError> {
        let deleted = sqlx::query("DELETE FROM timetree_account WHERE id = 1")
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
        assert!(store.timetree_account().await.unwrap().is_none());
        let account = TimeTreeAccount {
            email: "maison@exemple.fr".into(),
            password: Zeroizing::new("un mot de passe".into()),
        };
        store.save_timetree_account(&account).await.unwrap();
        let raw: String = sqlx::query_scalar("SELECT password FROM timetree_account")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert!(crate::is_sealed(&raw), "jamais en clair dans la base");
        assert!(!raw.contains("mot de passe"));
        let read = store.timetree_account().await.unwrap().unwrap();
        assert_eq!(read.email, "maison@exemple.fr");
        assert_eq!(read.password.as_str(), "un mot de passe");
        assert!(!format!("{read:?}").contains("mot de passe"));
        assert!(store.forget_timetree_account().await.unwrap());
        assert!(!store.forget_timetree_account().await.unwrap());
    }
}
