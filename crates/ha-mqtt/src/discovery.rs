//! Découverte automatique façon Home Assistant.
//!
//! Un appareil (ou Zigbee2MQTT, Tasmota…) publie sa description sur un sujet
//! `homeassistant/<type>/[<nœud>/]<objet>/config`, par exemple :
//!
//! ```json
//! {"name": "Température salon", "stat_t": "zigbee2mqtt/salon",
//!  "val_tpl": "{{ value_json.temperature }}", "unit_of_meas": "°C"}
//! ```
//!
//! On en déduit une entité (`sensor.temperature_salon`), le sujet à écouter
//! pour son état, et le sujet où envoyer les ordres.
//!
//! Sécurité : tout ce qui arrive par MQTT vient du réseau. Chaque champ est
//! vérifié et limité ; ce qui n'est pas compris est refusé, pas deviné.

use serde_json::{Map, Value, json};

use crate::profile::{AttributeTopic, Payload, Profile, ServiceCommand};
use crate::template::{Segment, Template};

/// Taille maximale d'une description de découverte.
pub const MAX_CONFIG_SIZE: usize = 16 * 1024;
/// Longueur maximale d'un sujet MQTT accepté.
const MAX_TOPIC_LEN: usize = 256;
/// Longueur maximale d'un texte (nom, unité, valeur de commande…).
const MAX_TEXT_LEN: usize = 255;

/// Types d'entités pris en charge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Component {
    Sensor,
    BinarySensor,
    Switch,
    Light,
    Lock,
    Siren,
    Valve,
    Vacuum,
    Humidifier,
    WaterHeater,
}

impl Component {
    fn from_str(value: &str) -> Option<Self> {
        match value {
            "sensor" => Some(Self::Sensor),
            "binary_sensor" => Some(Self::BinarySensor),
            "switch" => Some(Self::Switch),
            "light" => Some(Self::Light),
            "lock" => Some(Self::Lock),
            "siren" => Some(Self::Siren),
            "valve" => Some(Self::Valve),
            "vacuum" => Some(Self::Vacuum),
            "humidifier" => Some(Self::Humidifier),
            "water_heater" => Some(Self::WaterHeater),
            _ => None,
        }
    }

    /// Le domaine de l'identifiant d'entité : `sensor`, `light`…
    #[must_use]
    pub fn domain(self) -> &'static str {
        match self {
            Self::Sensor => "sensor",
            Self::BinarySensor => "binary_sensor",
            Self::Switch => "switch",
            Self::Light => "light",
            Self::Lock => "lock",
            Self::Siren => "siren",
            Self::Valve => "valve",
            Self::Vacuum => "vacuum",
            Self::Humidifier => "humidifier",
            Self::WaterHeater => "water_heater",
        }
    }

    /// Son état est « on » / « off » (et `turn_on` / `turn_off` le commandent).
    #[must_use]
    pub fn is_on_off(self) -> bool {
        matches!(
            self,
            Self::BinarySensor | Self::Switch | Self::Light | Self::Siren | Self::Humidifier
        )
    }
}

/// Une entité décrite par un message de découverte, déjà vérifiée.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityConfig {
    pub component: Component,
    /// Nom suggéré pour l'identifiant (sans le domaine), avant nettoyage.
    pub object_id: String,
    pub friendly_name: Option<String>,
    pub state_topic: Option<String>,
    pub command_topic: Option<String>,
    pub value_template: Template,
    /// Valeurs envoyées pour allumer / éteindre (`ON` / `OFF` par défaut).
    pub payload_on: String,
    pub payload_off: String,
    /// Valeurs reçues signifiant allumé / éteint (par défaut, les précédentes).
    pub state_on: String,
    pub state_off: String,
    pub unit_of_measurement: Option<String>,
    pub device_class: Option<String>,
    /// Lumière au format JSON (Zigbee2MQTT) : `{"state": "ON", "brightness": 128}`.
    pub json_schema: bool,
    /// Valeur maximale de luminosité côté appareil (255 par défaut).
    pub brightness_scale: u32,
    /// Ce que sait faire un verrou, une vanne, un aspirateur… (vide sinon).
    pub profile: Profile,
}

