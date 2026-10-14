//! Automatisations : « **quand** X se produit, **si** Y est vrai, **alors** fais Z ».
//!
//! - [`config`] : lecture et vérification du fichier `automations.yaml` ;
//! - [`evaluator`] : quels déclencheurs se produisent, quelles conditions sont remplies ;
//! - [`Engine`] : la tâche qui écoute le bus et l'horloge et exécute les actions.
//!
//! Sécurité : une automatisation ne peut **que** appeler des services sur des
//! entités et attendre. Pas de commande système, pas de code, pas de modèle
//! Jinja : un fichier d'automatisations ne peut rien faire d'autre.

pub mod config;
mod engine;
mod error;
pub mod evaluator;

use std::path::Path;

pub use config::{Action, Automation, Condition, Trigger, parse};
pub use engine::Engine;
pub use error::AutomationError;

/// Lit le fichier d'automatisations. Un fichier absent donne une liste vide.
pub fn load(path: &Path) -> Result<Vec<Automation>, AutomationError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    if metadata.len() > config::MAX_FILE_SIZE as u64 {
        return Err(AutomationError::TooLarge);
    }
    parse(&std::fs::read_to_string(path)?)
}
