//! Authentification : comptes utilisateurs, mots de passe et jetons d'accès.
//!
//! - Les mots de passe sont hachés avec **Argon2id** ([`password`]).
//! - À la connexion, l'utilisateur reçoit un **jeton d'accès** aléatoire de
//!   256 bits ([`token`]) ; seule son empreinte SHA-256 est stockée.
//! - Les tentatives de connexion sont limitées par adresse IP ([`limiter`]).

mod error;
pub mod limiter;
pub mod password;
pub mod token;

use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ha_storage::{Store, to_micros};
use sqlx::SqlitePool;
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

pub use error::AuthError;
pub use limiter::LoginLimiter;
pub use password::{MAX_PASSWORD_LEN, MIN_PASSWORD_LEN};
pub use token::TOKEN_PREFIX;

/// Durée de validité d'un jeton par défaut : 30 jours.
pub const DEFAULT_TOKEN_LIFETIME: Duration = Duration::from_secs(30 * 24 * 3600);

/// Nombre de calculs Argon2 simultanés au maximum. Chacun utilise ~19 Mo de
/// mémoire : sans limite, une rafale de connexions pourrait saturer la machine.
const MAX_CONCURRENT_HASHES: usize = 2;

/// Un compte utilisateur (sans son mot de passe, évidemment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: i64,
    pub username: String,
}

/// Jeton remis à l'utilisateur après une connexion réussie.
///
/// Concept Rust : on n'implémente pas `Debug` automatiquement ici, pour qu'un
/// `{:?}` oublié dans un log ne puisse pas afficher le jeton.
pub struct IssuedToken {
    pub token: Zeroizing<String>,
    pub expires_at: SystemTime,
}

