//! Ce qui peut mal tourner avec SmartThings.

/// Une erreur de SmartThings. Le message ne cite jamais un jeton.
#[derive(Debug, thiserror::Error)]
pub enum SmartThingsError {
    /// Jeton refusé, ou renouvellement impossible : il faut réassocier.
    #[error("accès refusé par SmartThings")]
    Unauthorized,
    /// Trop de demandes : attendre ce nombre de secondes.
    #[error("trop de demandes à SmartThings (attendre {0} s)")]
    RateLimited(u64),
    #[error("SmartThings injoignable")]
    Unreachable,
    #[error("SmartThings a répondu {0}")]
    Status(u16),
    #[error("réponse inattendue de SmartThings : {0}")]
    Unexpected(String),
    #[error("base de données : {0}")]
    Storage(#[from] ha_storage::StorageError),
}
