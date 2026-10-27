//! Erreurs de l'intégration Hue.

#[derive(Debug, thiserror::Error)]
pub enum HueError {
    #[error("connexion au pont impossible : {0}")]
    Http(#[from] reqwest::Error),

    #[error("{0}")]
    Tls(#[from] ha_tls::TlsError),

    #[error("adresse du pont invalide : {0:?} (exemple : 192.168.1.20)")]
    InvalidHost(String),

    #[error("le bouton du pont n'a pas été pressé à temps")]
    LinkButtonNotPressed,

    #[error("réponse inattendue du pont : {0}")]
    Bridge(String),

    #[error("réponse du pont trop grosse")]
    TooLarge,

    #[error("erreur de base de données : {0}")]
    Database(#[from] sqlx::Error),

    #[error("erreur de stockage : {0}")]
    Storage(#[from] ha_storage::StorageError),
}

/// Concept Rust : grâce à ce `From`, `?` transforme l'erreur de lecture
/// d'une réponse (voir [`ha_tls::read_limited`]) en `HueError`.
impl From<ha_tls::ReadError> for HueError {
    fn from(error: ha_tls::ReadError) -> Self {
        match error {
            ha_tls::ReadError::Http(error) => Self::Http(error),
            ha_tls::ReadError::TooLarge => Self::TooLarge,
        }
    }
}
