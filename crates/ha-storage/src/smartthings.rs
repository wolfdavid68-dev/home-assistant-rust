//! La maison SmartThings (étape 86) : l'application créée pour la maison et
//! ses jetons, chiffrés dans la base.

use std::time::SystemTime;

use zeroize::Zeroizing;

use crate::{StorageError, Store, from_micros, to_micros};

const SECRET_CONTEXT: &str = "smartthings_account.client_secret";
const ACCESS_CONTEXT: &str = "smartthings_account.access_token";
const REFRESH_CONTEXT: &str = "smartthings_account.refresh_token";

/// Les jetons d'une connexion SmartThings.
#[derive(Clone)]
pub struct SmartThingsTokens {
    pub access_token: Zeroizing<String>,
    pub refresh_token: Zeroizing<String>,
    /// Fin de validité du jeton d'accès.
    pub expires_at: SystemTime,
}

/// L'application SmartThings de la maison, et ses jetons.
#[derive(Clone)]
pub struct SmartThingsAccount {
    pub client_id: String,
    pub client_secret: Zeroizing<String>,
    /// L'adresse de retour enregistrée dans l'application.
    pub redirect_uri: String,
    pub tokens: SmartThingsTokens,
}

impl std::fmt::Debug for SmartThingsAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmartThingsAccount")
            .field("client_id", &self.client_id)
            .field("client_secret", &"<masqué>")
            .field("redirect_uri", &self.redirect_uri)
            .field("tokens", &"<masqués>")
            .finish()
    }
}

impl std::fmt::Debug for SmartThingsTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmartThingsTokens")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

impl Store {
    /// L'application SmartThings, si la maison est associée.
    pub async fn smartthings_account(&self) -> Result<Option<SmartThingsAccount>, StorageError> {
        type Row = (String, String, String, String, String, i64);
        let row: Option<Row> = sqlx::query_as(
            "SELECT client_id, client_secret, redirect_uri, access_token, refresh_token, expires_at
             FROM smartthings_account WHERE id = 1",
        )
        .fetch_optional(self.pool())
        .await?;
        row.map(
            |(client_id, secret, redirect_uri, access, refresh, expires_at)| {
                Ok(SmartThingsAccount {
                    client_id,
                    client_secret: self.unseal(SECRET_CONTEXT, &secret)?,
                    redirect_uri,
                    tokens: SmartThingsTokens {
                        access_token: self.unseal(ACCESS_CONTEXT, &access)?,
                        refresh_token: self.unseal(REFRESH_CONTEXT, &refresh)?,
                        expires_at: from_micros(expires_at)?,
                    },
                })
            },
        )
        .transpose()
    }

    /// Enregistre l'association (en remplaçant la précédente).
    pub async fn save_smartthings_account(
        &self,
        account: &SmartThingsAccount,
    ) -> Result<(), StorageError> {
        let now = to_micros(SystemTime::now())?;
        sqlx::query(
            "INSERT INTO smartthings_account (id, client_id, client_secret, redirect_uri,
                 access_token, refresh_token, expires_at, refreshed_at, paired_at)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT (id) DO UPDATE SET
                 client_id = excluded.client_id, client_secret = excluded.client_secret,
                 redirect_uri = excluded.redirect_uri, access_token = excluded.access_token,
                 refresh_token = excluded.refresh_token, expires_at = excluded.expires_at,
                 refreshed_at = excluded.refreshed_at, paired_at = excluded.paired_at",
        )
        .bind(&account.client_id)
        .bind(self.seal(SECRET_CONTEXT, &account.client_secret)?)
        .bind(&account.redirect_uri)
        .bind(self.seal(ACCESS_CONTEXT, &account.tokens.access_token)?)
        .bind(self.seal(REFRESH_CONTEXT, &account.tokens.refresh_token)?)
        .bind(to_micros(account.tokens.expires_at)?)
        .bind(now)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Remplace les deux jetons ensemble, après un renouvellement. Renvoie
    /// `false` si la maison n'est plus associée.
    pub async fn update_smartthings_tokens(
        &self,
        tokens: &SmartThingsTokens,
    ) -> Result<bool, StorageError> {
        let updated = sqlx::query(
            "UPDATE smartthings_account SET access_token = ?1, refresh_token = ?2,
                 expires_at = ?3, refreshed_at = ?4 WHERE id = 1",
        )
        .bind(self.seal(ACCESS_CONTEXT, &tokens.access_token)?)
        .bind(self.seal(REFRESH_CONTEXT, &tokens.refresh_token)?)
        .bind(to_micros(tokens.expires_at)?)
        .bind(to_micros(SystemTime::now())?)
        .execute(self.pool())
        .await?
        .rows_affected();
        Ok(updated > 0)
    }

