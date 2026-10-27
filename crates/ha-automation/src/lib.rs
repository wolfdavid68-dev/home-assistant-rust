//! Automatisations : « **quand** X se produit, **si** Y est vrai, **alors** fais Z ».
//!
//! - [`config`] : lecture et vérification du fichier `automations.yaml` ;
//! - [`evaluator`] : quels déclencheurs se produisent, quelles conditions sont remplies ;
//! - [`editor`] : les automatisations créées depuis le tableau de bord ;
//! - [`Engine`] : la tâche qui écoute le bus et l'horloge et exécute les actions.
//!
//! Sécurité : une automatisation ne peut **que** appeler des services sur des
//! entités et attendre. Pas de commande système, pas de code. Les
//! [`template`]s (`{{ states('sensor.x') }}`) ne savent que **lire** des
//! états et calculer : un fichier d'automatisations ne peut rien faire
//! d'autre. Les [`entities`] (capteurs modèles) publient le résultat d'un
//! modèle comme un capteur.

pub mod config;
pub mod editor;
mod engine;
pub mod entities;
mod error;
pub mod evaluator;
pub mod presets;
pub mod scripts;
pub mod template;
pub mod trace;

use std::path::Path;

pub use config::{
    Action, Automation, AutomationConfig, Condition, MAX_REPEAT, Pattern, RepeatUntil, RunMode,
    TimeAt, Trigger, check_all, parse, parse_configs, uses_sun,
};
pub use editor::Editor;
pub use engine::{AUTOMATION_TRIGGERED, Engine, Reloader, Reloads, Setup, reload_channel};
pub use entities::{
    TemplateEntities, TemplateEntity, load_template_entities, parse_template_entities,
};
pub use error::AutomationError;
pub use scripts::{Scene, Script, parse_scenes, parse_scripts};
pub use template::{Template, TriggerData};
pub use trace::{Outcome, StepStatus, Trace, Traces};

/// Lit et vérifie le fichier d'automatisations, et renvoie celles qui sont
/// activées. Un fichier absent donne une liste vide.
pub fn load(path: &Path) -> Result<Vec<Automation>, AutomationError> {
    check_all(&load_configs(path)?)
}

/// Lit le fichier d'automatisations sans vérifier les valeurs (voir
/// [`check_all`]). Un fichier absent donne une liste vide.
pub fn load_configs(path: &Path) -> Result<Vec<AutomationConfig>, AutomationError> {
    let Some(text) = ha_core::config_file::read(path, config::MAX_FILE_SIZE as u64)
        .map_err(|error| error.into_error(AutomationError::TooLarge))?
    else {
        return Ok(Vec::new());
    };
    parse_configs(&text)
}
