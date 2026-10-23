//! Intégration Somfy TaHoma : volets roulants, capteurs, alarme et chauffage,
//! via l'**API locale** de la box (« mode développeur » officiel de Somfy).
//!
//! - [`cloud`] : obtention du jeton, une seule fois, avec le compte Somfy ;
//! - [`api`] : appels à l'API locale de la box ;
//! - [`devices`] : appareils ⇄ entités du noyau ;
//! - [`pairing`] : association et sauvegarde ;
//! - [`heating`] : le chauffage Atlantic, lu par le serveur Somfy ;
//! - [`Somfy`] : la boucle principale (la box) ;
//! - [`HeatingCloud`] : la boucle du chauffage (le serveur Somfy).

pub mod api;
mod client;
pub mod cloud;
pub mod devices;
mod error;
pub mod heating;
mod heating_client;
pub mod pairing;

pub use client::Somfy;
pub use heating_client::{CloudAccount, HeatingCloud};

/// L'identifiant de l'intégration, propriétaire de ses entités (voir
/// [`ha_core::integrations`]).
pub const ID: &str = "somfy";
/// Son nom affiché.
pub const NAME: &str = "Somfy";
pub use error::SomfyError;
pub use pairing::Pairing;
