//! Le chauffage Atlantic (pompe à chaleur « Pass APC ») relié à la box Somfy.
//!
//! La box ne le montre pas à son API locale : il est lu et piloté par le
//! serveur Somfy, comme dans l'appli TaHoma (voir [`crate::cloud`] et
//! [`crate::HeatingCloud`]). Cette partie ne touche pas au réseau : elle est
//! testée sans box.
//!
//! | Appareil Atlantic | Entité | Pilotable |
//! |---|---|---|
//! | zone de chauffage (« Etage », « RdC »…) | `climate.<zone>` | consigne, confort/éco/programme, marche/arrêt |
//! | eau chaude (ballon) | `climate.eau_chaude` | consigne, confort/éco/boost, marche/arrêt |
//! | sonde extérieure | `sensor.temperature_exterieure` | non |
//! | pompe à chaleur | `sensor.pompe_a_chaleur` (chauffage, arrêt…) | non |
//! | compteurs d'énergie | `sensor.pac_consommation_*` (kWh) | non |
//!
//! La sonde d'une zone n'est pas une entité à part : sa mesure devient la
//! « température actuelle » de la zone. Comme dans Home Assistant, la sonde
//! est l'appareil qui suit la zone : `…#8` pour la zone, `…#9` pour sa sonde.
//!
//! La consigne d'une zone : si une dérogation est en cours (réglage
//! temporaire fait dans l'appli TaHoma), c'est elle qui change ; sinon,
//! c'est la consigne du mode en cours (confort ou éco). En mode programme,
//! une dérogation est lancée, comme le fait l'appli.

use std::collections::HashMap;

use ha_core::{Attributes, EntityId, StateMachine};
use serde_json::{Value, json};
use tracing::{info, warn};

use crate::api::{Device, Event};
use crate::devices::is_device_url;

/// L'intégration propriétaire des entités du chauffage.
pub const ID: &str = "somfy_chauffage";
/// Son nom affiché.
pub const NAME: &str = "Chauffage Atlantic";

const MAX_LABEL_LEN: usize = 100;
const MAX_DEVICES: usize = 50;
const ZONE_MIN: f64 = 15.0;
const ZONE_MAX: f64 = 26.0;
const WATER_MIN: f64 = 40.0;
const WATER_MAX: f64 = 65.0;

/// Le rôle d'un appareil Atlantic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    HeatPump,
    Zone,
    ZoneSensor,
    HotWater,
    Outside,
    Energy(Meter),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Meter {
    Total,
    Heating,
    HotWater,
}

impl Role {
    /// D'après le `controllableName` Somfy. `None` : pas du chauffage.
    fn of(controllable_name: &str) -> Option<Self> {
        Some(match controllable_name {
            "io:AtlanticPassAPCHeatPumpMainComponent" => Self::HeatPump,
            "io:AtlanticPassAPCZoneTemperatureSensor" => Self::ZoneSensor,
            "io:AtlanticPassAPCDHWComponent" => Self::HotWater,
            "io:AtlanticPassAPCOutsideTemperatureSensor" => Self::Outside,
            "io:TotalElectricalEnergyConsumptionSensor" => Self::Energy(Meter::Total),
            "io:HeatingRelatedElectricalEnergyConsumptionSensor" => Self::Energy(Meter::Heating),
            "io:DHWRelatedElectricalEnergyConsumptionSensor" => Self::Energy(Meter::HotWater),
            name if name.starts_with("io:AtlanticPassAPC") && name.ends_with("ZoneComponent") => {
                Self::Zone
            }
            _ => return None,
        })
    }

