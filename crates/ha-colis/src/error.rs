//! Ce qui peut mal tourner avec la boîte aux lettres.

/// Une erreur de la boîte aux lettres. Le message ne cite jamais le mot de
/// passe.
#[derive(Debug, thiserror::Error)]
pub enum ColisError {
    /// Compte ou mot de passe d'application refusé.
    #[error("compte refusé par la boîte aux lettres")]
    Unauthorized,
    #[error("boîte aux lettres injoignable ({0})")]
    Io(std::io::Error),
    #[error("chiffrement : {0}")]
    Tls(String),
    #[error("délai dépassé")]
    Timeout,
    #[error("réponse inattendue : {0}")]
    Protocol(String),
    #[error("demande refusée : {0}")]
    Refused(String),
    #[error("base de données : {0}")]
    Storage(#[from] ha_storage::StorageError),
}
