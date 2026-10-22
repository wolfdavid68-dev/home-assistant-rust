//! La boîte aux lettres des colis (étape 87) : l'adresse du serveur IMAP,
//! le compte, et le mot de passe d'application, chiffré dans la base.

use std::time::SystemTime;

use zeroize::Zeroizing;

use crate::{StorageError, Store, to_micros};

const PASSWORD_CONTEXT: &str = "mail_account.password";

/// Le compte de la boîte aux lettres.
#[derive(Clone)]
pub struct MailAccount {
    /// `imap.gmail.com`.
    pub host: String,
    /// 993 (IMAP sur TLS).
    pub port: u16,
    pub username: String,
    pub password: Zeroizing<String>,
}

impl std::fmt::Debug for MailAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MailAccount")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"<masqué>")
            .finish()
    }
}

impl Store {
    /// La boîte aux lettres, si elle est associée.
    pub async fn mail_account(&self) -> Result<Option<MailAccount>, StorageError> {
        let row: Option<(String, i64, String, String)> =
            sqlx::query_as("SELECT host, port, username, password FROM mail_account WHERE id = 1")
                .fetch_optional(self.pool())
                .await?;
        row.map(|(host, port, username, sealed)| {
            Ok(MailAccount {
                host,
                port: u16::try_from(port).unwrap_or(993),
                username,
                password: self.unseal(PASSWORD_CONTEXT, &sealed)?,
            })
        })
        .transpose()
    }

    /// Enregistre la boîte (en remplaçant la précédente).
    pub async fn save_mail_account(&self, account: &MailAccount) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO mail_account (id, host, port, username, password, paired_at)
             VALUES (1, ?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (id) DO UPDATE SET
                 host = excluded.host, port = excluded.port, username = excluded.username,
                 password = excluded.password, paired_at = excluded.paired_at",
        )
        .bind(&account.host)
        .bind(i64::from(account.port))
        .bind(&account.username)
        .bind(self.seal(PASSWORD_CONTEXT, &account.password)?)
        .bind(to_micros(SystemTime::now())?)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Oublie la boîte. Renvoie `false` s'il n'y en avait pas.
    pub async fn forget_mail_account(&self) -> Result<bool, StorageError> {
        let deleted = sqlx::query("DELETE FROM mail_account WHERE id = 1")
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
        assert!(store.mail_account().await.unwrap().is_none());
        let account = MailAccount {
            host: "imap.gmail.com".into(),
            port: 993,
            username: "maison@gmail.com".into(),
            password: Zeroizing::new("abcd efgh ijkl mnop".into()),
        };
        store.save_mail_account(&account).await.unwrap();
        let raw: String = sqlx::query_scalar("SELECT password FROM mail_account")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert!(crate::is_sealed(&raw), "jamais en clair dans la base");
        assert!(!raw.contains("efgh"));
        let read = store.mail_account().await.unwrap().unwrap();
        assert_eq!(read.username, "maison@gmail.com");
        assert_eq!(read.port, 993);
        assert_eq!(read.password.as_str(), "abcd efgh ijkl mnop");
        assert!(!format!("{read:?}").contains("efgh"));
        assert!(store.forget_mail_account().await.unwrap());
        assert!(!store.forget_mail_account().await.unwrap());
    }
}