    /// Le domaine, le début de l'identifiant et le nom affiché de l'entité.
    /// `None` : pas d'entité (la sonde d'une zone).
    fn entity(self, label: &str) -> Option<(&'static str, String, String)> {
        let (domain, slug, name) = match self {
            Self::Zone => ("climate", ha_core::slugify(label), label.to_owned()),
            Self::ZoneSensor => return None,
            Self::HotWater => ("climate", "eau_chaude".into(), "Eau chaude".into()),
            Self::Outside => (
                "sensor",
                "temperature_exterieure".into(),
                "Température extérieure".into(),
            ),
            Self::HeatPump => ("sensor", "pompe_a_chaleur".into(), "Pompe à chaleur".into()),
            Self::Energy(Meter::Total) => (
                "sensor",
                "pac_consommation_totale".into(),
                "Consommation de la pompe à chaleur".into(),
            ),
            Self::Energy(Meter::Heating) => (
                "sensor",
                "pac_consommation_chauffage".into(),
                "Consommation du chauffage".into(),
            ),
            Self::Energy(Meter::HotWater) => (
                "sensor",
                "pac_consommation_eau_chaude".into(),
                "Consommation de l'eau chaude".into(),
            ),
        };
        let slug = if slug.is_empty() { "zone".into() } else { slug };
        Some((domain, slug, name))
    }
}

/// Un appareil suivi.
#[derive(Debug)]
struct Tracked {
    role: Role,
    name: String,
    entity_id: Option<EntityId>,
    commands: Vec<String>,
    states: HashMap<String, Value>,
    available: bool,
}

impl Tracked {
    fn state(&self, name: &str) -> Option<&Value> {
        self.states.get(name)
    }

    fn text(&self, name: &str) -> Option<&str> {
        self.state(name).and_then(Value::as_str)
    }

    fn number(&self, name: &str) -> Option<f64> {
        self.state(name).and_then(as_f64)
    }

    fn knows(&self, command: &str) -> bool {
        self.commands.iter().any(|c| c == command)
    }
}

/// Un ordre pour le serveur Somfy : une ou plusieurs commandes sur un appareil.
#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    pub device_url: String,
    pub commands: Vec<(String, Vec<Value>)>,
}

/// Correspondance appareils Atlantic ⇄ entités.
#[derive(Debug, Default)]
pub struct Heating {
    by_url: HashMap<String, Tracked>,
    by_entity: HashMap<EntityId, String>,
}

