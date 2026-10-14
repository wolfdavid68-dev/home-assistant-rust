//! Erreurs de l'intégration Somfy.

#[derive(Debug, thiserror::Error)]
pub enum SomfyError {
    #[error("connexion impossible : {0}")]
    Http(#[from] reqwest::Error),

    #[error("{0}")]
    Tls(#[from] ha_tls::TlsError),

    #[error("adresse de la box invalide : {0:?} (exemple : 192.168.1.30)")]
    InvalidHost(String),

    #[error("jeton refusé par la box (révoqué ou mode développeur désactivé)")]
    Unauthorized,

    #[error("identifiants Somfy refusés")]
    BadCredentials,

    #[error("réponse inattendue : {0}")]
    Unexpected(String),

    #[error("réponse trop grosse")]
    TooLarge,

    #[error("erreur de base de données : {0}")]
    Database(#[from] sqlx::Error),

    #[error("erreur de stockage : {0}")]
    Storage(#[from] ha_storage::StorageError),
}
