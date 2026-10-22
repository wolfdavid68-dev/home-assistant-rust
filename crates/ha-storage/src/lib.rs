//! Stockage SQLite : dernier état connu de chaque entité (restauré au
//! démarrage) et historique de tous les changements.
//!
//! - [`Store`] : la base de données elle-même.
//! - [`Recorder`] : une tâche abonnée au bus d'événements qui écrit chaque
//!   changement d'état dans la base.
//! - [`AuditRecord`] : journal d'audit signé (qui a fait quoi, quand, d'où).
//! - [`SecretKey`] : chiffrement des secrets rangés dans la base (jetons des
//!   appareils), avec une clé gardée dans un fichier séparé.

mod agenda;
mod audit;
mod automations;
pub mod backup;
mod drawing;
mod error;
mod logbook;
mod mail;
mod push;
mod recorder;
mod secrets;
mod smartthings;
mod statistics;
mod store;
mod timetree;

pub use agenda::{CalendarEvent, DueChange, EventExtra, NewCalendarEvent, TodoItem};
pub use audit::{AuditCheck, AuditEntry, AuditRecord, MAX_AUDIT_LIMIT, clean_field};
pub use automations::{AutomationVersion, MAX_VERSIONS, StoredAutomation};
pub use drawing::{HomeImage, HomeImageInfo};
pub use error::StorageError;
pub use logbook::{LogbookEntry, MAX_LOGBOOK_LIMIT, NewLogbookEntry};
pub use mail::MailAccount;
pub use push::{NewPushSubscription, PushSubscriptionInfo, PushTarget, VapidKeys};
pub use recorder::Recorder;
pub use secrets::{SecretKey, is_sealed};
pub use smartthings::{SmartThingsAccount, SmartThingsTokens};
pub use statistics::{
    HourSummary, MAX_STATISTICS_POINTS, Period, StatisticPoint, Statistics, summarize,
};
pub use store::{
    HistoryEntry, HistorySeries, MAX_HISTORY_LIMIT, MAX_SERIES_BUCKETS, MAX_SERIES_CHANGES,
    SeriesPoint, StateChange, Store, StoredState, from_micros, to_micros,
};
pub use timetree::TimeTreeAccount;