impl Heating {
    /// Combien d'entités sont publiées.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_entity.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_entity.is_empty()
    }

    /// Met à jour toutes les entités à partir de la liste complète des
    /// appareils du compte (volets compris : ils sont laissés de côté, la
    /// box les donne déjà en local).
    pub fn sync(&mut self, devices: Vec<Device>, states: &StateMachine) {
        // Les appareils de la pompe à chaleur partagent le même début
        // d'adresse (`io://…/1234567#2`, `#3`…) : les compteurs d'énergie
        // n'en sont que s'ils sont de la même famille.
        let families: Vec<String> = devices
            .iter()
            .filter(|d| d.controllable_name.starts_with("io:AtlanticPassAPC"))
            .map(|d| family(&d.device_url).to_owned())
            .collect();
        let mut present = Vec::new();
        for device in devices {
            if present.len() >= MAX_DEVICES {
                break;
            }
            let Some(role) = Role::of(&device.controllable_name) else {
                continue;
            };
            if !is_device_url(&device.device_url)
                || !families.iter().any(|f| f == family(&device.device_url))
            {
                continue;
            }
            let label: String = device
                .label
                .chars()
                .filter(|c| !c.is_control())
                .take(MAX_LABEL_LEN)
                .collect();
            let (entity_id, name) = match role.entity(&label) {
                None => (None, label),
                Some((domain, slug, name)) => {
                    let known = self
                        .by_url
                        .get(&device.device_url)
                        .filter(|t| t.role == role)
                        .and_then(|t| t.entity_id.clone());
                    let entity_id = known.or_else(|| {
                        let id = self.allocate_id(domain, &slug, states)?;
                        info!("chauffage : {id} ({name})");
                        Some(id)
                    });
                    (entity_id, name)
                }
            };
            if let Some(entity_id) = &entity_id {
                self.by_entity
                    .insert(entity_id.clone(), device.device_url.clone());
            }
            present.push(device.device_url.clone());
            let tracked = Tracked {
                role,
                name,
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
            self.by_url.insert(device.device_url, tracked);
        }

        let gone: Vec<String> = self
            .by_url
            .keys()
            .filter(|url| !present.contains(url))
            .cloned()
            .collect();
        for url in gone {
            if let Some(entity_id) = self.by_url.remove(&url).and_then(|t| t.entity_id) {
                self.by_entity.remove(&entity_id);
                states.remove(&entity_id);
                info!("chauffage : {entity_id} retiré");
            }
        }
        self.publish_all(states);
    }

    /// Applique les événements du serveur Somfy. Renvoie `true` si la liste
    /// des appareils a changé : il faut alors tout relire.
    pub fn apply_events(&mut self, events: &[Event], states: &StateMachine) -> bool {
        let mut reload = false;
        let mut changed = false;
        for event in events {
            let tracked = event
                .device_url
                .as_ref()
                .and_then(|url| self.by_url.get_mut(url));
            match (event.name.as_str(), tracked) {
                ("DeviceCreatedEvent" | "DeviceRemovedEvent", _) => reload = true,
                ("DeviceStateChangedEvent", Some(tracked)) => {
                    for state in &event.device_states {
                        tracked
                            .states
                            .insert(state.name.clone(), state.value.clone());
                    }
                    changed = true;
                }
                ("DeviceAvailableEvent", Some(tracked)) => {
                    tracked.available = true;
                    changed = true;
                }
                ("DeviceUnavailableEvent", Some(tracked)) => {
                    tracked.available = false;
                    changed = true;
                }
                _ => {}
            }
        }
        // Une sonde change la zone qu'elle suit : on republie tout (une
        // dizaine d'entités ; celles qui n'ont pas changé ne bougent pas).
        if changed {
            self.publish_all(states);
        }
        reload
    }

    /// Traduit un appel de service en ordre pour le serveur Somfy.
    #[must_use]
    pub fn command(
        &self,
        domain: &str,
        service: &str,
        entity_id: &EntityId,
        data: &Attributes,
    ) -> Option<Order> {
        let url = self.by_entity.get(entity_id)?;
        let tracked = self.by_url.get(url)?;
        if domain != "climate" {
            warn!("chauffage : service {domain}.{service} impossible sur {entity_id}");
            return None;
        }
        let commands = match tracked.role {
            Role::Zone => zone_command(tracked, service, data),
            Role::HotWater => water_command(tracked, service, data),
            _ => None,
        };
        let Some(commands) = commands.filter(|c| !c.is_empty()) else {
            warn!("chauffage : climate.{service} impossible sur {entity_id}");
            return None;
        };
        if let Some((name, _)) = commands.iter().find(|(name, _)| !tracked.knows(name)) {
            warn!(
                "chauffage : {} ne connaît pas la commande « {name} »",
                tracked.name
            );
            return None;
        }
        Some(Order {
            device_url: url.clone(),
            commands,
        })
    }

    /// Un identifiant libre : ni à nous, ni à une autre intégration.
    fn allocate_id(&self, domain: &str, slug: &str, states: &StateMachine) -> Option<EntityId> {
        (1..=MAX_DEVICES).find_map(|n| {
            let candidate = if n == 1 {
                format!("{domain}.{slug}")
            } else {
                format!("{domain}.{slug}_{n}")
            };
            EntityId::new(&candidate).ok().filter(|id| {
                !self.by_entity.contains_key(id)
                    && states.owner_of(id).is_none_or(|owner| owner == ID)
            })
        })
    }

    fn publish_all(&self, states: &StateMachine) {
        for (url, tracked) in &self.by_url {
            let Some(entity_id) = &tracked.entity_id else {
                continue;
            };
            // La sonde d'une zone : l'appareil suivant (`#8` → `#9`).
            let sensor = (tracked.role == Role::Zone)
                .then(|| next_device(url))
                .flatten()
                .and_then(|next| self.by_url.get(&next))
                .filter(|s| s.role == Role::ZoneSensor && s.available);
            let (value, attributes) = entity_state(tracked, sensor);
            if let Err(error) = states.set_by(ID, entity_id.clone(), value, attributes) {
                warn!("état refusé pour {entity_id} : {error}");
            }
        }
    }
}

