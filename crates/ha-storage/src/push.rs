//! Notifications sur le téléphone, par le navigateur (Web Push) : la clé du
//! serveur, et les abonnements des appareils.
//!
//! Le stockage range ce qu'on lui donne : c'est `ha-push` qui vérifie les
//! adresses et les clés **avant** de les enregistrer.

use std::time::SystemTime;

use zeroize::Zeroizing;

use crate::error::StorageError;
use crate::store::{Store, from_micros, to_micros};

/// Contexte de chiffrement de la clé privée VAPID.
const VAPID_CONTEXT: &str = "push-vapid";
/// Contexte de chiffrement du secret « auth » d'un abonnement.
const AUTH_CONTEXT: &str = "push-auth";

/// La paire de clés VAPID du serveur.
pub struct VapidKeys {
    /// Clé privée PKCS#8, en hexadécimal.
    pub private_pkcs8_hex: Zeroizing<String>,
    /// Clé publique, en base64url.
    pub public_key: String,
}

impl std::fmt::Debug for VapidKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VapidKeys")
            .field("private_pkcs8_hex", &"<masquée>")
            .field("public_key", &self.public_key)
            .finish()
    }
}

/// Un nouvel abonnement, déjà vérifié.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPushSubscription<'a> {
    pub user_id: i64,
    pub endpoint: &'a str,
    pub p256dh: &'a str,
    pub auth: &'a str,
    pub name: &'a str,
    pub categories: u8,
}

/// Ce qu'on montre d'un abonnement à son propriétaire (jamais les clés).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushSubscriptionInfo {
    pub id: i64,
    pub endpoint: String,
    pub name: String,
    pub categories: u8,
    pub created_at: SystemTime,
    pub last_success: Option<SystemTime>,
    pub failures: u32,
}

/// Un appareil à qui envoyer une notification.
pub struct PushTarget {
    pub id: i64,
    pub user_id: i64,
    pub username: String,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: Zeroizing<String>,
}

impl std::fmt::Debug for PushTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PushTarget")
            .field("id", &self.id)
            .field("username", &self.username)
            .field("auth", &"<masqué>")
            .finish_non_exhaustive()
    }
}

/// Une ligne de `push_subscriptions`, telle que lue par la base.
type SubscriptionRow = (i64, String, String, i64, i64, Option<i64>, i64);

impl Store {
    /// La paire de clés VAPID, si elle existe déjà.
    pub async fn push_vapid(&self) -> Result<Option<VapidKeys>, StorageError> {
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT private_key, public_key FROM push_vapid WHERE id = 1")
                .fetch_optional(self.pool())
                .await?;
        row.map(|(sealed, public_key)| {
            Ok(VapidKeys {
                private_pkcs8_hex: self.unseal(VAPID_CONTEXT, &sealed)?,
                public_key,
            })
        })
        .transpose()
    }

    /// La clé publique VAPID seule (ce que l'API donne aux navigateurs).
    pub async fn push_public_key(&self) -> Result<Option<String>, StorageError> {
        Ok(
            sqlx::query_scalar("SELECT public_key FROM push_vapid WHERE id = 1")
                .fetch_optional(self.pool())
                .await?,
        )
    }

    /// Range une nouvelle paire de clés. Ne remplace jamais une paire
    /// existante : elle invaliderait tous les abonnements. Renvoie `false`
    /// si une paire existait déjà.
    pub async fn save_push_vapid(
        &self,
        private_pkcs8_hex: &str,
        public_key: &str,
    ) -> Result<bool, StorageError> {
        let sealed = self.seal(VAPID_CONTEXT, private_pkcs8_hex)?;
        let now = to_micros(SystemTime::now())?;
        let done = sqlx::query(
            "INSERT INTO push_vapid (id, private_key, public_key, created_at)
             VALUES (1, ?1, ?2, ?3) ON CONFLICT (id) DO NOTHING",
        )
        .bind(sealed)
        .bind(public_key)
        .bind(now)
        .execute(self.pool())
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Ajoute un abonnement, ou met à jour celui de ce navigateur (même
    /// adresse) : il passe alors au compte qui vient de s'abonner. Renvoie
    /// son numéro.
    pub async fn save_push_subscription(
        &self,
        subscription: &NewPushSubscription<'_>,
    ) -> Result<i64, StorageError> {
        let auth = self.seal(AUTH_CONTEXT, subscription.auth)?;
        let now = to_micros(SystemTime::now())?;
        let id = sqlx::query_scalar(
            "INSERT INTO push_subscriptions
                 (user_id, endpoint, p256dh, auth, name, categories, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (endpoint) DO UPDATE SET
                 user_id = excluded.user_id, p256dh = excluded.p256dh,
                 auth = excluded.auth, name = excluded.name,
                 categories = excluded.categories, failures = 0
             RETURNING id",
        )
        .bind(subscription.user_id)
        .bind(subscription.endpoint)
        .bind(subscription.p256dh)
        .bind(auth)
        .bind(subscription.name)
        .bind(i64::from(subscription.categories))
        .bind(now)
        .fetch_one(self.pool())
        .await?;
        Ok(id)
    }

    /// Nombre d'abonnements d'un compte, sans compter `except_endpoint`
    /// (le navigateur qui se réabonne ne compte pas deux fois).
    pub async fn count_push_subscriptions(
        &self,
        user_id: i64,
        except_endpoint: &str,
    ) -> Result<u64, StorageError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM push_subscriptions WHERE user_id = ?1 AND endpoint != ?2",
        )
        .bind(user_id)
        .bind(except_endpoint)
        .fetch_one(self.pool())
        .await?;
        Ok(u64::try_from(count).unwrap_or_default())
    }

    /// Nombre total d'abonnements.
    pub async fn count_all_push_subscriptions(&self) -> Result<u64, StorageError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM push_subscriptions")
            .fetch_one(self.pool())
            .await?;
        Ok(u64::try_from(count).unwrap_or_default())
    }

