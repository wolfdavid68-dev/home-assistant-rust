//! État d'une entité à un instant donné.

use std::sync::Arc;
use std::time::SystemTime;

use crate::entity_id::EntityId;

/// Longueur maximale de la valeur d'un état, en caractères (comme Home Assistant).
pub const MAX_STATE_LEN: usize = 255;

/// Attributs d'une entité : des paires clé → valeur JSON,
/// par exemple `{"brightness": 200, "unit_of_measurement": "°C"}`.
pub type Attributes = serde_json::Map<String, serde_json::Value>;

/// Ce que l'on sait d'une entité : sa valeur (`"on"`, `"21.5"`…), ses attributs
/// et les dates de ses derniers changements.
///
/// Concept Rust : les champs sont privés (pas de `pub`). Seule la machine à
/// états peut créer ou modifier un `State` ; le reste du code ne peut que le
/// lire avec les méthodes ci-dessous. Ainsi `last_changed` ne peut pas être
/// incohérent avec la valeur.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    entity_id: EntityId,
    #[expect(
        clippy::struct_field_names,
        reason = "« state » est le nom utilisé par Home Assistant pour la valeur"
    )]
    state: String,
    attributes: Attributes,
    last_changed: SystemTime,
    last_updated: SystemTime,
    /// L'intégration qui publie l'entité (`hue`, `mqtt`…), si on la connaît.
    pub(crate) owner: Option<Arc<str>>,
}

impl State {
    /// `pub(crate)` : visible uniquement à l'intérieur de `ha-core`.
    pub(crate) fn new(
        entity_id: EntityId,
        state: String,
        attributes: Attributes,
        last_changed: SystemTime,
        last_updated: SystemTime,
        owner: Option<Arc<str>>,
    ) -> Self {
        Self {
            entity_id,
            state,
            attributes,
            last_changed,
            last_updated,
            owner,
        }
    }

    /// L'intégration qui publie l'entité (`hue`, `mqtt`…), si on la connaît.
    #[must_use]
    pub fn owner(&self) -> Option<&str> {
        self.owner.as_deref()
    }

    #[must_use]
    pub fn entity_id(&self) -> &EntityId {
        &self.entity_id
    }

    /// La valeur de l'état, par exemple `"on"`.
    #[must_use]
    pub fn state(&self) -> &str {
        &self.state
    }

    #[must_use]
    pub fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    /// Dernière fois que la **valeur** a changé.
    #[must_use]
    pub fn last_changed(&self) -> SystemTime {
        self.last_changed
    }

    /// Dernière fois que la valeur **ou** un attribut a changé.
    #[must_use]
    pub fn last_updated(&self) -> SystemTime {
        self.last_updated
    }
}
