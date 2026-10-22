//! **Les colis Amazon** (étape 87) : comme l'Echo d'Amazon qui affiche
//! « Plaid va arriver aujourd'hui », la tablette murale ouvre une fenêtre
//! avec la photo de l'article quand un colis part en livraison, puis quand
//! il est livré.
//!
//! Amazon n'offre pas d'accès au suivi des colis d'un particulier. Mais il
//! envoie un e-mail à chaque étape (« Expédié », « En cours de livraison »,
//! « Livré »). La maison lit donc ces e-mails, **en lecture seule**, dans la
//! boîte Gmail (IMAP, avec un mot de passe d'application), et n'en garde
//! que l'essentiel.
//!
//! - [`imap`] : le client IMAP, qui ne sait pas modifier la boîte ;
//! - [`mail`] : lire un e-mail, et vérifier qu'il vient d'Amazon (DKIM) ;
//! - [`amazon`] : l'état des colis, d'après les e-mails ;
//! - [`Colis`] : la boucle, qui publie `sensor.colis_amazon` et
//!   `camera.colis_amazon` (la photo du dernier colis).

pub mod amazon;
mod client;
mod error;
pub mod imap;
pub mod mail;

pub use client::{Colis, Mailbox, POLL, check};
pub use error::ColisError;

/// L'identifiant de l'intégration, propriétaire de ses entités.
pub const ID: &str = "colis";
/// Son nom affiché.
pub const NAME: &str = "Colis Amazon";
/// Le capteur : le nombre de colis en route, et la liste.
pub const SENSOR: &str = "sensor.colis_amazon";
/// La photo de l'article du dernier colis.
pub const CAMERA: &str = "camera.colis_amazon";
/// L'évènement d'un colis qui part en livraison (« arrive aujourd'hui »).
pub const OUT_FOR_DELIVERY: &str = "colis_en_livraison";
/// L'évènement d'un colis livré.
pub const DELIVERED: &str = "colis_livre";
