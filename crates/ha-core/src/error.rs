//! Erreurs du noyau.

/// Concept Rust : en Rust, une fonction qui peut échouer renvoie un
/// `Result<T, E>` : soit `Ok(valeur)`, soit `Err(erreur)`. Il n'y a pas
/// d'exceptions cachées ; l'appelant est obligé de traiter l'erreur.
///
/// `thiserror` génère le texte des messages à partir des `#[error(...)]`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoreError {
    #[error("identifiant d'entité invalide : {0:?} (format attendu : domaine.nom, en minuscules)")]
    InvalidEntityId(String),

    #[error("état trop long pour {entity_id} : {len} caractères (maximum {max})")]
    StateTooLong {
        entity_id: String,
        len: usize,
        max: usize,
    },
}
