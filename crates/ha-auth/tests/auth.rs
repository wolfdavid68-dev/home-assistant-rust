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

    let issued = auth.login("david", PASSWORD, IP).await.unwrap();
    let user = auth.authenticate(&issued.token).await.unwrap().unwrap();

    assert_eq!(user.username, "david");
    // Le jeton ne doit jamais apparaître dans un affichage de débogage.
    assert!(!format!("{issued:?}").contains(issued.token.as_str()));
}

#[tokio::test]
async fn wrong_password_and_unknown_user_give_the_same_error() {
    let auth = setup().await;

    let wrong = auth.login("david", "pas le bon mot de passe", IP).await;
    let unknown = auth.login("inconnu", PASSWORD, IP).await;

    assert!(matches!(wrong, Err(AuthError::InvalidCredentials)));
    assert!(matches!(unknown, Err(AuthError::InvalidCredentials)));
}

#[tokio::test]
async fn too_many_failures_block_even_the_right_password() {
    let auth = setup().await;
    for _ in 0..MAX_FAILURES {
        let _ = auth.login("david", "mauvais mot de passe", IP).await;
    }

    let result = auth.login("david", PASSWORD, IP).await;

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
    let issued = auth.login("david", PASSWORD, IP).await.unwrap();

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
    let issued = auth.login("david", PASSWORD, IP).await.unwrap();

    assert!(auth.authenticate(&issued.token).await.unwrap().is_none());
    assert_eq!(auth.purge_expired_tokens().await.unwrap(), 1);
}

#[tokio::test]
async fn changing_password_revokes_all_tokens() {
    let auth = setup().await;
    let issued = auth.login("david", PASSWORD, IP).await.unwrap();

    let new_password = "une toute nouvelle phrase";
    assert!(auth.set_password("david", new_password).await.unwrap());

    assert!(auth.authenticate(&issued.token).await.unwrap().is_none());
    assert!(auth.login("david", PASSWORD, IP).await.is_err());
    assert!(auth.login("david", new_password, IP).await.is_ok());
}

#[tokio::test]
async fn deleting_a_user_revokes_their_tokens() {
    let auth = setup().await;
    let issued = auth.login("david", PASSWORD, IP).await.unwrap();

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
    auth.login("david", PASSWORD, IP).await.unwrap();

    let (hash,): (String,) = sqlx::query_as("SELECT password_hash FROM users")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert!(hash.starts_with("$argon2id$"));
    assert!(!hash.contains(PASSWORD));
}
