//! Erreurs de chargement des automatisations.

#[derive(Debug, thiserror::Error)]
pub enum AutomationError {
    #[error("lecture du fichier impossible : {0}")]
    Io(#[from] std::io::Error),

    #[error("fichier trop gros (1 Mio au plus)")]
    TooLarge,

    #[error("YAML invalide : {0}")]
    Yaml(String),

    #[error("trop d'automatisations ({0} au plus)")]
    TooMany(usize),

    #[error("automatisation « {id} » : {reason}")]
    Invalid { id: String, reason: String },

    #[error("{kind} « {id} » : {reason}")]
    InvalidEntry {
        kind: &'static str,
        id: String,
        reason: String,
    },
}
