//! Les lampes Hue comme entités du noyau : états, mises à jour en direct et
//! commandes. Comme le pont MQTT, cette partie ne touche pas au réseau.

use std::collections::HashMap;

use ha_core::{Attributes, EntityId, StateMachine};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::{debug, info, warn};

use crate::api::{Dimming, LightResource, OnState, is_resource_id};

/// Nombre maximal de lampes suivies (un pont Hue en gère une soixantaine).
const MAX_LIGHTS: usize = 500;
/// Longueur maximale d'un nom de lampe.
const MAX_NAME_LEN: usize = 100;

/// Une mise à jour partielle reçue dans le flux d'événements.
#[derive(Debug, Deserialize)]
struct LightUpdate {
    id: String,
    #[serde(default)]
    on: Option<OnState>,
    #[serde(default)]
    dimming: Option<Dimming>,
}

/// Correspondance entre lampes Hue (identifiants UUID) et entités.
#[derive(Debug, Default)]
pub struct Lights {
    by_id: HashMap<String, EntityId>,
    by_entity: HashMap<EntityId, String>,
}

impl Lights {
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Met à jour toutes les lampes à partir de la liste complète du pont.
    /// Les lampes qui ont disparu du pont sont retirées.
    pub fn sync(&mut self, resources: Vec<LightResource>, states: &StateMachine) {
        let mut present = Vec::new();
        for resource in resources.into_iter().take(MAX_LIGHTS) {
            if !is_resource_id(&resource.id) {
                warn!("lampe Hue ignorée : identifiant invalide {:?}", resource.id);
                continue;
            }
            let name: String = resource
                .metadata
                .as_ref()
                .map(|m| {
                    m.name
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(MAX_NAME_LEN)
                        .collect::<String>()
                })
                .unwrap_or_default();
            let entity_id = if let Some(entity_id) = self.by_id.get(&resource.id) {
                entity_id.clone()
            } else {
                let Some(entity_id) = self.allocate_id(&name, &resource.id) else {
                    continue;
                };
                info!("lampe Hue trouvée : {entity_id} ({name})");
                self.by_id.insert(resource.id.clone(), entity_id.clone());
                self.by_entity
                    .insert(entity_id.clone(), resource.id.clone());
                entity_id
            };
            present.push(resource.id.clone());

            let mut attributes = Attributes::new();
            if !name.is_empty() {
                attributes.insert("friendly_name".into(), json!(name));
            }
            let on = resource.on.is_some_and(|on| on.on);
            if let (true, Some(dimming)) = (on, resource.dimming) {
                attributes.insert(
                    "brightness".into(),
                    json!(percent_to_255(dimming.brightness)),
                );
            }
            set(states, &entity_id, on, attributes);
        }

        // Lampes supprimées du pont.
        let gone: Vec<String> = self
            .by_id
            .keys()
            .filter(|id| !present.contains(id))
            .cloned()
            .collect();
        for id in gone {
            if let Some(entity_id) = self.by_id.remove(&id) {
                self.by_entity.remove(&entity_id);
                states.remove(&entity_id);
                info!("lampe Hue retirée : {entity_id}");
            }
        }
    }

    /// Applique un message du flux d'événements. Renvoie `true` si une lampe
    /// a été ajoutée ou supprimée : il faut alors relire la liste complète.
    pub fn apply_event(&self, payload: &str, states: &StateMachine) -> bool {
        let Ok(Value::Array(events)) = serde_json::from_str::<Value>(payload) else {
            debug!("événement Hue illisible, ignoré");
            return false;
        };
        let mut reload = false;
        for event in &events {
            let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
            let Some(items) = event.get("data").and_then(Value::as_array) else {
                continue;
            };
            for item in items {
                if item.get("type").and_then(Value::as_str) != Some("light") {
                    continue; // Capteurs, scènes… : pas encore pris en charge.
                }
                match kind {
                    "update" => self.apply_update(item, states),
                    "add" | "delete" => reload = true,
                    _ => {}
                }
            }
        }
        reload
    }