/// Résultat de l'analyse d'un message sur un sujet de découverte.
#[derive(Debug, Clone, PartialEq)]
pub enum Discovery {
    /// Une entité est annoncée ou mise à jour.
    Config(Box<EntityConfig>),
    /// Message vide : l'entité est retirée.
    Remove,
}

/// Erreur d'analyse : le message est ignoré et l'erreur apparaît dans les logs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DiscoveryError {
    #[error("type d'entité non pris en charge : {0}")]
    UnsupportedComponent(String),
    #[error("description trop grosse ({0} octets)")]
    TooLarge(usize),
    #[error("JSON invalide")]
    InvalidJson,
    #[error("champ « {0} » invalide")]
    InvalidField(&'static str),
    #[error("modèle de valeur non pris en charge : {0:?}")]
    UnsupportedTemplate(String),
    #[error("il faut au moins un sujet d'état ou de commande")]
    NoTopic,
}

/// Abréviations utilisées dans les descriptions (Zigbee2MQTT et Tasmota en
/// abusent pour économiser de la place). On ne retient que celles qu'on utilise.
const ABBREVIATIONS: &[(&str, &str)] = &[
    ("stat_t", "state_topic"),
    ("cmd_t", "command_topic"),
    ("val_tpl", "value_template"),
    ("stat_val_tpl", "state_value_template"),
    ("pl_on", "payload_on"),
    ("pl_off", "payload_off"),
    ("stat_on", "state_on"),
    ("stat_off", "state_off"),
    ("uniq_id", "unique_id"),
    ("obj_id", "object_id"),
    ("unit_of_meas", "unit_of_measurement"),
    ("dev_cla", "device_class"),
    ("bri_scl", "brightness_scale"),
    ("dev", "device"),
    // Verrou, vanne, aspirateur.
    ("pl_lock", "payload_lock"),
    ("pl_unlk", "payload_unlock"),
    ("pl_open", "payload_open"),
    ("pl_cls", "payload_close"),
    ("pl_stop", "payload_stop"),
    ("stat_locked", "state_locked"),
    ("stat_unlocked", "state_unlocked"),
    ("stat_locking", "state_locking"),
    ("stat_unlocking", "state_unlocking"),
    ("stat_jam", "state_jammed"),
    ("stat_open", "state_open"),
    ("stat_opening", "state_opening"),
    ("stat_closed", "state_closed"),
    ("stat_clsing", "state_closing"),
    ("pl_strt", "payload_start"),
    ("pl_paus", "payload_pause"),
    ("pl_ret", "payload_return_to_base"),
    ("pl_loc", "payload_locate"),
    ("pl_cln_sp", "payload_clean_spot"),
    // Humidificateur.
    ("hum_cmd_t", "target_humidity_command_topic"),
    ("hum_stat_t", "target_humidity_state_topic"),
    ("hum_state_tpl", "target_humidity_state_template"),
    ("curr_hum_t", "current_humidity_topic"),
    ("curr_hum_tpl", "current_humidity_template"),
    ("min_hum", "min_humidity"),
    ("max_hum", "max_humidity"),
    // Chauffe-eau.
    ("mode_cmd_t", "mode_command_topic"),
    ("mode_stat_t", "mode_state_topic"),
    ("mode_stat_tpl", "mode_state_template"),
    ("temp_cmd_t", "temperature_command_topic"),
    ("temp_stat_t", "temperature_state_topic"),
    ("temp_stat_tpl", "temperature_state_template"),
    ("curr_temp_t", "current_temperature_topic"),
    ("curr_temp_tpl", "current_temperature_template"),
    ("pow_cmd_t", "power_command_topic"),
];

/// Les modes d'un chauffe-eau quand l'appareil n'en annonce pas.
const WATER_HEATER_MODES: [&str; 7] = [
    "off",
    "eco",
    "electric",
    "gas",
    "heat_pump",
    "high_demand",
    "performance",
];
/// Les états d'un aspirateur (ceux de Home Assistant).
const VACUUM_STATES: [&str; 6] = ["cleaning", "docked", "paused", "idle", "returning", "error"];
/// Nombre maximal de modes annoncés par un appareil.
const MAX_OPTIONS: usize = 20;