/// La valeur et les attributs d'une entité.
fn entity_state(tracked: &Tracked, sensor: Option<&Tracked>) -> (String, Attributes) {
    let mut a = Attributes::new();
    a.insert("friendly_name".into(), json!(tracked.name));
    if !tracked.available {
        return (ha_core::integrations::UNAVAILABLE.to_owned(), a);
    }
    let value = match tracked.role {
        Role::Zone => zone_state(tracked, sensor, &mut a),
        Role::HotWater => water_state(tracked, &mut a),
        Role::Outside | Role::ZoneSensor => {
            a.insert("device_class".into(), json!("temperature"));
            a.insert("unit_of_measurement".into(), json!("°C"));
            a.insert("state_class".into(), json!("measurement"));
            rounded(tracked.number("core:TemperatureState"))
        }
        Role::HeatPump => {
            let mode = tracked.text("io:PassAPCOperatingModeState");
            if let Some(code) = tracked
                .number("core:ErrorCodeState")
                .filter(|code| *code != 0.0)
            {
                a.insert("code_erreur".into(), json!(code));
            }
            match mode {
                Some("heating") => "chauffage",
                Some("cooling") => "climatisation",
                Some("drying") => "déshumidification",
                Some("stop") => "arrêt",
                Some(_) => "autre",
                None => "unknown",
            }
            .to_owned()
        }
        Role::Energy(_) => {
            a.insert("device_class".into(), json!("energy"));
            a.insert("unit_of_measurement".into(), json!("kWh"));
            a.insert("state_class".into(), json!("total_increasing"));
            // Le compteur est en Wh.
            rounded(
                tracked
                    .number("core:ElectricEnergyConsumptionState")
                    .map(|wh| wh / 1000.0),
            )
        }
    };
    (value, a)
}

fn zone_state(tracked: &Tracked, sensor: Option<&Tracked>, a: &mut Attributes) -> String {
    let mode = tracked.text("io:PassAPCHeatingModeState");
    let on = tracked.text("core:HeatingOnOffState") != Some("off") && !matches!(mode, Some("stop"));
    if let Some(target) = tracked.number("core:TargetTemperatureState") {
        a.insert("temperature".into(), json!(target));
    }
    if let Some(current) = sensor.and_then(|s| s.number("core:TemperatureState")) {
        a.insert("current_temperature".into(), json!(current));
    }
    for (state, attribute) in [
        (
            "core:ComfortHeatingTargetTemperatureState",
            "comfort_temperature",
        ),
        ("core:EcoHeatingTargetTemperatureState", "eco_temperature"),
    ] {
        if let Some(value) = tracked.number(state) {
            a.insert(attribute.into(), json!(value));
        }
    }
    if derogation_on(tracked) {
        a.insert("derogation".into(), json!(true));
        if let Some(minutes) = tracked
            .number("io:DerogationRemainingTimeState")
            .filter(|m| *m > 0.0)
        {
            a.insert("derogation_minutes".into(), json!(minutes.round()));
        }
    }
    a.insert("hvac_modes".into(), json!(["off", "heat"]));
    a.insert(
        "preset_modes".into(),
        json!(["confort", "eco", "programme"]),
    );
    if let Some(preset) = mode.and_then(zone_preset) {
        a.insert("preset_mode".into(), json!(preset));
    }
    a.insert("min_temp".into(), json!(ZONE_MIN));
    a.insert("max_temp".into(), json!(ZONE_MAX));
    a.insert("target_temp_step".into(), json!(0.5));
    a.insert("unit_of_measurement".into(), json!("°C"));
    if on { "heat" } else { "off" }.to_owned()
}

