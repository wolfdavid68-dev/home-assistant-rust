//! Erreurs de l'authentification.

use std::time::Duration;

/// Concept : les messages de `InvalidCredentials` ne disent volontairement pas
/// si c'est le nom ou le mot de passe qui est faux. Sinon, un attaquant
/// pourrait d'abord deviner les noms de compte existants.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("nom d'utilisateur, mot de passe ou code incorrect")]
    InvalidCredentials,

    #[error("code de vérification incorrect (vérifie l'heure du téléphone)")]
    InvalidTotpCode,

    #[error("trop de tentatives de connexion, réessayer dans {} s", retry_after.as_secs())]
    TooManyAttempts { retry_after: Duration },

    #[error("nom d'utilisateur invalide : 3 à 32 caractères parmi a-z, 0-9, « _ », « - » et « . »")]
    InvalidUsername,

    #[error("mot de passe trop court : au moins {min} caractères")]
    PasswordTooShort { min: usize },

    #[error("mot de passe trop long : au plus {max} caractères")]
    PasswordTooLong { max: usize },

    #[error("ce nom d'utilisateur existe déjà")]
    UserExists,

    #[error("c'est le dernier administrateur : il en faut toujours au moins un")]
    LastAdmin,

    #[error("nom de tablette invalide : 1 à {max} caractères, sans caractère de contrôle")]
    InvalidWallLabel { max: usize },

    #[error(
        "un compte administrateur ne reste pas connecté sur une tablette murale : \
         connecte-la avec un compte « famille » ou « invité »"
    )]
    AdminOnWall,

    #[error("erreur de base de données : {0}")]
    Database(#[from] sqlx::Error),

    #[error("erreur de stockage : {0}")]
    Storage(#[from] ha_storage::StorageError),

    #[error("erreur interne : {0}")]
    Internal(String),
}
