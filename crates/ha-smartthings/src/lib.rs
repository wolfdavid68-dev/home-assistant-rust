//! **SmartThings** de Samsung (étape 86) : les appareils de la maison
//! SmartThings (lumières, prises, volets, thermostats, climatiseurs,
//! serrures, capteurs, lave-linge…) dans le tableau de bord, Assist, la
//! voix et les automatisations.
//!
//! On passe par l'API officielle de Samsung, avec une « application »
//! SmartThings créée une fois pour la maison (OAuth) : les jetons simples
//! ne vivent plus que 24 heures. Le jeton d'accès est renouvelé tout seul
//! avant d'expirer ; le jeton de renouvellement, à usage unique, est
//! enregistré aussitôt (chiffré dans la base).
//!
//! - [`api`] : l'association, les jetons, les appels à l'API ;
//! - [`devices`] : appareils ⇄ entités (sans réseau, testé à part) ;
//! - [`SmartThings`] : la boucle principale.

pub mod api;
mod client;
pub mod devices;
mod error;

pub use client::{POLL, SmartThings, fresh_tokens};
pub use error::SmartThingsError;

/// L'identifiant de l'intégration, propriétaire de ses entités.
pub const ID: &str = "smartthings";
/// Son nom affiché.
pub const NAME: &str = "SmartThings";