fn water_state(tracked: &Tracked, a: &mut Attributes) -> String {
    let mode = tracked.text("io:PassAPCDHWModeState");
    let on = tracked.text("core:DHWOnOffState") != Some("off") && !matches!(mode, Some("stop"));
    if let Some(target) = tracked.number("core:TargetDHWTemperatureState") {
        a.insert("temperature".into(), json!(target));
    }
    for (state, attribute) in [
        (
            "core:ComfortTargetDHWTemperatureState",
            "comfort_temperature",
        ),
        ("core:EcoTargetDHWTemperatureState", "eco_temperature"),
    ] {
        if let Some(value) = tracked.number(state) {
            a.insert(attribute.into(), json!(value));
        }
    }
    a.insert("hvac_modes".into(), json!(["off", "heat"]));
    a.insert("preset_modes".into(), json!(["confort", "eco", "boost"]));
    let preset = if boost_on(tracked) {
        Some("boost")
    } else {
        mode.and_then(zone_preset)
    };
    if let Some(preset) = preset {
        a.insert("preset_mode".into(), json!(preset));
    }
    a.insert("min_temp".into(), json!(WATER_MIN));
    a.insert("max_temp".into(), json!(WATER_MAX));
    a.insert("target_temp_step".into(), json!(1));
    a.insert("unit_of_measurement".into(), json!("°C"));
    if on { "heat" } else { "off" }.to_owned()
}

/// Le préréglage affiché, d'après le mode Atlantic.
fn zone_preset(mode: &str) -> Option<&'static str> {
    Some(match mode {
        "comfort" | "manu" => "confort",
        "eco" => "eco",
        "internalScheduling" | "externalScheduling" | "auto" => "programme",
        "absence" => "absent",
        _ => return None,
    })
}

fn derogation_on(tracked: &Tracked) -> bool {
    tracked.text("core:DerogationOnOffState") == Some("on")
}

fn boost_on(tracked: &Tracked) -> bool {
    tracked.text("core:BoostOnOffState") == Some("on")
}

type Commands = Vec<(String, Vec<Value>)>;

fn cmd(name: &str, parameter: Value) -> (String, Vec<Value>) {
    (name.to_owned(), vec![parameter])
}

/// La consigne demandée, arrondie au pas et bornée.
fn wanted(data: &Attributes, step: f64, min: f64, max: f64) -> Option<f64> {
    let value = data.get("temperature").and_then(as_f64)?;
    Some(((value / step).round() * step).clamp(min, max))
}

fn zone_command(tracked: &Tracked, service: &str, data: &Attributes) -> Option<Commands> {
    let mode = tracked.text("io:PassAPCHeatingModeState").unwrap_or("");
    Some(match service {
        "set_temperature" => {
            let t = json!(wanted(data, 0.5, ZONE_MIN, ZONE_MAX)?);
            if derogation_on(tracked) {
                vec![cmd("setDerogatedTargetTemperature", t)]
            } else {
                match zone_preset(mode) {
                    Some("eco") => vec![cmd("setEcoHeatingTargetTemperature", t)],
                    Some("confort") => vec![cmd("setComfortHeatingTargetTemperature", t)],
                    // Programme (ou mode inconnu) : une dérogation, comme l'appli.
                    _ => vec![
                        cmd("setDerogatedTargetTemperature", t),
                        cmd("setDerogationOnOffState", json!("on")),
                    ],
                }
            }
        }
        "set_preset_mode" => {
            let mode = match data.get("preset_mode").and_then(Value::as_str)? {
                "confort" => "comfort",
                "eco" => "eco",
                "programme" => "internalScheduling",
                _ => return None,
            };
            vec![cmd("setPassAPCHeatingMode", json!(mode))]
        }
        "set_hvac_mode" => match data.get("hvac_mode").and_then(Value::as_str)? {
            "off" => vec![cmd("setHeatingOnOffState", json!("off"))],
            "heat" => {
                let mut commands = vec![cmd("setHeatingOnOffState", json!("on"))];
                if mode == "stop" {
                    commands.push(cmd("setPassAPCHeatingMode", json!("comfort")));
                }
                commands
            }
            _ => return None,
        },
        "turn_off" => vec![cmd("setHeatingOnOffState", json!("off"))],
        "turn_on" => vec![cmd("setHeatingOnOffState", json!("on"))],
        _ => return None,
    })
}