/// Découpe un sujet de découverte. Renvoie le type et l'identifiant d'objet,
/// ou `None` si ce n'est pas un sujet de découverte.
///
/// Formats : `<préfixe>/<type>/<objet>/config` ou `<préfixe>/<type>/<nœud>/<objet>/config`.
#[must_use]
pub fn parse_topic<'a>(prefix: &str, topic: &'a str) -> Option<(&'a str, &'a str)> {
    let rest = topic.strip_prefix(prefix)?.strip_prefix('/')?;
    let rest = rest.strip_suffix("/config")?;
    let parts: Vec<&str> = rest.split('/').collect();
    match parts.as_slice() {
        [component, object] | [component, _, object] => Some((component, object)),
        _ => None,
    }
}

/// Analyse un message reçu sur un sujet de découverte.
pub fn parse(
    prefix: &str,
    component: &str,
    object_id: &str,
    payload: &[u8],
) -> Result<Discovery, DiscoveryError> {
    if payload.is_empty() {
        return Ok(Discovery::Remove);
    }
    let component = Component::from_str(component)
        .ok_or_else(|| DiscoveryError::UnsupportedComponent(component.to_owned()))?;
    if payload.len() > MAX_CONFIG_SIZE {
        return Err(DiscoveryError::TooLarge(payload.len()));
    }
    let Ok(Value::Object(raw)) = serde_json::from_slice::<Value>(payload) else {
        return Err(DiscoveryError::InvalidJson);
    };
    let config = expand(raw);

    // `~` : préfixe commun des sujets, remplacé au début ou à la fin.
    let base = text(&config, "~")?;
    let topic = |key: &'static str| -> Result<Option<String>, DiscoveryError> {
        let Some(value) = text(&config, key)? else {
            return Ok(None);
        };
        let value = match &base {
            Some(base) if value.starts_with('~') => format!("{base}{}", &value[1..]),
            Some(base) if value.ends_with('~') => {
                format!("{}{base}", &value[..value.len() - 1])
            }
            _ => value,
        };
        check_topic(&value, prefix, key).map(Some)
    };
    // Un chauffe-eau n'a pas de sujet d'état : son état, c'est son mode.
    let (state_key, command_key) = match component {
        Component::WaterHeater => ("mode_state_topic", "mode_command_topic"),
        _ => ("state_topic", "command_topic"),
    };
    let state_topic = topic(state_key)?;
    let command_topic = topic(command_key)?;

    // Pour une lumière, le modèle de l'état s'appelle `state_value_template`.
    let template_key = match component {
        Component::Light => "state_value_template",
        Component::WaterHeater => "mode_state_template",
        _ => "value_template",
    };
    let value_template = match (template(&config, template_key)?, component) {
        (Some(template), _) => template,
        // Un aspirateur envoie `{"state": "cleaning", "battery_level": 61}`.
        (None, Component::Vacuum) => Template::Json(vec![Segment::Key("state".into())]),
        (None, _) => Template::Raw,
    };

    let payload_on = text(&config, "payload_on")?.unwrap_or_else(|| "ON".to_owned());
    let payload_off = text(&config, "payload_off")?.unwrap_or_else(|| "OFF".to_owned());
    let profile = profile(
        component,
        &config,
        &topic,
        state_topic.as_deref(),
        &payload_on,
        &payload_off,
    )?;
    if state_topic.is_none() && command_topic.is_none() && profile.services.is_empty() {
        return Err(DiscoveryError::NoTopic);
    }
    let json_schema = match text(&config, "schema")?.as_deref() {
        None | Some("default") => false,
        Some("json") => true,
        Some(_) => return Err(DiscoveryError::InvalidField("schema")),
    };
    let brightness_scale = match config.get("brightness_scale") {
        None => 255,
        Some(value) => value
            .as_u64()
            .and_then(|scale| u32::try_from(scale).ok())
            .filter(|scale| (1..=65_535).contains(scale))
            .ok_or(DiscoveryError::InvalidField("brightness_scale"))?,
    };

    // Nom affiché : « <appareil> <entité> », comme Home Assistant.
    let device_name = config
        .get("device")
        .and_then(|device| device.get("name"))
        .and_then(Value::as_str)
        .map(|name| name.chars().take(MAX_TEXT_LEN).collect::<String>());
    let friendly_name = match (device_name, text(&config, "name")?) {
        (Some(device), Some(name)) if !name.starts_with(&device) => {
            Some(format!("{device} {name}"))
        }
        (_, Some(name)) => Some(name),
        (device, None) => device,
    };

    // Identifiant suggéré : `object_id` explicite, sinon le nom, sinon le sujet.
    let suggested = text(&config, "object_id")?
        .or_else(|| friendly_name.clone())
        .unwrap_or_else(|| object_id.to_owned());

    Ok(Discovery::Config(Box::new(EntityConfig {
        component,
        object_id: suggested,
        friendly_name,
        state_topic,
        command_topic,
        value_template,
        state_on: text(&config, "state_on")?.unwrap_or_else(|| payload_on.clone()),
        state_off: text(&config, "state_off")?.unwrap_or_else(|| payload_off.clone()),
        payload_on,
        payload_off,
        unit_of_measurement: text(&config, "unit_of_measurement")?,
        device_class: text(&config, "device_class")?,
        json_schema: json_schema && component == Component::Light,
        brightness_scale,
        profile,
    })))
}

