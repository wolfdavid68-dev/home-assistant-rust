//! Stockage SQLite : dernier état connu de chaque entité (restauré au
//! démarrage) et historique de tous les changements.
//!
//! - [`Store`] : la base de données elle-même.
//! - [`Recorder`] : une tâche abonnée au bus d'événements qui écrit chaque
//!   changement d'état dans la base.
//! - [`AuditRecord`] : journal d'audit signé (qui a fait quoi, quand, d'où).
//! - [`SecretKey`] : chiffrement des secrets rangés dans la base (jetons des
//!   appareils), avec une clé gardée dans un fichier séparé.

mod audit;
mod automations;
pub mod backup;
mod error;
mod logbook;
mod recorder;
mod secrets;
mod store;

pub use audit::{AuditCheck, AuditEntry, AuditRecord, MAX_AUDIT_LIMIT, clean_field};
pub use automations::{AutomationVersion, MAX_VERSIONS, StoredAutomation};
pub use error::StorageError;
pub use logbook::{LogbookEntry, MAX_LOGBOOK_LIMIT, NewLogbookEntry};
pub use recorder::Recorder;
pub use secrets::{SecretKey, is_sealed};
pub use store::{
    HistoryEntry, HistorySeries, MAX_HISTORY_LIMIT, MAX_SERIES_BUCKETS, MAX_SERIES_CHANGES,
    SeriesPoint, StateChange, Store, StoredState, from_micros, to_micros,
};