fn water_command(tracked: &Tracked, service: &str, data: &Attributes) -> Option<Commands> {
    let mode = tracked.text("io:PassAPCDHWModeState").unwrap_or("");
    Some(match service {
        "set_temperature" => {
            let t = json!(wanted(data, 1.0, WATER_MIN, WATER_MAX)?);
            if zone_preset(mode) == Some("eco") {
                vec![cmd("setEcoTargetDHWTemperature", t)]
            } else {
                vec![cmd("setComfortTargetDHWTemperature", t)]
            }
        }
        "set_preset_mode" => {
            let wanted = data.get("preset_mode").and_then(Value::as_str)?;
            if wanted == "boost" {
                vec![cmd("setBoostOnOffState", json!("on"))]
            } else {
                let mode = match wanted {
                    "confort" => "comfort",
                    "eco" => "eco",
                    _ => return None,
                };
                let mut commands = Vec::new();
                if boost_on(tracked) {
                    commands.push(cmd("setBoostOnOffState", json!("off")));
                }
                commands.push(cmd("setPassAPCDHWMode", json!(mode)));
                commands
            }
        }
        "set_hvac_mode" => match data.get("hvac_mode").and_then(Value::as_str)? {
            "off" => vec![cmd("setDHWOnOffState", json!("off"))],
            "heat" => vec![cmd("setDHWOnOffState", json!("on"))],
            _ => return None,
        },
        "turn_off" => vec![cmd("setDHWOnOffState", json!("off"))],
        "turn_on" => vec![cmd("setDHWOnOffState", json!("on"))],
        _ => return None,
    })
}

/// Le début commun des adresses d'une même pompe à chaleur (avant `#`).
fn family(url: &str) -> &str {
    url.split_once('#').map_or(url, |(base, _)| base)
}

/// L'adresse de l'appareil suivant : `…#8` → `…#9`.
fn next_device(url: &str) -> Option<String> {
    let (base, index) = url.rsplit_once('#')?;
    let index: u32 = index.parse().ok()?;
    Some(format!("{base}#{}", index.checked_add(1)?))
}

fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
    .filter(|v| v.is_finite())
}

