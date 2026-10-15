//! Automatisations : « **quand** X se produit, **si** Y est vrai, **alors** fais Z ».
//!
//! - [`config`] : lecture et vérification du fichier `automations.yaml` ;
//! - [`evaluator`] : quels déclencheurs se produisent, quelles conditions sont remplies ;
//! - [`editor`] : les automatisations créées depuis le tableau de bord ;
//! - [`Engine`] : la tâche qui écoute le bus et l'horloge et exécute les actions.
//!
//! Sécurité : une automatisation ne peut **que** appeler des services sur des
//! entités et attendre. Pas de commande système, pas de code, pas de modèle
//! Jinja : un fichier d'automatisations ne peut rien faire d'autre.

pub mod config;
pub mod editor;
mod engine;
mod error;
pub mod evaluator;
pub mod scripts;

use std::path::Path;

pub use config::{
    Action, Automation, AutomationConfig, Condition, RunMode, Trigger, check_all, parse,
    parse_configs, uses_sun,
};
pub use editor::Editor;
pub use engine::{Engine, Reloader, Reloads, Setup, reload_channel};
pub use error::AutomationError;
pub use scripts::{Scene, Script, parse_scenes, parse_scripts};

/// Lit et vérifie le fichier d'automatisations, et renvoie celles qui sont
/// activées. Un fichier absent donne une liste vide.
pub fn load(path: &Path) -> Result<Vec<Automation>, AutomationError> {
    check_all(&load_configs(path)?)
}

/// Lit le fichier d'automatisations sans vérifier les valeurs (voir
/// [`check_all`]). Un fichier absent donne une liste vide.
pub fn load_configs(path: &Path) -> Result<Vec<AutomationConfig>, AutomationError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    if metadata.len() > config::MAX_FILE_SIZE as u64 {
        return Err(AutomationError::TooLarge);
    }
    parse_configs(&std::fs::read_to_string(path)?)
}