impl std::fmt::Debug for IssuedToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IssuedToken")
            .field("token", &"<masqué>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Le service d'authentification.
#[derive(Debug)]
pub struct Auth {
    pool: SqlitePool,
    limiter: LoginLimiter,
    /// Empreinte d'un mot de passe bidon : voir [`Auth::login`].
    dummy_hash: Arc<str>,
    hashing: Semaphore,
    token_lifetime: Duration,
}

impl Auth {
    /// Prépare le service sur la base `store` (tables créées par les migrations).
    pub async fn new(store: &Store) -> Result<Self, AuthError> {
        let dummy_hash = hash_blocking(Zeroizing::new("mot de passe bidon".to_owned())).await?;
        Ok(Self {
            pool: store.pool().clone(),
            limiter: LoginLimiter::new(),
            dummy_hash: dummy_hash.into(),
            hashing: Semaphore::new(MAX_CONCURRENT_HASHES),
            token_lifetime: DEFAULT_TOKEN_LIFETIME,
        })
    }

    /// Change la durée de validité des nouveaux jetons.
    pub fn with_token_lifetime(mut self, lifetime: Duration) -> Self {
        self.token_lifetime = lifetime;
        self
    }

    /// Crée un compte.
    pub async fn create_user(&self, username: &str, password: &str) -> Result<User, AuthError> {
        check_username(username)?;
        password::check_password_policy(password)?;
        let hash = self.hash(password).await?;

        let result = sqlx::query(
            "INSERT INTO users (username, password_hash, created_at) VALUES (?1, ?2, ?3)",
        )
        .bind(username)
        .bind(&hash)
        .bind(to_micros(SystemTime::now())?)
        .execute(&self.pool)
        .await;

        match result {
            Ok(done) => Ok(User {
                id: done.last_insert_rowid(),
                username: username.to_owned(),
            }),
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                Err(AuthError::UserExists)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Change le mot de passe d'un compte et **révoque tous ses jetons** :
    /// si le mot de passe a fuité, l'intrus est déconnecté partout.
    /// Renvoie `false` si le compte n'existe pas.
    pub async fn set_password(&self, username: &str, password: &str) -> Result<bool, AuthError> {
        password::check_password_policy(password)?;
        let hash = self.hash(password).await?;

        let mut tx = self.pool.begin().await?;
        let updated = sqlx::query("UPDATE users SET password_hash = ?1 WHERE username = ?2")
            .bind(&hash)
            .bind(username)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        sqlx::query(
            "DELETE FROM access_tokens WHERE user_id = (SELECT id FROM users WHERE username = ?1)",
        )
        .bind(username)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(updated > 0)
    }

    /// Supprime un compte et ses jetons. Renvoie `false` s'il n'existait pas.
    pub async fn delete_user(&self, username: &str) -> Result<bool, AuthError> {
        // Les jetons sont supprimés automatiquement (`ON DELETE CASCADE`).
        let deleted = sqlx::query("DELETE FROM users WHERE username = ?1")
            .bind(username)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(deleted > 0)
    }

    /// Tous les comptes, triés par nom.
    pub async fn list_users(&self) -> Result<Vec<User>, AuthError> {
        let rows: Vec<(i64, String)> =
            sqlx::query_as("SELECT id, username FROM users ORDER BY username")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows
            .into_iter()
            .map(|(id, username)| User { id, username })
            .collect())
    }

    /// Connexion par nom et mot de passe. Renvoie un nouveau jeton d'accès.
    ///
    /// `ip` est l'adresse du client, pour limiter les tentatives.
    pub async fn login(
        &self,
        username: &str,
        password: &str,
        ip: IpAddr,
    ) -> Result<IssuedToken, AuthError> {
        if let Err(retry_after) = self.limiter.check(ip) {
            return Err(AuthError::TooManyAttempts { retry_after });
        }

        let user: Option<(i64, String, String)> = if check_username(username).is_ok() {
            sqlx::query_as("SELECT id, username, password_hash FROM users WHERE username = ?1")
                .bind(username)
                .fetch_optional(&self.pool)
                .await?
        } else {
            None
        };

        // Sécurité : même si le compte n'existe pas, on fait un vrai calcul
        // Argon2 (sur une empreinte bidon). Sinon la réponse serait bien plus
        // rapide pour un nom inconnu, et un attaquant pourrait mesurer ce
        // temps pour découvrir quels comptes existent.
        let hash = user.as_ref().map_or_else(
            || Arc::clone(&self.dummy_hash),
            |(_, _, hash)| hash.as_str().into(),
        );
        let password_ok =
            password.chars().count() <= MAX_PASSWORD_LEN && self.verify(password, hash).await?;

        let Some((user_id, _, _)) = user.filter(|_| password_ok) else {
            self.limiter.record_failure(ip);
            tracing::warn!("échec de connexion depuis {ip}");
            return Err(AuthError::InvalidCredentials);
        };
        self.limiter.record_success(ip);
        self.issue_token(user_id).await
    }

    /// Retrouve l'utilisateur d'un jeton. `None` si le jeton est inconnu,
    /// révoqué ou expiré.
    pub async fn authenticate(&self, token: &str) -> Result<Option<User>, AuthError> {
        if !looks_like_token(token) {
            return Ok(None);
        }
        // Concept : on ne compare jamais le jeton reçu au jeton stocké (on ne
        // le stocke pas !). On cherche son empreinte SHA-256 dans la base. Le
        // temps de recherche ne dépend que de l'empreinte, qui ne révèle rien
        // du jeton : pas d'attaque possible par mesure du temps de réponse.
        let row: Option<(i64, String)> = sqlx::query_as(
            "SELECT users.id, users.username
             FROM access_tokens JOIN users ON users.id = access_tokens.user_id
             WHERE access_tokens.token_hash = ?1 AND access_tokens.expires_at > ?2",
        )
        .bind(token::hash_token(token).as_slice())
        .bind(to_micros(SystemTime::now())?)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(id, username)| User { id, username }))
    }

    /// Révoque un jeton (déconnexion). Renvoie `false` s'il n'existait pas.
    pub async fn revoke_token(&self, token: &str) -> Result<bool, AuthError> {
        let deleted = sqlx::query("DELETE FROM access_tokens WHERE token_hash = ?1")
            .bind(token::hash_token(token).as_slice())
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(deleted > 0)
    }

    /// Supprime les jetons expirés. Renvoie leur nombre.
    pub async fn purge_expired_tokens(&self) -> Result<u64, AuthError> {
        let deleted = sqlx::query("DELETE FROM access_tokens WHERE expires_at <= ?1")
            .bind(to_micros(SystemTime::now())?)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(deleted)
    }

    async fn issue_token(&self, user_id: i64) -> Result<IssuedToken, AuthError> {
        let token = token::generate_token()?;
        let now = SystemTime::now();
        let expires_at = now + self.token_lifetime;
        sqlx::query(
            "INSERT INTO access_tokens (user_id, token_hash, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(user_id)
        .bind(token::hash_token(&token).as_slice())
        .bind(to_micros(now)?)
        .bind(to_micros(expires_at)?)
        .execute(&self.pool)
        .await?;
        Ok(IssuedToken { token, expires_at })
    }

    async fn hash(&self, password: &str) -> Result<String, AuthError> {
        let _permit = self.permit().await?;
        hash_blocking(Zeroizing::new(password.to_owned())).await
    }

    async fn verify(&self, password: &str, hash: Arc<str>) -> Result<bool, AuthError> {
        let _permit = self.permit().await?;
        let password = Zeroizing::new(password.to_owned());
        tokio::task::spawn_blocking(move || password::verify_password(&password, &hash))
            .await
            .map_err(|error| AuthError::Internal(error.to_string()))
    }

    async fn permit(&self) -> Result<tokio::sync::SemaphorePermit<'_>, AuthError> {
        self.hashing
            .acquire()
            .await
            .map_err(|error| AuthError::Internal(error.to_string()))
    }
}

/// Concept : Argon2 occupe le processeur pendant des dizaines de
/// millisecondes. `spawn_blocking` l'exécute sur un fil dédié pour ne pas
/// bloquer les tâches `async` (l'API, l'enregistreur…) pendant ce temps.
async fn hash_blocking(password: Zeroizing<String>) -> Result<String, AuthError> {
    tokio::task::spawn_blocking(move || password::hash_password(&password))
        .await
        .map_err(|error| AuthError::Internal(error.to_string()))?
}

/// Nom d'utilisateur : 3 à 32 caractères parmi `a-z`, `0-9`, `_`, `-`, `.`.
fn check_username(username: &str) -> Result<(), AuthError> {
    let valid = (3..=32).contains(&username.len())
        && username
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-.".contains(&b));
    if valid {
        Ok(())
    } else {
        Err(AuthError::InvalidUsername)
    }
}

fn looks_like_token(token: &str) -> bool {
    token.len() == TOKEN_PREFIX.len() + 64
        && token.starts_with(TOKEN_PREFIX)
        && token[TOKEN_PREFIX.len()..]
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames() {
        for ok in ["david", "d.wolf", "admin_2", "abc"] {
            assert!(check_username(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "ab",
            "David",
            "a b",
            "admin'--",
            &"a".repeat(33),
            "é_admin",
        ] {
            assert!(check_username(bad).is_err(), "{bad:?}");
        }
    }
}
