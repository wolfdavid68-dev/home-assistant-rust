//! Tests de l'authentification, sur une base SQLite en mémoire.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use ha_auth::limiter::MAX_FAILURES;
use ha_auth::{Auth, AuthError};
use ha_storage::Store;

const IP: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const PASSWORD: &str = "cafe chaud sur la terrasse";

async fn setup() -> Auth {
    let store = Store::open_in_memory().await.unwrap();
    let auth = Auth::new(&store).await.unwrap();
    auth.create_user("david", PASSWORD).await.unwrap();
    auth
}

#[tokio::test]
async fn login_gives_a_working_token() {
    let auth = setup().await;

    let issued = auth.login("david", PASSWORD, None, IP).await.unwrap();
    let user = auth.authenticate(&issued.token).await.unwrap().unwrap();

    assert_eq!(user.username, "david");
    // Le jeton ne doit jamais apparaître dans un affichage de débogage.
    assert!(!format!("{issued:?}").contains(issued.token.as_str()));
}

#[tokio::test]
async fn wrong_password_and_unknown_user_give_the_same_error() {
    let auth = setup().await;

    let wrong = auth
        .login("david", "pas le bon mot de passe", None, IP)
        .await;
    let unknown = auth.login("inconnu", PASSWORD, None, IP).await;

    assert!(matches!(wrong, Err(AuthError::InvalidCredentials)));
    assert!(matches!(unknown, Err(AuthError::InvalidCredentials)));
}

#[tokio::test]
async fn too_many_failures_block_even_the_right_password() {
    let auth = setup().await;
    for _ in 0..MAX_FAILURES {
        let _ = auth.login("david", "mauvais mot de passe", None, IP).await;
    }

    let result = auth.login("david", PASSWORD, None, IP).await;

    assert!(matches!(result, Err(AuthError::TooManyAttempts { .. })));
}

#[tokio::test]
async fn invalid_tokens_are_rejected() {
    let auth = setup().await;
    let fake = format!("hat_{}", "0".repeat(64));

    for token in ["", "n'importe quoi", "hat_123", fake.as_str()] {
        assert!(
            auth.authenticate(token).await.unwrap().is_none(),
            "{token:?}"
        );
    }
}

#[tokio::test]
async fn revoked_token_stops_working() {
    let auth = setup().await;
    let issued = auth.login("david", PASSWORD, None, IP).await.unwrap();

    assert!(auth.revoke_token(&issued.token).await.unwrap());

    assert!(auth.authenticate(&issued.token).await.unwrap().is_none());
}

#[tokio::test]
async fn expired_token_stops_working() {
    let store = Store::open_in_memory().await.unwrap();
    let auth = Auth::new(&store)
        .await
        .unwrap()
        .with_token_lifetime(Duration::ZERO);
    auth.create_user("david", PASSWORD).await.unwrap();
    let issued = auth.login("david", PASSWORD, None, IP).await.unwrap();

    assert!(auth.authenticate(&issued.token).await.unwrap().is_none());
    assert_eq!(auth.purge_expired_tokens().await.unwrap(), 1);
}

#[tokio::test]
async fn changing_password_revokes_all_tokens() {
    let auth = setup().await;
    let issued = auth.login("david", PASSWORD, None, IP).await.unwrap();

    let new_password = "une toute nouvelle phrase";
    assert!(auth.set_password("david", new_password).await.unwrap());

    assert!(auth.authenticate(&issued.token).await.unwrap().is_none());
    assert!(auth.login("david", PASSWORD, None, IP).await.is_err());
    assert!(auth.login("david", new_password, None, IP).await.is_ok());
}

#[tokio::test]
async fn deleting_a_user_revokes_their_tokens() {
    let auth = setup().await;
    let issued = auth.login("david", PASSWORD, None, IP).await.unwrap();

    assert!(auth.delete_user("david").await.unwrap());

    assert!(auth.authenticate(&issued.token).await.unwrap().is_none());
    assert_eq!(
        auth.list_users().await.unwrap(),
        Vec::<ha_auth::User>::new()
    );
}

#[tokio::test]
async fn account_rules_are_enforced() {
    let auth = setup().await;

    assert!(matches!(
        auth.create_user("david", PASSWORD).await,
        Err(AuthError::UserExists)
    ));
    assert!(matches!(
        auth.create_user("Robert'); DROP TABLE users;--", PASSWORD)
            .await,
        Err(AuthError::InvalidUsername)
    ));
    assert!(matches!(
        auth.create_user("alice", "court").await,
        Err(AuthError::PasswordTooShort { .. })
    ));
}

