//! Noyau domotique : bus d'événements, machine à états et registre des services.
//!
//! Le contenu arrive à l'étape 1 de la feuille de route.

/// Version du noyau, reprise de `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_set() {
        assert!(!VERSION.is_empty());
    }
}
