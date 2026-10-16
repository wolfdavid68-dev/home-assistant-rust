//! Authentification : comptes utilisateurs, mots de passe et jetons d'accès.
//!
//! - Les mots de passe sont hachés avec **Argon2id** ([`password`]).
//! - À la connexion, l'utilisateur reçoit un **jeton d'accès** aléatoire de
//!   256 bits ([`token`]) ; seule son empreinte SHA-256 est stockée.
//! - Les tentatives de connexion sont limitées par adresse IP ([`limiter`]).
//! - Double authentification facultative par code à usage unique ([`totp`]).

mod error;
pub mod limiter;
pub mod password;
mod role;
pub mod token;
pub mod totp;

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
pub use role::{Permission, Role};
pub use token::TOKEN_PREFIX;

/// Usage du secret TOTP, pour son chiffrement dans la base.
const TOTP_CONTEXT: &str = "users.totp_secret";

/// Ligne de `users` lue à la connexion :
/// (id, empreinte du mot de passe, secret TOTP chiffré, dernier créneau TOTP).
type UserRow = (i64, String, Option<String>, Option<i64>);

/// Durée de validité d'un jeton par défaut : 30 jours.
pub const DEFAULT_TOKEN_LIFETIME: Duration = Duration::from_hours(30 * 24);

/// Nombre de calculs Argon2 simultanés au maximum. Chacun utilise ~19 Mo de
/// mémoire : sans limite, une rafale de connexions pourrait saturer la machine.
const MAX_CONCURRENT_HASHES: usize = 2;

/// Un compte utilisateur (sans son mot de passe, évidemment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub role: Role,
}

impl User {
    /// Ce compte a-t-il le droit de… ? (voir [`Role::can`])
    #[must_use]
    pub fn can(&self, permission: Permission) -> bool {
        self.role.can(permission)
    }
}

