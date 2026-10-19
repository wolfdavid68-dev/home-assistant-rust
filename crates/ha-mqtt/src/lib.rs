//! Intégration MQTT : pilote de vrais appareils (Zigbee2MQTT, Tasmota,
//! Shelly, ESPHome…) grâce à la découverte automatique de Home Assistant.
//!
//! - [`discovery`] : lecture des descriptions `homeassistant/…/config` ;
//! - [`template`] : extraction des valeurs (`{{ value_json.temperature }}`) ;
//! - [`bridge`] : messages MQTT ⇄ états des entités et appels de service ;
//! - [`shelly`] : les Shelly récents (Gen2 et plus), reconnus sans découverte ;
//! - [`Mqtt`] : la connexion au broker.

pub mod bridge;
mod client;
pub mod discovery;
pub mod profile;
pub mod shelly;
pub mod template;

use zeroize::Zeroizing;

pub use bridge::Bridge;

/// L'identifiant de l'intégration, propriétaire de ses entités (voir
/// [`ha_core::integrations`]).
pub const ID: &str = "mqtt";
/// Son nom affiché.
pub const NAME: &str = "MQTT";
pub use client::Mqtt;

/// Paramètres de connexion au broker.
#[derive(Clone)]
pub struct MqttConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<Zeroizing<String>>,
    pub client_id: String,
    /// Préfixe des sujets de découverte (`homeassistant` par défaut).
    pub discovery_prefix: String,
}

/// `Debug` écrit à la main pour ne jamais afficher le mot de passe.
impl std::fmt::Debug for MqttConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MqttConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<masqué>"))
            .field("client_id", &self.client_id)
            .field("discovery_prefix", &self.discovery_prefix)
            .finish()
    }
}
