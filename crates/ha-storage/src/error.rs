//! Erreurs du stockage.

/// Concept Rust : `#[from]` permet d'utiliser l'opérateur `?` sur une erreur
/// `sqlx::Error` dans une fonction qui renvoie `StorageError` : la conversion
/// est faite automatiquement.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("erreur de base de données : {0}")]
    Database(#[from] sqlx::Error),

    #[error("erreur de migration du schéma : {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    #[error("impossible de préparer le fichier de base de données : {0}")]
    Io(#[from] std::io::Error),

    #[error("attributs illisibles : {0}")]
    Json(#[from] serde_json::Error),

    #[error("chiffrement des secrets : {0}")]
    Secret(&'static str),

    #[error("date hors limites : {0}")]
    Timestamp(&'static str),
}
