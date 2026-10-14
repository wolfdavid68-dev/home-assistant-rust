//! Les appareils Somfy comme entités du noyau. Comme pour MQTT et Hue, cette
//! partie ne touche pas au réseau : elle est testée sans box.
//!
//! | Appareil Somfy (`uiClass`) | Entité | Pilotable |
//! |---|---|---|
//! | `RollerShutter`, `ExteriorScreen`, `Screen`, `Awning`… | `cover.*` (position 0–100 %) | oui |
//! | `ContactSensor` | `binary_sensor.*` (ouvert / fermé) | non |
//! | `TemperatureSensor`, `LightSensor` | `sensor.*` | non |
//! | `OccupancySensor`, `SmokeSensor` | `binary_sensor.*` | non |
//! | `Alarm` | `sensor.*` (mode de l'alarme) | non (pour l'instant) |
//! | `HeatingSystem` | `sensor.*` (température, consigne en attribut) | non (pour l'instant) |
//!
//! Les autres appareils (portail, porte de garage…) sont ignorés : on
//! n'ouvre pas une porte d'entrée sans y avoir réfléchi spécifiquement.

use std::collections::HashMap;

use ha_core::{Attributes, EntityId, StateMachine};
use serde_json::{Value, json};
use tracing::{debug, info, warn};

use crate::api::{Device, Event};

const MAX_DEVICES: usize = 500;
const MAX_LABEL_LEN: usize = 100;

/// Ce qu'on fait d'un appareil.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Cover,
    Contact,
    Temperature,
    Luminance,
    Occupancy,
    Smoke,
    Alarm,
    Heating,
}

impl Kind {
    /// D'après la catégorie Somfy (`uiClass`). `None` : appareil ignoré.
    #[must_use]
    pub fn from_ui_class(ui_class: &str) -> Option<Self> {
        Some(match ui_class {
            "RollerShutter"
            | "ExteriorScreen"
            | "Screen"
            | "Awning"
            | "Shutter"
            | "ExteriorVenetianBlind"
            | "VenetianBlind"
            | "Curtain"
            | "Pergola" => Self::Cover,
            "ContactSensor" => Self::Contact,
            "TemperatureSensor" => Self::Temperature,
            "LightSensor" => Self::Luminance,
            "OccupancySensor" => Self::Occupancy,
            "SmokeSensor" => Self::Smoke,
            "Alarm" => Self::Alarm,
            "HeatingSystem" | "ThermostaticValve" => Self::Heating,
            _ => return None,
        })
    }

    fn domain(self) -> &'static str {
        match self {
            Self::Cover => "cover",
            Self::Contact | Self::Occupancy | Self::Smoke => "binary_sensor",
            Self::Temperature | Self::Luminance | Self::Alarm | Self::Heating => "sensor",
        }
    }
}

/// Un appareil suivi : ses états bruts et l'entité correspondante.
#[derive(Debug)]
struct Tracked {
    kind: Kind,
    label: String,
    entity_id: EntityId,
    commands: Vec<String>,
    states: HashMap<String, Value>,
    available: bool,
}

/// Correspondance appareils Somfy ⇄ entités.
#[derive(Debug, Default)]
pub struct Devices {
    by_url: HashMap<String, Tracked>,
    by_entity: HashMap<EntityId, String>,
}

/// Une commande à envoyer à la box.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub device_url: String,
    pub name: String,
    pub parameters: Vec<Value>,
}