#[tokio::test]
async fn password_is_never_stored_in_clear() {
    let store = Store::open_in_memory().await.unwrap();
    let auth = Auth::new(&store).await.unwrap();
    auth.create_user("david", PASSWORD).await.unwrap();
    auth.login("david", PASSWORD, None, IP).await.unwrap();

    let (hash,): (String,) = sqlx::query_as("SELECT password_hash FROM users")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert!(hash.starts_with("$argon2id$"));
    assert!(!hash.contains(PASSWORD));
}

/// Code de l'appli d'authentification pour le créneau `now + offset`.
fn app_code(secret: &[u8], offset: i64) -> String {
    use ha_auth::totp;
    let now = totp::step_at(std::time::SystemTime::now());
    let step = now.checked_add_signed(offset).unwrap();
    format!("{:06}", totp::code_at(secret, step))
}

#[tokio::test]
async fn two_factor_requires_a_fresh_code() {
    let store = Store::open_in_memory().await.unwrap();
    let auth = Auth::new(&store).await.unwrap();
    auth.create_user("david", PASSWORD).await.unwrap();
    let before = auth.login("david", PASSWORD, None, IP).await.unwrap();

    let setup = ha_auth::totp::new_setup("david").unwrap();
    // Un mauvais code n'active rien : on ne peut pas s'enfermer dehors.
    assert!(matches!(
        auth.enable_totp("david", &setup.secret, "000000x").await,
        Err(AuthError::InvalidTotpCode)
    ));
    assert_eq!(auth.has_totp("david").await.unwrap(), Some(false));
    assert!(
        auth.enable_totp("david", &setup.secret, &app_code(&setup.secret, 0))
            .await
            .unwrap()
    );
    assert_eq!(auth.has_totp("david").await.unwrap(), Some(true));
    assert_eq!(auth.has_totp("inconnu").await.unwrap(), None);

    // Les sessions ouvertes avant sont fermées.
    assert!(auth.authenticate(&before.token).await.unwrap().is_none());

    // Le secret est chiffré dans la base.
    let raw: String = sqlx::query_scalar("SELECT totp_secret FROM users")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert!(ha_storage::is_sealed(&raw));
    assert!(!raw.contains(setup.secret_base32.as_str()));

    // Le mot de passe seul ne suffit plus, et l'erreur est la même que pour
    // un mauvais mot de passe.
    assert!(matches!(
        auth.login("david", PASSWORD, None, IP).await,
        Err(AuthError::InvalidCredentials)
    ));
    assert!(matches!(
        auth.login("david", PASSWORD, Some("123456"), IP).await,
        Err(AuthError::InvalidCredentials)
    ));
    // Le code de l'activation a déjà servi : refusé. Le suivant passe.
    let used = app_code(&setup.secret, 0);
    let next = app_code(&setup.secret, 1);
    let ip2 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50));
    if used != next {
        assert!(
            auth.login("david", PASSWORD, Some(&used), ip2)
                .await
                .is_err()
        );
    }
    let issued = auth
        .login("david", PASSWORD, Some(&next), ip2)
        .await
        .unwrap();
    assert!(auth.authenticate(&issued.token).await.unwrap().is_some());
    // Rejouer un code accepté : refusé.
    assert!(
        auth.login("david", PASSWORD, Some(&next), ip2)
            .await
            .is_err()
    );
    // Un bon code avec un mauvais mot de passe : refusé.
    let ip3 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 51));
    assert!(
        auth.login("david", "pas le bon mot de passe", Some(&next), ip3)
            .await
            .is_err()
    );

    // Désactivée : le mot de passe suffit de nouveau.
    assert!(auth.disable_totp("david").await.unwrap());
    assert!(auth.login("david", PASSWORD, None, ip3).await.is_ok());
    assert!(!auth.disable_totp("inconnu").await.unwrap());
}

#[tokio::test]
async fn unreadable_two_factor_secret_refuses_login_without_crashing() {
    let store = Store::open_in_memory().await.unwrap();
    let auth = Auth::new(&store).await.unwrap();
    auth.create_user("david", PASSWORD).await.unwrap();
    // Secret chiffré avec une autre clé (clé perdue, par exemple).
    let other = ha_storage::SecretKey::random().unwrap();
    sqlx::query("UPDATE users SET totp_secret = ?1")
        .bind(other.seal("users.totp_secret", "MZXW6YTBOI").unwrap())
        .execute(store.pool())
        .await
        .unwrap();
    assert!(matches!(
        auth.login("david", PASSWORD, Some("123456"), IP).await,
        Err(AuthError::InvalidCredentials)
    ));
    // La solution : désactiver en ligne de commande, puis réactiver.
    assert!(auth.disable_totp("david").await.unwrap());
    assert!(auth.login("david", PASSWORD, None, IP).await.is_ok());
}