/// Une valeur arrondie au dixième (`unknown` si absente).
fn rounded(value: Option<f64>) -> String {
    value.map_or_else(
        || "unknown".to_owned(),
        |v| format!("{}", (v * 10.0).round() / 10.0),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    // Des adresses inventées (aucun identifiant de la vraie maison).
    const BASE: &str = "io://1234-5678-9012/7654321";

    fn device(suffix: &str, label: &str, controllable: &str, states: Value) -> Device {
        let commands: Vec<Value> = [
            "setComfortHeatingTargetTemperature",
            "setEcoHeatingTargetTemperature",
            "setDerogatedTargetTemperature",
            "setDerogationOnOffState",
            "setHeatingOnOffState",
            "setPassAPCHeatingMode",
            "setComfortTargetDHWTemperature",
            "setEcoTargetDHWTemperature",
            "setBoostOnOffState",
            "setDHWOnOffState",
            "setPassAPCDHWMode",
        ]
        .iter()
        .map(|name| json!({"commandName": name}))
        .collect();
        let Value::Object(states) = states else {
            panic!("les états d'essai sont un objet");
        };
        let states: Vec<Value> = states
            .into_iter()
            .map(|(name, value)| json!({"name": name, "value": value}))
            .collect();
        serde_json::from_value(json!({
            "deviceURL": format!("{BASE}{suffix}"),
            "label": label,
            "controllableName": controllable,
            "definition": {"uiClass": "HeatingSystem", "commands": commands},
            "states": states,
        }))
        .unwrap()
    }

    fn house() -> Vec<Device> {
        vec![
            device(
                "#1",
                "HP_Actuator",
                "io:AtlanticPassAPCHeatPumpMainComponent",
                json!({"io:PassAPCOperatingModeState": "heating"}),
            ),
            device(
                "#2",
                "IO (7654321#2)",
                "io:AtlanticPassAPCDHWComponent",
                json!({
                    "io:PassAPCDHWModeState": "comfort",
                    "core:TargetDHWTemperatureState": 55,
                    "core:BoostOnOffState": "off",
                    "core:DHWOnOffState": "on",
                }),
            ),
            device(
                "#3",
                "IO (7654321#3)",
                "io:AtlanticPassAPCOutsideTemperatureSensor",
                json!({"core:TemperatureState": 13.8}),
            ),
            device(
                "#4",
                "IO (7654321#4)",
                "io:TotalElectricalEnergyConsumptionSensor",
                json!({"core:ElectricEnergyConsumptionState": 3_792_000}),
            ),
            device(
                "#8",
                "Etage",
                "io:AtlanticPassAPCHeatingAndCoolingZoneComponent",
                json!({
                    "io:PassAPCHeatingModeState": "comfort",
                    "core:HeatingOnOffState": "on",
                    "core:TargetTemperatureState": 19.0,
                    "core:ComfortHeatingTargetTemperatureState": 18.5,
                    "core:DerogationOnOffState": "on",
                    "io:DerogationRemainingTimeState": 117,
                }),
            ),
            device(
                "#9",
                "IO (7654321#9)",
                "io:AtlanticPassAPCZoneTemperatureSensor",
                json!({"core:TemperatureState": 18.8}),
            ),
            device(
                "#10",
                "RdC",
                "io:AtlanticPassAPCHeatingAndCoolingZoneComponent",
                json!({
                    "io:PassAPCHeatingModeState": "comfort",
                    "core:HeatingOnOffState": "on",
                    "core:TargetTemperatureState": 19.5,
                    "core:DerogationOnOffState": "off",
                }),
            ),
            device(
                "#11",
                "IO (7654321#11)",
                "io:AtlanticPassAPCZoneTemperatureSensor",
                json!({"core:TemperatureState": 18.1}),
            ),
            // Un volet du même compte : ignoré ici.
            device(
                "",
                "volet Salon",
                "io:RollerShutterGenericIOComponent",
                json!({}),
            ),
            // Un compteur d'un autre appareil : ignoré.
            serde_json::from_value(json!({
                "deviceURL": "io://1234-5678-9012/1111111#4",
                "label": "autre",
                "controllableName": "io:TotalElectricalEnergyConsumptionSensor",
            }))
            .unwrap(),
        ]
    }

    fn id(text: &str) -> EntityId {
        text.parse().unwrap()
    }

    fn loaded() -> (Heating, StateMachine) {
        let states = StateMachine::new(ha_core::EventBus::default());
        let mut heating = Heating::default();
        heating.sync(house(), &states);
        (heating, states)
    }

    fn data(value: Value) -> Attributes {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn the_heat_pump_becomes_entities() {
        let (heating, states) = loaded();
        let mut ids = states.owned_by(ID);
        ids.sort();
        let names: Vec<String> = ids.iter().map(ToString::to_string).collect();
        assert_eq!(
            names,
            [
                "climate.eau_chaude",
                "climate.etage",
                "climate.rdc",
                "sensor.pac_consommation_totale",
                "sensor.pompe_a_chaleur",
                "sensor.temperature_exterieure",
            ]
        );
        assert_eq!(heating.len(), 6);

        let etage = states.get(&id("climate.etage")).unwrap();
        assert_eq!(etage.state(), "heat");
        let a = etage.attributes();
        assert_eq!(a["temperature"], json!(19.0));
        // La sonde qui suit la zone (#8 → #9).
        assert_eq!(a["current_temperature"], json!(18.8));
        assert_eq!(a["preset_mode"], json!("confort"));
        assert_eq!(a["derogation_minutes"], json!(117.0));
        let rdc = states.get(&id("climate.rdc")).unwrap();
        assert_eq!(rdc.attributes()["current_temperature"], json!(18.1));
        assert!(!rdc.attributes().contains_key("derogation"));

        let outside = states.get(&id("sensor.temperature_exterieure")).unwrap();
        assert_eq!(outside.state(), "13.8");
        let energy = states.get(&id("sensor.pac_consommation_totale")).unwrap();
        assert_eq!(energy.state(), "3792");
        let pump = states.get(&id("sensor.pompe_a_chaleur")).unwrap();
        assert_eq!(pump.state(), "chauffage");
        let water = states.get(&id("climate.eau_chaude")).unwrap();
        assert_eq!(water.state(), "heat");
        assert_eq!(water.attributes()["temperature"], json!(55.0));
        assert_eq!(water.attributes()["preset_mode"], json!("confort"));
    }

    #[test]
    fn events_update_the_zone_through_its_sensor() {
        let (mut heating, states) = loaded();
        let events: Vec<Event> = serde_json::from_value(json!([
            {"name": "DeviceStateChangedEvent", "deviceURL": format!("{BASE}#9"),
             "deviceStates": [{"name": "core:TemperatureState", "value": "19.4"}]},
        ]))
        .unwrap();
        assert!(!heating.apply_events(&events, &states));
        let etage = states.get(&id("climate.etage")).unwrap();
        assert_eq!(etage.attributes()["current_temperature"], json!(19.4));
    }

    #[test]
    fn setpoint_follows_the_current_mode() {
        let (heating, _) = loaded();
        let set = |entity: &str, temperature: f64| {
            heating
                .command(
                    "climate",
                    "set_temperature",
                    &id(entity),
                    &data(json!({"temperature": temperature})),
                )
                .unwrap()
                .commands
        };
        // Dérogation en cours : c'est elle qui change.
        assert_eq!(
            set("climate.etage", 19.5),
            [cmd("setDerogatedTargetTemperature", json!(19.5))]
        );
        // Sinon, la consigne confort (arrondie au demi-degré, bornée).
        assert_eq!(
            set("climate.rdc", 20.26),
            [cmd("setComfortHeatingTargetTemperature", json!(20.5))]
        );
        assert_eq!(
            set("climate.rdc", 40.0),
            [cmd("setComfortHeatingTargetTemperature", json!(ZONE_MAX))]
        );
        assert_eq!(
            set("climate.eau_chaude", 52.4),
            [cmd("setComfortTargetDHWTemperature", json!(52.0))]
        );
        let order = heating
            .command(
                "climate",
                "set_temperature",
                &id("climate.rdc"),
                &data(json!({"temperature": 20})),
            )
            .unwrap();
        assert_eq!(order.device_url, format!("{BASE}#10"));
    }

    #[test]
    fn presets_and_power() {
        let (heating, _) = loaded();
        let call = |entity: &str, service: &str, value: Value| {
            heating
                .command("climate", service, &id(entity), &data(value))
                .map(|o| o.commands)
        };
        assert_eq!(
            call(
                "climate.rdc",
                "set_preset_mode",
                json!({"preset_mode": "programme"})
            ),
            Some(vec![cmd(
                "setPassAPCHeatingMode",
                json!("internalScheduling")
            )])
        );
        assert_eq!(
            call("climate.rdc", "set_hvac_mode", json!({"hvac_mode": "off"})),
            Some(vec![cmd("setHeatingOnOffState", json!("off"))])
        );
        assert_eq!(
            call(
                "climate.eau_chaude",
                "set_preset_mode",
                json!({"preset_mode": "boost"})
            ),
            Some(vec![cmd("setBoostOnOffState", json!("on"))])
        );
        assert_eq!(
            call(
                "climate.rdc",
                "set_preset_mode",
                json!({"preset_mode": "nuit"})
            ),
            None
        );
        // Les capteurs ne se pilotent pas.
        assert!(
            heating
                .command(
                    "sensor",
                    "turn_on",
                    &id("sensor.temperature_exterieure"),
                    &data(json!({}))
                )
                .is_none()
        );
    }

    #[test]
    fn removed_devices_disappear() {
        let (mut heating, states) = loaded();
        let fewer: Vec<Device> = house().into_iter().filter(|d| d.label != "RdC").collect();
        heating.sync(fewer, &states);
        assert!(states.get(&id("climate.rdc")).is_none());
        assert!(states.get(&id("climate.etage")).is_some());
    }

    #[test]
    fn addresses() {
        assert_eq!(next_device("io://x/1#8").as_deref(), Some("io://x/1#9"));
        assert_eq!(next_device("io://x/1"), None);
        assert_eq!(family("io://x/1#8"), "io://x/1");
    }
}
