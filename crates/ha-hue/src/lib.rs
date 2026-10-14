//! Intégration Philips Hue : pilote les lampes via le pont Hue, en local,
//! sans passer par Internet ni par le cloud de Philips.
//!
//! - [`pairing`] : association avec le pont (bouton à presser) ;
//! - [`tls`] : connexion HTTPS avec épinglage du certificat du pont ;
//! - [`api`] : appels à l'API v2 du pont ;
//! - [`lights`] : lampes ⇄ entités du noyau ;
//! - [`Hue`] : la boucle principale.

pub mod api;
mod client;
mod error;
pub mod lights;
pub mod pairing;
pub mod sse;
pub mod tls;

pub use client::Hue;
pub use error::HueError;
pub use pairing::Pairing;