/// Ce que sait faire un verrou, une vanne, un aspirateur, un humidificateur
/// ou un chauffe-eau, avec les valeurs par défaut de Home Assistant.
fn profile(
    component: Component,
    config: &Map<String, Value>,
    topic: &dyn Fn(&'static str) -> Result<Option<String>, DiscoveryError>,
    state_topic: Option<&str>,
    payload_on: &str,
    payload_off: &str,
) -> Result<Profile, DiscoveryError> {
    let mut profile = Profile::default();
    // `état` ← `state_xxx` reçu (ou la valeur par défaut).
    let mut map_state = |key: &'static str, default: &str, state: &str| {
        text(config, key).map(|value| {
            let received = value.unwrap_or_else(|| default.to_owned());
            profile.states.push((received, state.to_owned()));
        })
    };
    match component {
        Component::Lock => {
            map_state("state_locked", "LOCKED", "locked")?;
            map_state("state_unlocked", "UNLOCKED", "unlocked")?;
            map_state("state_locking", "LOCKING", "locking")?;
            map_state("state_unlocking", "UNLOCKING", "unlocking")?;
            map_state("state_jammed", "JAMMED", "jammed")?;
            map_state("state_open", "OPEN", "open")?;
            map_state("state_opening", "OPENING", "opening")?;
        }
        Component::Valve => {
            map_state("state_open", "open", "open")?;
            map_state("state_closed", "closed", "closed")?;
            map_state("state_opening", "opening", "opening")?;
            map_state("state_closing", "closing", "closing")?;
        }
        Component::Vacuum => {
            for state in VACUUM_STATES {
                profile.states.push((state.to_owned(), state.to_owned()));
            }
        }
        _ => {}
    }

    let command_topic = topic("command_topic")?;
    let fixed = |payload_key: &'static str, default: Option<&str>| {
        text(config, payload_key).map(|value| value.or_else(|| default.map(str::to_owned)))
    };
    let mut add = |service: &'static str, topic: Option<&String>, payload: Payload, optimistic| {
        if let Some(topic) = topic {
            profile.services.push(ServiceCommand {
                service,
                topic: topic.clone(),
                payload,
                optimistic,
            });
        }
    };
    match component {
        Component::Lock => {
            for (service, key, default, optimistic) in [
                ("lock", "payload_lock", Some("LOCK"), "locked"),
                ("unlock", "payload_unlock", Some("UNLOCK"), "unlocked"),
                // Ouvrir la porte (pas seulement déverrouiller) : seulement si
                // l'appareil le propose.
                ("open", "payload_open", None, "open"),
            ] {
                if let Some(payload) = fixed(key, default)? {
                    add(
                        service,
                        command_topic.as_ref(),
                        Payload::Fixed(payload),
                        Some(optimistic),
                    );
                }
            }
        }
        Component::Valve => {
            for (service, key, default, optimistic) in [
                ("open_valve", "payload_open", Some("OPEN"), Some("open")),
                (
                    "close_valve",
                    "payload_close",
                    Some("CLOSE"),
                    Some("closed"),
                ),
                ("stop_valve", "payload_stop", None, None),
            ] {
                if let Some(payload) = fixed(key, default)? {
                    add(
                        service,
                        command_topic.as_ref(),
                        Payload::Fixed(payload),
                        optimistic,
                    );
                }
            }
        }
        Component::Vacuum => {
            for (service, key, default) in [
                ("start", "payload_start", "start"),
                ("pause", "payload_pause", "pause"),
                ("stop", "payload_stop", "stop"),
                ("return_to_base", "payload_return_to_base", "return_to_base"),
                ("locate", "payload_locate", "locate"),
                ("clean_spot", "payload_clean_spot", "clean_spot"),
            ] {
                if let Some(payload) = fixed(key, Some(default))? {
                    add(
                        service,
                        command_topic.as_ref(),
                        Payload::Fixed(payload),
                        None,
                    );
                }
            }
        }
        Component::Humidifier => {
            let min = number(config, "min_humidity")?.unwrap_or(0.0);
            let max = number(config, "max_humidity")?.unwrap_or(100.0);
            if !(0.0..=100.0).contains(&min) || !(min..=100.0).contains(&max) {
                return Err(DiscoveryError::InvalidField("max_humidity"));
            }
            add(
                "set_humidity",
                topic("target_humidity_command_topic")?.as_ref(),
                Payload::Number {
                    field: "humidity",
                    min,
                    max,
                },
                None,
            );
            profile.fixed.push(("min_humidity", json!(min)));
            profile.fixed.push(("max_humidity", json!(max)));
        }
        Component::WaterHeater => {
            let modes = match config.get("modes") {
                None => WATER_HEATER_MODES.iter().map(|m| (*m).to_owned()).collect(),
                Some(Value::Array(list)) if !list.is_empty() && list.len() <= MAX_OPTIONS => list
                    .iter()
                    .map(|mode| {
                        mode.as_str()
                            .filter(|m| is_short_text(m))
                            .map(str::to_owned)
                            .ok_or(DiscoveryError::InvalidField("modes"))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                Some(_) => return Err(DiscoveryError::InvalidField("modes")),
            };
            for mode in &modes {
                profile.states.push((mode.clone(), mode.clone()));
            }
            let min = number(config, "min_temp")?.unwrap_or(43.3);
            let max = number(config, "max_temp")?.unwrap_or(60.0);
            if !(-50.0..=150.0).contains(&min) || !(min..=150.0).contains(&max) {
                return Err(DiscoveryError::InvalidField("max_temp"));
            }
            add(
                "set_operation_mode",
                topic("mode_command_topic")?.as_ref(),
                Payload::Choice {
                    field: "operation_mode",
                    options: modes.clone(),
                },
                None,
            );
            add(
                "set_temperature",
                topic("temperature_command_topic")?.as_ref(),
                Payload::Number {
                    field: "temperature",
                    min,
                    max,
                },
                None,
            );
            let power = topic("power_command_topic")?;
            add(
                "turn_on",
                power.as_ref(),
                Payload::Fixed(payload_on.to_owned()),
                None,
            );
            add(
                "turn_off",
                power.as_ref(),
                Payload::Fixed(payload_off.to_owned()),
                None,
            );
            profile.fixed.push(("operation_list", json!(modes)));
            profile.fixed.push(("min_temp", json!(min)));
            profile.fixed.push(("max_temp", json!(max)));
        }
        _ => {}
    }
    if component == Component::Lock {
        let supports_open = profile.command("open").is_some();
        profile.fixed.push(("supports_open", json!(supports_open)));
    }

    // Les sujets d'attributs.
    let attribute = |name: &'static str,
                     topic_key: Option<&'static str>,
                     template_key: Option<&'static str>,
                     default_template: Template,
                     numeric: bool|
     -> Result<Option<AttributeTopic>, DiscoveryError> {
        let topic = match topic_key {
            Some(key) => topic(key)?,
            None => state_topic.map(str::to_owned),
        };
        let template = match template_key {
            Some(key) => template(config, key)?,
            None => None,
        };
        Ok(topic.map(|topic| AttributeTopic {
            name,
            topic,
            template: template.unwrap_or(default_template),
            numeric,
        }))
    };
    let json_key = |key: &str| Template::Json(vec![Segment::Key(key.to_owned())]);
    let wanted = match component {
        Component::Vacuum => vec![
            attribute("battery_level", None, None, json_key("battery_level"), true)?,
            attribute("fan_speed", None, None, json_key("fan_speed"), false)?,
        ],
        Component::Humidifier => vec![
            attribute(
                "humidity",
                Some("target_humidity_state_topic"),
                Some("target_humidity_state_template"),
                Template::Raw,
                true,
            )?,
            attribute(
                "current_humidity",
                Some("current_humidity_topic"),
                Some("current_humidity_template"),
                Template::Raw,
                true,
            )?,
        ],
        Component::WaterHeater => vec![
            attribute(
                "temperature",
                Some("temperature_state_topic"),
                Some("temperature_state_template"),
                Template::Raw,
                true,
            )?,
            attribute(
                "current_temperature",
                Some("current_temperature_topic"),
                Some("current_temperature_template"),
                Template::Raw,
                true,
            )?,
        ],
        _ => Vec::new(),
    };
    profile.attributes = wanted.into_iter().flatten().collect();
    Ok(profile)
}

/// Un modèle facultatif, refusé s'il n'est pas compris.
fn template(
    config: &Map<String, Value>,
    key: &'static str,
) -> Result<Option<Template>, DiscoveryError> {
    match text(config, key)? {
        None => Ok(None),
        Some(source) => Template::parse(&source)
            .map(Some)
            .ok_or(DiscoveryError::UnsupportedTemplate(source)),
    }
}

/// Un nombre facultatif, fini.
fn number(config: &Map<String, Value>, key: &'static str) -> Result<Option<f64>, DiscoveryError> {
    match config.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|n| n.is_finite())
            .map(Some)
            .ok_or(DiscoveryError::InvalidField(key)),
    }
}

