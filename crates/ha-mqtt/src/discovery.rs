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

use serde_json::{Map, Value};

use crate::template::Template;

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
}

impl Component {
    fn from_str(value: &str) -> Option<Self> {
        match value {
            "sensor" => Some(Self::Sensor),
            "binary_sensor" => Some(Self::BinarySensor),
            "switch" => Some(Self::Switch),
            "light" => Some(Self::Light),
            _ => None,
        }
    }

    /// Le domaine de l'identifiant d'entité : `sensor`, `light`…
    pub fn domain(self) -> &'static str {
        match self {
            Self::Sensor => "sensor",
            Self::BinarySensor => "binary_sensor",
            Self::Switch => "switch",
            Self::Light => "light",
        }
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
];

/// Découpe un sujet de découverte. Renvoie le type et l'identifiant d'objet,
/// ou `None` si ce n'est pas un sujet de découverte.
///
/// Formats : `<préfixe>/<type>/<objet>/config` ou `<préfixe>/<type>/<nœud>/<objet>/config`.
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
    let state_topic = topic("state_topic")?;
    let command_topic = topic("command_topic")?;
    if state_topic.is_none() && command_topic.is_none() {
        return Err(DiscoveryError::NoTopic);
    }

    // Pour une lumière, le modèle de l'état s'appelle `state_value_template`.
    let template_key = match component {
        Component::Light => "state_value_template",
        _ => "value_template",
    };
    let value_template = match text(&config, template_key)? {
        None => Template::Raw,
        Some(source) => {
            Template::parse(&source).ok_or(DiscoveryError::UnsupportedTemplate(source))?
        }
    };

    let payload_on = text(&config, "payload_on")?.unwrap_or_else(|| "ON".to_owned());
    let payload_off = text(&config, "payload_off")?.unwrap_or_else(|| "OFF".to_owned());
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
    })))
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

/// Transforme un nom en morceau d'identifiant : `Température Salon` → `temperature_salon`.
pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars().flat_map(fold_accent) {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('_') && !slug.is_empty() {
            slug.push('_');
        }
    }
    let slug = slug.trim_end_matches('_');
    slug.chars()
        .take(100)
        .collect::<String>()
        .trim_end_matches('_')
        .to_owned()
}

/// Retire les accents les plus courants (é → e, ç → c…).
fn fold_accent(c: char) -> Option<char> {
    Some(match c {
        'à' | 'â' | 'ä' | 'á' | 'ã' | 'À' | 'Â' | 'Ä' | 'Á' => 'a',
        'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => 'e',
        'î' | 'ï' | 'í' | 'Î' | 'Ï' => 'i',
        'ô' | 'ö' | 'ó' | 'õ' | 'Ô' | 'Ö' => 'o',
        'ù' | 'û' | 'ü' | 'ú' | 'Ù' | 'Û' | 'Ü' => 'u',
        'ç' | 'Ç' => 'c',
        'ñ' | 'Ñ' => 'n',
        'ÿ' => 'y',
        'œ' | 'Œ' => 'o',
        other => other,
    })
}

#[cfg(test)]
#[allow(clippy::panic)]
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

    #[test]
    fn slugs() {
        assert_eq!(slugify("Température Salon"), "temperature_salon");
        assert_eq!(slugify("  Prise -- café!! "), "prise_cafe");
        assert_eq!(slugify("0x00158d0001"), "0x00158d0001");
        assert_eq!(slugify("___"), "");
        assert_eq!(slugify("日本"), "");
    }
}