impl Devices {
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_url.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_url.is_empty()
    }

    /// Met à jour toutes les entités à partir de la liste complète des appareils.
    pub fn sync(&mut self, devices: Vec<Device>, states: &StateMachine) {
        let mut present = Vec::new();
        for device in devices.into_iter().take(MAX_DEVICES) {
            let Some(kind) = Kind::from_ui_class(&device.definition.ui_class) else {
                debug!(
                    "appareil Somfy ignoré : {} ({})",
                    device.label, device.definition.ui_class
                );
                continue;
            };
            if !is_device_url(&device.device_url) {
                warn!(
                    "appareil Somfy ignoré : adresse invalide {:?}",
                    device.device_url
                );
                continue;
            }
            let label: String = device
                .label
                .chars()
                .filter(|c| !c.is_control())
                .take(MAX_LABEL_LEN)
                .collect();
            let entity_id = match self.by_url.get(&device.device_url) {
                Some(tracked) if tracked.kind == kind => tracked.entity_id.clone(),
                _ => {
                    let Some(entity_id) = self.allocate_id(kind, &label) else {
                        continue;
                    };
                    info!("appareil Somfy trouvé : {entity_id} ({label})");
                    entity_id
                }
            };
            present.push(device.device_url.clone());
            self.by_entity
                .insert(entity_id.clone(), device.device_url.clone());
            let tracked = Tracked {
                kind,
                label,
                entity_id,
                commands: device
                    .definition
                    .commands
                    .into_iter()
                    .map(|c| c.name)
                    .collect(),
                states: device
                    .states
                    .into_iter()
                    .map(|s| (s.name, s.value))
                    .collect(),
                available: device.available,
            };
            publish(&tracked, states);
            self.by_url.insert(device.device_url, tracked);
        }

        // Appareils retirés de la box.
        let gone: Vec<String> = self
            .by_url
            .keys()
            .filter(|url| !present.contains(url))
            .cloned()
            .collect();
        for url in gone {
            if let Some(tracked) = self.by_url.remove(&url) {
                self.by_entity.remove(&tracked.entity_id);
                states.remove(&tracked.entity_id);
                info!("appareil Somfy retiré : {}", tracked.entity_id);
            }
        }
    }

    /// Applique les événements de la box. Renvoie `true` si la liste des
    /// appareils a changé (ajout, suppression) : il faut alors tout relire.
    pub fn apply_events(&mut self, events: &[Event], states: &StateMachine) -> bool {
        let mut reload = false;
        for event in events {
            let Some(tracked) = event
                .device_url
                .as_ref()
                .and_then(|url| self.by_url.get_mut(url))
            else {
                reload |= matches!(
                    event.name.as_str(),
                    "DeviceCreatedEvent" | "DeviceRemovedEvent"
                );
                continue;
            };
            match event.name.as_str() {
                "DeviceStateChangedEvent" => {
                    for state in &event.device_states {
                        tracked
                            .states
                            .insert(state.name.clone(), state.value.clone());
                    }
                }
                "DeviceAvailableEvent" => tracked.available = true,
                "DeviceUnavailableEvent" => tracked.available = false,
                "DeviceCreatedEvent" | "DeviceRemovedEvent" => reload = true,
                _ => continue,
            }
            publish(tracked, states);
        }
        reload
    }

    /// Traduit un appel de service en commande pour la box.
    #[must_use]
    pub fn command(
        &self,
        domain: &str,
        service: &str,
        entity_id: &EntityId,
        data: &Attributes,
    ) -> Option<Command> {
        let url = self.by_entity.get(entity_id)?;
        let tracked = self.by_url.get(url)?;
        if tracked.kind != Kind::Cover || domain != "cover" {
            warn!("service {domain}.{service} impossible sur {entity_id}");
            return None;
        }
        let (name, parameters) = match service {
            "open_cover" => ("open", vec![]),
            "close_cover" => ("close", vec![]),
            "stop_cover" => ("stop", vec![]),
            "set_cover_position" => {
                let Some(position) = data
                    .get("position")
                    .and_then(Value::as_u64)
                    .filter(|p| *p <= 100)
                else {
                    warn!("position invalide pour {entity_id} (0 à 100 attendu)");
                    return None;
                };
                // Somfy compte la fermeture (100 = fermé), Home Assistant
                // l'ouverture (100 = ouvert).
                ("setClosure", vec![json!(100 - position)])
            }
            _ => {
                warn!("service cover.{service} non pris en charge");
                return None;
            }
        };
        if !tracked.commands.iter().any(|c| c == name) {
            warn!(
                "{} ne connaît pas la commande Somfy « {name} »",
                tracked.label
            );
            return None;
        }
        Some(Command {
            device_url: url.clone(),
            name: name.to_owned(),
            parameters,
        })
    }

    fn allocate_id(&self, kind: Kind, label: &str) -> Option<EntityId> {
        let slug = Some(ha_core::slugify(label))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "somfy".to_owned());
        (1..=MAX_DEVICES).find_map(|n| {
            let candidate = if n == 1 {
                format!("{}.{slug}", kind.domain())
            } else {
                format!("{}.{slug}_{n}", kind.domain())
            };
            EntityId::new(&candidate)
                .ok()
                .filter(|id| !self.by_entity.contains_key(id))
        })
    }
}

