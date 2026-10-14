//! Intégration Somfy TaHoma : volets roulants, capteurs, alarme et chauffage,
//! via l'**API locale** de la box (« mode développeur » officiel de Somfy).
//!
//! - [`cloud`] : obtention du jeton, une seule fois, avec le compte Somfy ;
//! - [`api`] : appels à l'API locale de la box ;
//! - [`devices`] : appareils ⇄ entités du noyau ;
//! - [`pairing`] : association et sauvegarde ;
//! - [`Somfy`] : la boucle principale.

pub mod api;
mod client;
pub mod cloud;
pub mod devices;
mod error;
pub mod pairing;

pub use client::Somfy;
pub use error::SomfyError;
pub use pairing::Pairing;