/// Lit le rôle rangé dans la base. Une valeur inconnue (base modifiée à la
/// main) donne le rôle le plus restreint : jamais plus de droits par erreur.
fn role_from_db(value: &str) -> Role {
    Role::parse(value).unwrap_or(Role::Guest)
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
    /// Pour chiffrer les secrets TOTP.
    store: Store,
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
            store: store.clone(),
            pool: store.pool().clone(),
            limiter: LoginLimiter::new(),
            dummy_hash: dummy_hash.into(),
            hashing: Semaphore::new(MAX_CONCURRENT_HASHES),
            token_lifetime: DEFAULT_TOKEN_LIFETIME,
        })
    }

    /// Change la durée de validité des nouveaux jetons.
    #[must_use]
    pub fn with_token_lifetime(mut self, lifetime: Duration) -> Self {
        self.token_lifetime = lifetime;
        self
    }

    /// Crée un compte **administrateur** (voir [`Auth::create_user_as`]).
    pub async fn create_user(&self, username: &str, password: &str) -> Result<User, AuthError> {
        self.create_user_as(username, password, Role::Admin).await
    }

    /// Crée un compte avec ce rôle.
    pub async fn create_user_as(
        &self,
        username: &str,
        password: &str,
        role: Role,
    ) -> Result<User, AuthError> {
        check_username(username)?;
        password::check_password_policy(password)?;
        let hash = self.hash(password).await?;

        let result = sqlx::query(
            "INSERT INTO users (username, password_hash, created_at, role) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(username)
        .bind(&hash)
        .bind(to_micros(SystemTime::now())?)
        .bind(role.as_str())
        .execute(&self.pool)
        .await;

        match result {
            Ok(done) => Ok(User {
                id: done.last_insert_rowid(),
                username: username.to_owned(),
                role,
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
    /// Le dernier administrateur ne peut pas être supprimé.
    pub async fn delete_user(&self, username: &str) -> Result<bool, AuthError> {
        let mut tx = self.pool.begin().await?;
        if is_last_admin(&mut tx, username).await? {
            return Err(AuthError::LastAdmin);
        }
        // Les jetons sont supprimés automatiquement (`ON DELETE CASCADE`).
        let deleted = sqlx::query("DELETE FROM users WHERE username = ?1")
            .bind(username)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        tx.commit().await?;
        Ok(deleted > 0)
    }

    /// Change le rôle d'un compte. Il s'applique tout de suite, même aux
    /// sessions ouvertes (le rôle est relu à chaque demande). Le dernier
    /// administrateur ne peut pas perdre son rôle. Renvoie `false` si le
    /// compte n'existe pas.
    pub async fn set_role(&self, username: &str, role: Role) -> Result<bool, AuthError> {
        let mut tx = self.pool.begin().await?;
        if role != Role::Admin && is_last_admin(&mut tx, username).await? {
            return Err(AuthError::LastAdmin);
        }
        let updated = sqlx::query("UPDATE users SET role = ?1 WHERE username = ?2")
            .bind(role.as_str())
            .bind(username)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        tx.commit().await?;
        Ok(updated > 0)
    }

    /// Tous les comptes, triés par nom.
    pub async fn list_users(&self) -> Result<Vec<User>, AuthError> {
        let rows: Vec<(i64, String, String)> =
            sqlx::query_as("SELECT id, username, role FROM users ORDER BY username")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows
            .into_iter()
            .map(|(id, username, role)| User {
                id,
                username,
                role: role_from_db(&role),
            })
            .collect())
    }

    /// Connexion par nom, mot de passe et, si la double authentification
    /// est activée pour ce compte, code à usage unique. Renvoie un nouveau
    /// jeton d'accès.
    ///
    /// `ip` est l'adresse du client, pour limiter les tentatives. Toutes les
    /// erreurs donnent le même message : un attaquant n'apprend pas si c'est
    /// le nom, le mot de passe ou le code qui est faux.
    pub async fn login(
        &self,
        username: &str,
        password: &str,
        code: Option<&str>,
        ip: IpAddr,
    ) -> Result<IssuedToken, AuthError> {
        let user_id = self.check_credentials(username, password, code, ip).await?;
        self.issue_token(user_id, self.token_lifetime).await
    }

    /// Un jeton pour un programme (un assistant IA branché par MCP, étape
    /// 55), sans mot de passe : seule la ligne de commande du serveur en
    /// crée. `lifetime` : 1 à 365 jours. `None` si le compte n'existe pas.
    pub async fn issue_program_token(
        &self,
        username: &str,
        lifetime: Duration,
    ) -> Result<Option<IssuedToken>, AuthError> {
        let lifetime = lifetime.clamp(Duration::from_hours(24), Duration::from_hours(365 * 24));
        let user_id: Option<i64> = sqlx::query_scalar("SELECT id FROM users WHERE username = ?1")
            .bind(username)
            .fetch_optional(&self.pool)
            .await?;
        match user_id {
            Some(user_id) => Ok(Some(self.issue_token(user_id, lifetime).await?)),
            None => Ok(None),
        }
    }

    /// Confirme l'identité d'un utilisateur **déjà connecté**, avant une
    /// action sensible (modifier une automatisation…) : mot de passe, et code
    /// à usage unique si la double authentification est activée.
    ///
    /// Mêmes protections qu'à la connexion (tentatives limitées par adresse,
    /// message d'erreur unique), mais aucun jeton n'est créé.
    pub async fn confirm(
        &self,
        username: &str,
        password: &str,
        code: Option<&str>,
        ip: IpAddr,
    ) -> Result<(), AuthError> {
        self.check_credentials(username, password, code, ip)
            .await
            .map(|_| ())
    }

    /// Vérifie nom, mot de passe et code. Renvoie l'identifiant du compte.
    async fn check_credentials(
        &self,
        username: &str,
        password: &str,
        code: Option<&str>,
        ip: IpAddr,
    ) -> Result<i64, AuthError> {
        if let Err(retry_after) = self.limiter.check(ip) {
            return Err(AuthError::TooManyAttempts { retry_after });
        }

        let user: Option<UserRow> = if check_username(username).is_ok() {
            sqlx::query_as(
                "SELECT id, password_hash, totp_secret, totp_last_step
                 FROM users WHERE username = ?1",
            )
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
            |(_, hash, _, _)| hash.as_str().into(),
        );
        let password_ok =
            password.chars().count() <= MAX_PASSWORD_LEN && self.verify(password, hash).await?;

        let Some((user_id, _, totp_secret, last_step)) = user.filter(|_| password_ok) else {
            self.limiter.record_failure(ip);
            tracing::warn!("échec de connexion depuis {ip}");
            return Err(AuthError::InvalidCredentials);
        };
        if let Some(sealed) = totp_secret {
            let code_ok = match code {
                Some(code) => self.check_totp(user_id, &sealed, code, last_step).await?,
                None => false,
            };
            if !code_ok {
                self.limiter.record_failure(ip);
                tracing::warn!("échec de connexion depuis {ip} (code de vérification)");
                return Err(AuthError::InvalidCredentials);
            }
        }
        self.limiter.record_success(ip);
        Ok(user_id)
    }

    /// Vérifie un code à usage unique et mémorise son créneau, pour qu'il ne
    /// puisse pas resservir.
    async fn check_totp(
        &self,
        user_id: i64,
        sealed: &str,
        code: &str,
        last_step: Option<i64>,
    ) -> Result<bool, AuthError> {
        let secret = match self.totp_secret(sealed) {
            Ok(secret) => secret,
            Err(error) => {
                tracing::error!(
                    "secret de double authentification illisible ({error}) : \
                     désactive-la avec « ha-server user totp-off <nom> »"
                );
                return Ok(false);
            }
        };
        let last_step = last_step.and_then(|step| u64::try_from(step).ok());
        let Some(step) = totp::verify(&secret, code, totp::step_at(SystemTime::now()), last_step)
        else {
            return Ok(false);
        };
        // La condition `totp_last_step < ?1` rend l'opération atomique : si
        // deux connexions arrivent en même temps avec le même code, une seule
        // modifie la ligne, donc une seule est acceptée.
        let updated = sqlx::query(
            "UPDATE users SET totp_last_step = ?1
             WHERE id = ?2 AND (totp_last_step IS NULL OR totp_last_step < ?1)",
        )
        .bind(i64::try_from(step).map_err(|_| AuthError::Internal("créneau".into()))?)
        .bind(user_id)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(updated == 1)
    }

    fn totp_secret(&self, sealed: &str) -> Result<Zeroizing<Vec<u8>>, AuthError> {
        let base32 = self.store.unseal(TOTP_CONTEXT, sealed)?;
        totp::base32_decode(&base32)
            .map(Zeroizing::new)
            .ok_or_else(|| AuthError::Internal("secret TOTP invalide".into()))
    }

    /// Active (ou remplace) la double authentification d'un compte.
    ///
    /// `code` est le code affiché par l'appli juste après avoir scanné le QR
    /// code : il prouve que l'appli a bien le secret, sinon on risquerait de
    /// s'enfermer dehors. Tous les jetons du compte sont révoqués : il faudra
    /// se reconnecter, avec un code. Renvoie `false` si le compte n'existe pas.
    pub async fn enable_totp(
        &self,
        username: &str,
        secret: &[u8],
        code: &str,
    ) -> Result<bool, AuthError> {
        let Some(step) = totp::verify(secret, code, totp::step_at(SystemTime::now()), None) else {
            return Err(AuthError::InvalidTotpCode);
        };
        let sealed = self
            .store
            .seal(TOTP_CONTEXT, &totp::base32_encode(secret))?;
        let mut tx = self.pool.begin().await?;
        let updated = sqlx::query(
            "UPDATE users SET totp_secret = ?1, totp_last_step = ?2 WHERE username = ?3",
        )
        .bind(&sealed)
        .bind(i64::try_from(step).map_err(|_| AuthError::Internal("créneau".into()))?)
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

    /// Désactive la double authentification d'un compte. Renvoie `false` si
    /// le compte n'existe pas.
    pub async fn disable_totp(&self, username: &str) -> Result<bool, AuthError> {
        let updated = sqlx::query(
            "UPDATE users SET totp_secret = NULL, totp_last_step = NULL WHERE username = ?1",
        )
        .bind(username)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(updated > 0)
    }

    /// La double authentification est-elle activée ? `None` si le compte
    /// n'existe pas.
    pub async fn has_totp(&self, username: &str) -> Result<Option<bool>, AuthError> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT totp_secret FROM users WHERE username = ?1")
                .bind(username)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(secret,)| secret.is_some()))
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
        let row: Option<(i64, String, String)> = sqlx::query_as(
            "SELECT users.id, users.username, users.role
             FROM access_tokens JOIN users ON users.id = access_tokens.user_id
             WHERE access_tokens.token_hash = ?1 AND access_tokens.expires_at > ?2",
        )
        .bind(token::hash_token(token).as_slice())
        .bind(to_micros(SystemTime::now())?)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(id, username, role)| User {
            id,
            username,
            role: role_from_db(&role),
        }))
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

    async fn issue_token(
        &self,
        user_id: i64,
        lifetime: Duration,
    ) -> Result<IssuedToken, AuthError> {
        let token = token::generate_token()?;
        let now = SystemTime::now();
        let expires_at = now + lifetime;
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

/// Ce compte est-il le seul administrateur ? (Dans une transaction, pour
/// qu'une autre modification ne puisse pas se glisser entre la vérification
/// et le changement.)
async fn is_last_admin(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    username: &str,
) -> Result<bool, AuthError> {
    let (is_admin, admins): (i64, i64) = sqlx::query_as(
        "SELECT
             (SELECT COUNT(*) FROM users WHERE username = ?1 AND role = 'admin'),
             (SELECT COUNT(*) FROM users WHERE role = 'admin')",
    )
    .bind(username)
    .fetch_one(&mut **tx)
    .await?;
    Ok(is_admin == 1 && admins == 1)
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