    fn apply_update(&self, item: &Value, states: &StateMachine) {
        let Ok(update) = LightUpdate::deserialize(item) else {
            return;
        };
        let Some(entity_id) = self.by_id.get(&update.id) else {
            return;
        };
        let current = states.get(entity_id);
        let mut attributes = current
            .as_ref()
            .map(|state| state.attributes().clone())
            .unwrap_or_default();
        let on = update.on.map_or_else(
            || current.as_ref().is_some_and(|s| s.state() == "on"),
            |on| on.on,
        );
        if let Some(dimming) = update.dimming {
            attributes.insert(
                "brightness".into(),
                json!(percent_to_255(dimming.brightness)),
            );
        }
        if !on {
            attributes.remove("brightness");
        }
        set(states, entity_id, on, attributes);
    }

    /// Traduit un appel de service en commande pour le pont : `(id, corps)`.
    pub fn command(
        &self,
        domain: &str,
        service: &str,
        entity_id: &EntityId,
        data: &Attributes,
        states: &StateMachine,
    ) -> Option<(String, Value)> {
        let id = self.by_entity.get(entity_id)?;
        if domain != "light" {
            warn!("service {domain}.{service} incompatible avec {entity_id}");
            return None;
        }
        let turn_on = match service {
            "turn_on" => true,
            "turn_off" => false,
            "toggle" => states
                .get(entity_id)
                .is_none_or(|state| state.state() != "on"),
            _ => {
                warn!("service light.{service} non pris en charge");
                return None;
            }
        };
        let mut body = json!({"on": {"on": turn_on}});
        if let (true, Some(brightness)) = (turn_on, data.get("brightness")) {
            let Some(brightness) = brightness.as_u64().filter(|b| *b <= 255) else {
                warn!("luminosité invalide pour {entity_id} : {brightness} (0 à 255)");
                return None;
            };
            if brightness == 0 {
                body = json!({"on": {"on": false}});
            } else {
                body["dimming"] = json!({"brightness": brightness_to_percent(brightness)});
            }
        }
        Some((id.clone(), body))
    }

    /// `light.lampe_salon`, sinon `_2`, `_3`…
    fn allocate_id(&self, name: &str, resource_id: &str) -> Option<EntityId> {
        let slug = [name, resource_id]
            .into_iter()
            .map(ha_core::slugify)
            .find(|slug| !slug.is_empty())?;
        (1..=MAX_LIGHTS).find_map(|n| {
            let candidate = if n == 1 {
                format!("light.{slug}")
            } else {
                format!("light.{slug}_{n}")
            };
            EntityId::new(&candidate)
                .ok()
                .filter(|id| !self.by_entity.contains_key(id))
        })
    }
}

fn set(states: &StateMachine, entity_id: &EntityId, on: bool, attributes: Attributes) {
    let value = if on { "on" } else { "off" };
    if let Err(error) = states.set_by(crate::ID, entity_id.clone(), value, attributes) {
        warn!("état refusé pour {entity_id} : {error}");
    }
}

/// Hue : 0 à 100 %. Home Assistant : 0 à 255.
fn percent_to_255(percent: f64) -> u64 {
    // `as` sur un flottant borné entre 0 et 255 : pas de dépassement possible.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "valeur bornée entre 0 et 255 juste avant"
    )]
    let value = (percent.clamp(0.0, 100.0) * 255.0 / 100.0).round() as u64;
    value
}

fn brightness_to_percent(brightness: u64) -> f64 {
    // Valeur entre 1 et 255 : la conversion en flottant est exacte.
    #[expect(
        clippy::cast_precision_loss,
        reason = "valeur entre 1 et 255 : exacte en flottant"
    )]
    let percent = brightness as f64 * 100.0 / 255.0;
    (percent * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brightness_conversions() {
        assert_eq!(percent_to_255(100.0), 255);
        assert_eq!(percent_to_255(50.0), 128);
        assert_eq!(percent_to_255(-3.0), 0);
        assert_eq!(percent_to_255(f64::NAN.max(0.0)), 0);
        assert!((brightness_to_percent(255) - 100.0).abs() < f64::EPSILON);
        assert!((brightness_to_percent(128) - 50.2).abs() < f64::EPSILON);
    }
}