    /// Le dernier renouvellement des jetons, et l'association.
    pub async fn smartthings_dates(
        &self,
    ) -> Result<Option<(SystemTime, SystemTime)>, StorageError> {
        let row: Option<(i64, i64)> =
            sqlx::query_as("SELECT refreshed_at, paired_at FROM smartthings_account WHERE id = 1")
                .fetch_optional(self.pool())
                .await?;
        row.map(|(refreshed, paired)| Ok((from_micros(refreshed)?, from_micros(paired)?)))
            .transpose()
    }

    /// Oublie l'association. Renvoie `false` s'il n'y en avait pas.
    pub async fn forget_smartthings_account(&self) -> Result<bool, StorageError> {
        let deleted = sqlx::query("DELETE FROM smartthings_account WHERE id = 1")
            .execute(self.pool())
            .await?
            .rows_affected();
        Ok(deleted > 0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use std::time::Duration;

    use super::*;

    fn tokens(access: &str, refresh: &str) -> SmartThingsTokens {
        SmartThingsTokens {
            access_token: Zeroizing::new(access.into()),
            refresh_token: Zeroizing::new(refresh.into()),
            expires_at: SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000_000),
        }
    }

    #[tokio::test]
    async fn secrets_are_sealed_and_tokens_replaced_together() {
        let store = Store::open_in_memory().await.unwrap();
        assert!(store.smartthings_account().await.unwrap().is_none());
        assert!(
            !store
                .update_smartthings_tokens(&tokens("a", "r"))
                .await
                .unwrap()
        );
        let account = SmartThingsAccount {
            client_id: "client-123".into(),
            client_secret: Zeroizing::new("secret-du-client".into()),
            redirect_uri: "https://192.168.1.10:8123/smartthings".into(),
            tokens: tokens("acces-1", "renouvellement-1"),
        };
        store.save_smartthings_account(&account).await.unwrap();
        let raw: (String, String, String) = sqlx::query_as(
            "SELECT client_secret, access_token, refresh_token FROM smartthings_account",
        )
        .fetch_one(store.pool())
        .await
        .unwrap();
        for (sealed, clear) in [
            (&raw.0, "secret-du-client"),
            (&raw.1, "acces-1"),
            (&raw.2, "renouvellement-1"),
        ] {
            assert!(crate::is_sealed(sealed), "jamais en clair dans la base");
            assert!(!sealed.contains(clear));
        }
        assert!(
            store
                .update_smartthings_tokens(&tokens("acces-2", "renouvellement-2"))
                .await
                .unwrap()
        );
        let read = store.smartthings_account().await.unwrap().unwrap();
        assert_eq!(read.client_id, "client-123");
        assert_eq!(read.client_secret.as_str(), "secret-du-client");
        assert_eq!(read.tokens.access_token.as_str(), "acces-2");
        assert_eq!(read.tokens.refresh_token.as_str(), "renouvellement-2");
        assert_eq!(read.tokens.expires_at, account.tokens.expires_at);
        let shown = format!("{read:?}");
        assert!(
            !shown.contains("secret-du-client") && !shown.contains("acces-2"),
            "{shown}"
        );
        assert!(store.smartthings_dates().await.unwrap().is_some());
        assert!(store.forget_smartthings_account().await.unwrap());
        assert!(!store.forget_smartthings_account().await.unwrap());
    }
}
