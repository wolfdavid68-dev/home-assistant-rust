//! Intégration Freebox (Freebox OS), en **lecture seule** :
//!
//! - présence des appareils suivis (téléphones…) : `device_tracker.<nom>` ;
//! - connexion Internet : `binary_sensor.freebox_internet`, débits ;
//! - signalement des appareils inconnus qui rejoignent le réseau.
//!
//! - [`api`] : l'API locale (HTTPS, session par défi HMAC) ;
//! - [`presence`] : présence et nouveaux appareils (sans réseau, testé) ;
//! - [`pairing`] : association, appareils suivis et connus ;
//! - [`client`] : la boucle de relevé.

pub mod api;
pub mod client;
mod error;
pub mod pairing;
pub mod presence;

pub use client::Freebox;
pub use error::FreeboxError;
pub use pairing::Pairing;
