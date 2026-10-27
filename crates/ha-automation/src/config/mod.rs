//! Le fichier `automations.yaml` : lecture et vérification.
//!
//! Deux temps :
//! 1. **lecture** : le YAML est transformé en structures Rust (`*Config`), avec
//!    `deny_unknown_fields` pour refuser toute faute de frappe ;
//! 2. **vérification** : chaque valeur est contrôlée (identifiants d'entités,
//!    heures, durées, limites) et l'on obtient des [`Automation`] prêtes à
//!    l'emploi, où plus rien ne peut être invalide.
//!
//! Le module est rangé par sujet :
//!
//! - `read` : ce qu'on lit (les structures `*Config`) ;
//! - `model` : ce qu'on obtient, vérifié ([`Automation`], [`Trigger`]…) ;
//! - `check` : les points d'entrée ([`parse`], [`check_all`]…) et la
//!   vérification d'une automatisation ;
//! - `triggers`, `conditions`, `actions` : la vérification de chaque partie ;
//! - `data` : les modèles `{{ … }}` dans les données d'une action ;
//! - `describe` : chaque partie, en français, pour l'historique ;
//! - `entities` : les entités citées, et le soleil ;
//! - `parsing` : les petites lectures partagées (heures, durées, jours…).

use std::time::Duration;

/// Taille maximale du fichier.
pub const MAX_FILE_SIZE: usize = 1024 * 1024;
/// Nombre maximal d'automatisations, et d'éléments dans chacune.
pub const MAX_AUTOMATIONS: usize = 200;
const MAX_ITEMS: usize = 20;
/// Durée maximale d'une attente (`delay`, `for`).
const MAX_DURATION: Duration = Duration::from_hours(24);

mod actions;
mod check;
mod conditions;
mod data;
mod describe;
mod entities;
mod model;
mod parsing;
mod read;
mod triggers;

pub(crate) use actions::check_action;
pub use check::{check_all, parse, parse_configs};
pub use data::DataTemplates;
pub use entities::uses_sun;
pub use model::{
    Action, Automation, CalendarEdge, Condition, MAX_REPEAT, Pattern, RepeatUntil, TimeAt, Trigger,
};
pub(crate) use read::ActionConfig;
pub use read::{AutomationConfig, RunMode};