/// Calcule l'état de l'entité à partir des états bruts Somfy, et le publie.
fn publish(tracked: &Tracked, states: &StateMachine) {
    let mut attributes = Attributes::new();
    attributes.insert("friendly_name".into(), json!(tracked.label));
    let state = |name: &str| tracked.states.get(name);

    let value: String = if tracked.available {
        match tracked.kind {
            Kind::Cover => {
                attributes.insert("device_class".into(), json!("shutter"));
                // `core:ClosureState` : 0 = ouvert, 100 = fermé.
                match state("core:ClosureState").and_then(as_f64) {
                    Some(closure) => {
                        let position = (100.0 - closure.clamp(0.0, 100.0)).round();
                        attributes.insert("current_position".into(), json!(position));
                        if position > 0.0 { "open" } else { "closed" }.to_owned()
                    }
                    // Volet sans position (moteur RTS) : seulement ouvert/fermé.
                    None => text(state("core:OpenClosedState"))
                        .unwrap_or("unknown")
                        .to_owned(),
                }
            }
            Kind::Contact => {
                attributes.insert("device_class".into(), json!("opening"));
                on_off(text(state("core:ContactState")), "open")
            }
            Kind::Occupancy => {
                attributes.insert("device_class".into(), json!("occupancy"));
                on_off(text(state("core:OccupancyState")), "personInside")
            }
            Kind::Smoke => {
                attributes.insert("device_class".into(), json!("smoke"));
                on_off(text(state("core:SmokeState")), "detected")
            }
            Kind::Temperature => {
                attributes.insert("device_class".into(), json!("temperature"));
                attributes.insert("unit_of_measurement".into(), json!("°C"));
                number(state("core:TemperatureState"))
            }
            Kind::Luminance => {
                attributes.insert("device_class".into(), json!("illuminance"));
                attributes.insert("unit_of_measurement".into(), json!("lx"));
                number(state("core:LuminanceState"))
            }
            Kind::Heating => {
                attributes.insert("device_class".into(), json!("temperature"));
                attributes.insert("unit_of_measurement".into(), json!("°C"));
                if let Some(target) = state("core:TargetTemperatureState").and_then(as_f64) {
                    attributes.insert("target_temperature".into(), json!(target));
                }
                if let Some(mode) = text(state("core:OperatingModeState")) {
                    attributes.insert("mode".into(), json!(mode));
                }
                number(
                    state("core:TemperatureState").or_else(|| state("core:TargetTemperatureState")),
                )
            }
            Kind::Alarm => {
                attributes.insert("device_class".into(), json!("alarm"));
                alarm_mode(tracked)
            }
        }
    } else {
        "unavailable".to_owned()
    };

    if let Err(error) = states.set(tracked.entity_id.clone(), value, attributes) {
        warn!("état refusé pour {} : {error}", tracked.entity_id);
    }
}

/// Mode de l'alarme, d'après le premier état connu parmi les variantes Somfy.
fn alarm_mode(tracked: &Tracked) -> String {
    [
        "internal:CurrentAlarmModeState",
        "myfox:AlarmStatusState",
        "verisure:AlarmPanelMainArmTypeState",
        "core:ActiveZonesState",
    ]
    .into_iter()
    .find_map(|name| text(tracked.states.get(name)))
    .map_or_else(|| "unknown".to_owned(), |v| v.chars().take(64).collect())
}

fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
    .filter(|v| v.is_finite())
}

fn text(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str)
}

fn number(value: Option<&Value>) -> String {
    value.and_then(as_f64).map_or_else(
        || "unknown".to_owned(),
        |v| format!("{}", (v * 10.0).round() / 10.0),
    )
}

fn on_off(value: Option<&str>, on_value: &str) -> String {
    match value {
        Some(v) if v == on_value => "on",
        Some(_) => "off",
        None => "unknown",
    }
    .to_owned()
}

/// Adresse d'un appareil Somfy : `io://1234-5678-9012/1234567`, `rts://…/16756#1`…
///
/// Sécurité : elle vient de la box et repart vers elle dans les commandes ;
/// on la limite au format attendu, morceau par morceau (pas de `..`).
#[must_use]
pub fn is_device_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    let part_ok = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.#".contains(&b))
    };
    url.len() <= 200
        && !scheme.is_empty()
        && scheme
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && rest.split('/').all(part_ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_urls() {
        for ok in [
            "io://1234-5678-9012/1111111",
            "io://1234-5678-9012/3333333#2",
            "rts://1234-5678-9012/16756006",
            "internal://1234-5678-9012/alarm/0",
            "zigbee://1234-5678-9012/abc/def#1",
        ] {
            assert!(is_device_url(ok), "{ok}");
        }
        for bad in [
            "",
            "io:/x",
            "io://x/../../exec",
            "io://x/./y",
            "io://x//y",
            "io://x/y?z",
            "IO://x/y",
            "io://x/y z",
        ] {
            assert!(!is_device_url(bad), "{bad}");
        }
    }
}
