//! Stockage SQLite : dernier état connu de chaque entité (restauré au
//! démarrage) et historique de tous les changements.
//!
//! - [`Store`] : la base de données elle-même.
//! - [`Recorder`] : une tâche abonnée au bus d'événements qui écrit chaque
//!   changement d'état dans la base.
//! - [`SecretKey`] : chiffrement des secrets rangés dans la base (jetons des
//!   appareils), avec une clé gardée dans un fichier séparé.

mod error;
mod recorder;
mod secrets;
mod store;

pub use error::StorageError;
pub use recorder::Recorder;
pub use secrets::{SecretKey, is_sealed};
pub use store::{HistoryEntry, MAX_HISTORY_LIMIT, Store, StoredState, from_micros, to_micros};