    /// Les appareils d'un compte.
    pub async fn push_subscriptions_of(
        &self,
        user_id: i64,
    ) -> Result<Vec<PushSubscriptionInfo>, StorageError> {
        let rows: Vec<SubscriptionRow> = sqlx::query_as(
            "SELECT id, endpoint, name, categories, created_at, last_success, failures
             FROM push_subscriptions WHERE user_id = ?1 ORDER BY id",
        )
        .bind(user_id)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(
                |(id, endpoint, name, categories, created_at, last_success, failures)| {
                    Ok(PushSubscriptionInfo {
                        id,
                        endpoint,
                        name,
                        categories: u8::try_from(categories).unwrap_or_default(),
                        created_at: from_micros(created_at)?,
                        last_success: last_success.map(from_micros).transpose()?,
                        failures: u32::try_from(failures).unwrap_or(u32::MAX),
                    })
                },
            )
            .collect()
    }

    /// Change ce qu'un appareil reçoit. `false` si ce n'est pas un appareil
    /// de ce compte.
    pub async fn set_push_categories(
        &self,
        user_id: i64,
        id: i64,
        categories: u8,
    ) -> Result<bool, StorageError> {
        let done = sqlx::query(
            "UPDATE push_subscriptions SET categories = ?3 WHERE id = ?1 AND user_id = ?2",
        )
        .bind(id)
        .bind(user_id)
        .bind(i64::from(categories))
        .execute(self.pool())
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Retire un appareil de ce compte. `false` s'il n'existe pas (ou
    /// appartient à quelqu'un d'autre).
    pub async fn delete_push_subscription(
        &self,
        user_id: i64,
        id: i64,
    ) -> Result<bool, StorageError> {
        let done = sqlx::query("DELETE FROM push_subscriptions WHERE id = ?1 AND user_id = ?2")
            .bind(id)
            .bind(user_id)
            .execute(self.pool())
            .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Oublie un abonnement que le service de notifications dit expiré.
    pub async fn forget_push_subscription(&self, id: i64) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM push_subscriptions WHERE id = ?1")
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Les appareils qui veulent recevoir `category`, des comptes famille et
    /// administrateur seulement (un compte devenu invité ne reçoit plus
    /// rien). `user_id`, s'il est donné, limite à ce compte.
    pub async fn push_targets(
        &self,
        category: u8,
        user_id: Option<i64>,
    ) -> Result<Vec<PushTarget>, StorageError> {
        let rows: Vec<(i64, i64, String, String, String, String)> = sqlx::query_as(
            "SELECT s.id, u.id, u.username, s.endpoint, s.p256dh, s.auth
             FROM push_subscriptions s JOIN users u ON u.id = s.user_id
             WHERE (s.categories & ?1) != 0 AND u.role IN ('admin', 'family')
               AND (?2 IS NULL OR s.user_id = ?2)
             ORDER BY s.id",
        )
        .bind(i64::from(category))
        .bind(user_id)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|(id, user_id, username, endpoint, p256dh, auth)| {
                Ok(PushTarget {
                    id,
                    user_id,
                    username,
                    endpoint,
                    p256dh,
                    auth: self.unseal(AUTH_CONTEXT, &auth)?,
                })
            })
            .collect()
    }

    /// Note le résultat d'un envoi.
    pub async fn push_delivered(&self, id: i64, ok: bool) -> Result<(), StorageError> {
        if ok {
            sqlx::query(
                "UPDATE push_subscriptions SET last_success = ?2, failures = 0 WHERE id = ?1",
            )
            .bind(id)
            .bind(to_micros(SystemTime::now())?)
            .execute(self.pool())
            .await?;
        } else {
            sqlx::query("UPDATE push_subscriptions SET failures = failures + 1 WHERE id = ?1")
                .bind(id)
                .execute(self.pool())
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn user(store: &Store, name: &str, role: &str) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO users (username, password_hash, created_at, role)
             VALUES (?1, 'x', 0, ?2) RETURNING id",
        )
        .bind(name)
        .bind(role)
        .fetch_one(store.pool())
        .await
        .unwrap()
    }

    fn subscription(user_id: i64, endpoint: &str, categories: u8) -> NewPushSubscription<'_> {
        NewPushSubscription {
            user_id,
            endpoint,
            p256dh: "BPUB",
            auth: "secret-auth",
            name: "Téléphone",
            categories,
        }
    }

    #[tokio::test]
    async fn the_vapid_key_is_sealed_and_never_replaced() {
        let store = Store::open_in_memory().await.unwrap();
        assert!(store.push_vapid().await.unwrap().is_none());
        assert!(store.save_push_vapid("a1b2", "PUBLIC").await.unwrap());
        assert!(!store.save_push_vapid("ffff", "AUTRE").await.unwrap());
        let keys = store.push_vapid().await.unwrap().unwrap();
        assert_eq!(keys.private_pkcs8_hex.as_str(), "a1b2");
        assert_eq!(keys.public_key, "PUBLIC");
        assert!(!format!("{keys:?}").contains("a1b2"));
        let raw: String = sqlx::query_scalar("SELECT private_key FROM push_vapid")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert!(raw.starts_with("enc:v1:"), "chiffrée en base");
    }

    #[tokio::test]
    async fn subscriptions_follow_their_owner() {
        let store = Store::open_in_memory().await.unwrap();
        let david = user(&store, "david", "admin").await;
        let alice = user(&store, "alice", "family").await;
        let invite = user(&store, "invite", "guest").await;
        let a = store
            .save_push_subscription(&subscription(david, "https://fcm/a", 0b011))
            .await
            .unwrap();
        store
            .save_push_subscription(&subscription(alice, "https://fcm/b", 0b111))
            .await
            .unwrap();
        store
            .save_push_subscription(&subscription(invite, "https://fcm/c", 0b111))
            .await
            .unwrap();
        // Le même navigateur se réabonne : même ligne.
        let again = store
            .save_push_subscription(&subscription(david, "https://fcm/a", 0b001))
            .await
            .unwrap();
        assert_eq!(a, again);
        assert_eq!(store.count_push_subscriptions(david, "").await.unwrap(), 1);
        assert_eq!(
            store
                .count_push_subscriptions(david, "https://fcm/a")
                .await
                .unwrap(),
            0
        );

        let names = |targets: Vec<PushTarget>| -> Vec<String> {
            targets.into_iter().map(|t| t.username).collect()
        };
        // L'invité ne reçoit jamais rien.
        assert_eq!(
            names(store.push_targets(1, None).await.unwrap()),
            ["david", "alice"]
        );
        assert_eq!(names(store.push_targets(4, None).await.unwrap()), ["alice"]);
        assert_eq!(
            names(store.push_targets(1, Some(alice)).await.unwrap()),
            ["alice"]
        );
        let target = store.push_targets(1, Some(david)).await.unwrap().remove(0);
        assert_eq!(target.auth.as_str(), "secret-auth");
        assert_eq!(target.user_id, david);

        // Personne ne touche aux appareils des autres.
        assert!(!store.set_push_categories(alice, a, 7).await.unwrap());
        assert!(!store.delete_push_subscription(alice, a).await.unwrap());
        assert!(store.set_push_categories(david, a, 4).await.unwrap());

        store.push_delivered(a, false).await.unwrap();
        store.push_delivered(a, false).await.unwrap();
        let info = store.push_subscriptions_of(david).await.unwrap();
        assert_eq!(info[0].failures, 2);
        assert_eq!(info[0].categories, 4);
        store.push_delivered(a, true).await.unwrap();
        let info = store.push_subscriptions_of(david).await.unwrap();
        assert_eq!(info[0].failures, 0);
        assert!(info[0].last_success.is_some());

        // Un compte supprimé emporte ses appareils.
        sqlx::query("DELETE FROM users WHERE id = ?1")
            .bind(alice)
            .execute(store.pool())
            .await
            .unwrap();
        assert_eq!(store.count_all_push_subscriptions().await.unwrap(), 2);
        store.forget_push_subscription(a).await.unwrap();
        assert!(store.delete_push_subscription(david, a).await.is_ok());
        assert_eq!(store.count_all_push_subscriptions().await.unwrap(), 1);
    }
}