/// Un texte court et sans caractères de contrôle (un nom de mode…).
fn is_short_text(value: &str) -> bool {
    !value.is_empty() && value.chars().count() <= 64 && !value.chars().any(char::is_control)
}

/// Remplace les abréviations par les noms complets.
fn expand(raw: Map<String, Value>) -> Map<String, Value> {
    raw.into_iter()
        .map(|(key, value)| {
            let key = ABBREVIATIONS
                .iter()
                .find(|(short, _)| *short == key)
                .map_or(key, |(_, long)| (*long).to_owned());
            (key, value)
        })
        .collect()
}

/// Lit un champ texte facultatif, limité en longueur et sans caractères de contrôle.
fn text(config: &Map<String, Value>, key: &'static str) -> Result<Option<String>, DiscoveryError> {
    match config.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value))
            if value.chars().count() <= MAX_TEXT_LEN && !value.chars().any(char::is_control) =>
        {
            Ok(Some(value.clone()))
        }
        // Certains appareils envoient `payload_on: 1` ou `true`.
        Some(value @ (Value::Number(_) | Value::Bool(_))) => Ok(Some(value.to_string())),
        Some(_) => Err(DiscoveryError::InvalidField(key)),
    }
}

/// Vérifie un sujet d'état ou de commande.
fn check_topic(topic: &str, prefix: &str, key: &'static str) -> Result<String, DiscoveryError> {
    let valid = !topic.is_empty()
        && topic.len() <= MAX_TOPIC_LEN
        // Les jokers `+` et `#` n'ont de sens que pour s'abonner.
        && !topic.contains(['+', '#', '\0'])
        // Sécurité : un appareil ne doit pas pouvoir nous faire publier des
        // descriptions de découverte (et donc créer de fausses entités).
        && !topic.starts_with(&format!("{prefix}/"));
    if valid {
        Ok(topic.to_owned())
    } else {
        Err(DiscoveryError::InvalidField(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PREFIX: &str = "homeassistant";

    fn config(component: &str, payload: &str) -> EntityConfig {
        match parse(PREFIX, component, "objet", payload.as_bytes()) {
            Ok(Discovery::Config(config)) => *config,
            other => panic!("config attendue, reçu {other:?}"),
        }
    }

    #[test]
    fn topics() {
        assert_eq!(
            parse_topic(PREFIX, "homeassistant/sensor/0x00158d/temperature/config"),
            Some(("sensor", "temperature"))
        );
        assert_eq!(
            parse_topic(PREFIX, "homeassistant/switch/prise/config"),
            Some(("switch", "prise"))
        );
        assert_eq!(
            parse_topic(PREFIX, "homeassistant/switch/prise/state"),
            None
        );
        assert_eq!(parse_topic(PREFIX, "autre/switch/prise/config"), None);
        assert_eq!(parse_topic(PREFIX, "homeassistant/a/b/c/d/config"), None);
    }

    #[test]
    fn zigbee2mqtt_sensor() {
        let c = config(
            "sensor",
            r#"{"name": "Temperature", "stat_t": "zigbee2mqtt/capteur_salon",
                "val_tpl": "{{ value_json.temperature }}", "unit_of_meas": "°C",
                "dev_cla": "temperature", "obj_id": "capteur_salon_temperature",
                "dev": {"name": "Capteur salon"}}"#,
        );
        assert_eq!(c.component, Component::Sensor);
        assert_eq!(c.state_topic.as_deref(), Some("zigbee2mqtt/capteur_salon"));
        assert_eq!(c.object_id, "capteur_salon_temperature");
        assert_eq!(
            c.friendly_name.as_deref(),
            Some("Capteur salon Temperature")
        );
        assert_eq!(c.unit_of_measurement.as_deref(), Some("°C"));
    }

    #[test]
    fn tasmota_switch_with_base_topic() {
        let c = config(
            "switch",
            r#"{"name": "Prise café", "~": "tasmota/prise/", "stat_t": "~stat/POWER",
                "cmd_t": "~cmnd/POWER", "pl_on": "ON", "pl_off": "OFF"}"#,
        );
        assert_eq!(c.state_topic.as_deref(), Some("tasmota/prise/stat/POWER"));
        assert_eq!(c.command_topic.as_deref(), Some("tasmota/prise/cmnd/POWER"));
        assert_eq!(c.object_id, "Prise café");
    }

    #[test]
    fn json_light() {
        let c = config(
            "light",
            r#"{"name": null, "schema": "json", "stat_t": "z2m/lampe", "cmd_t": "z2m/lampe/set",
                "brightness": true, "dev": {"name": "Lampe"}}"#,
        );
        assert!(c.json_schema);
        assert_eq!(c.friendly_name.as_deref(), Some("Lampe"));
        assert_eq!(c.brightness_scale, 255);
    }

    #[test]
    fn empty_payload_removes() {
        assert_eq!(parse(PREFIX, "sensor", "x", b""), Ok(Discovery::Remove));
    }

    #[test]
    fn dangerous_or_invalid_configs_are_refused() {
        let cases = [
            ("climate", r#"{"stat_t": "a"}"#),
            ("sensor", "pas du json"),
            ("sensor", "[1, 2]"),
            ("sensor", r#"{"name": "rien à écouter"}"#),
            ("sensor", r#"{"stat_t": "maison/#"}"#),
            ("sensor", r#"{"stat_t": "maison/+/temp"}"#),
            ("switch", r#"{"cmd_t": "homeassistant/switch/faux/config"}"#),
            (
                "sensor",
                r#"{"stat_t": "a", "val_tpl": "{{ value_json.a | float }}"}"#,
            ),
            ("sensor", r#"{"stat_t": {"objet": "au lieu d'un texte"}}"#),
            ("sensor", r#"{"stat_t": "a", "name": "ligne\nnouvelle"}"#),
            ("light", r#"{"stat_t": "a", "schema": "template"}"#),
        ];
        for (component, payload) in cases {
            assert!(
                parse(PREFIX, component, "x", payload.as_bytes()).is_err(),
                "{component} {payload}"
            );
        }
        let huge = format!(
            r#"{{"stat_t": "a", "name": "{}"}}"#,
            "x".repeat(MAX_CONFIG_SIZE)
        );
        assert!(matches!(
            parse(PREFIX, "sensor", "x", huge.as_bytes()),
            Err(DiscoveryError::TooLarge(_))
        ));
    }
}
