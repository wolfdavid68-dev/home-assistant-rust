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

use crate::profile::{
    AttrKind, AttributeTopic, FreeState, Optimistic, Payload, Profile, ServiceCommand,
};
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
    Cover,
    Fan,
    Climate,
    Button,
    Number,
    Select,
    Text,
    Event,
    AlarmControlPanel,
    /// Une caméra (étape 82) : la photo d'une sonnette, reçue telle quelle
    /// (JPEG ou PNG, ou en base64) sur son sujet.
    Camera,
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
            "cover" => Some(Self::Cover),
            "fan" => Some(Self::Fan),
            "climate" => Some(Self::Climate),
            "button" => Some(Self::Button),
            "number" => Some(Self::Number),
            "select" => Some(Self::Select),
            "text" => Some(Self::Text),
            "event" => Some(Self::Event),
            "alarm_control_panel" => Some(Self::AlarmControlPanel),
            "camera" => Some(Self::Camera),
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
            Self::Cover => "cover",
            Self::Fan => "fan",
            Self::Climate => "climate",
            Self::Button => "button",
            Self::Number => "number",
            Self::Select => "select",
            Self::Text => "text",
            Self::Event => "event",
            Self::AlarmControlPanel => "alarm_control_panel",
            Self::Camera => "camera",
        }
    }

    /// Son état est « on » / « off » (et `turn_on` / `turn_off` le commandent).
    #[must_use]
    pub fn is_on_off(self) -> bool {
        matches!(
            self,
            Self::BinarySensor
                | Self::Switch
                | Self::Light
                | Self::Siren
                | Self::Humidifier
                | Self::Fan
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
    ("stat_clsd", "state_closed"),
    ("stat_closing", "state_closing"),
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
    // Volet.
    ("pos_t", "position_topic"),
    ("pos_tpl", "position_template"),
    ("set_pos_t", "set_position_topic"),
    ("pos_open", "position_open"),
    ("pos_clsd", "position_closed"),
    ("stat_stopped", "state_stopped"),
    // Ventilateur.
    ("pct_cmd_t", "percentage_command_topic"),
    ("pct_stat_t", "percentage_state_topic"),
    ("pct_val_tpl", "percentage_value_template"),
    ("spd_rng_min", "speed_range_min"),
    ("spd_rng_max", "speed_range_max"),
    ("osc_cmd_t", "oscillation_command_topic"),
    ("osc_stat_t", "oscillation_state_topic"),
    ("osc_val_tpl", "oscillation_value_template"),
    ("pl_osc_on", "payload_oscillation_on"),
    ("pl_osc_off", "payload_oscillation_off"),
    ("pr_mode_cmd_t", "preset_mode_command_topic"),
    ("pr_mode_stat_t", "preset_mode_state_topic"),
    ("pr_mode_val_tpl", "preset_mode_value_template"),
    ("pr_modes", "preset_modes"),
    // Thermostat.
    ("act_t", "action_topic"),
    ("act_tpl", "action_template"),
    // Bouton, nombre, liste, texte, événement, alarme.
    ("pl_prs", "payload_press"),
    ("ops", "options"),
    ("ptrn", "pattern"),
    ("evt_typ", "event_types"),
    ("sup_feat", "supported_features"),
    ("cmd_tpl", "command_template"),
    ("pl_arm_away", "payload_arm_away"),
    ("pl_arm_home", "payload_arm_home"),
    ("pl_arm_nite", "payload_arm_night"),
    ("pl_arm_vacation", "payload_arm_vacation"),
    ("pl_arm_custom_b", "payload_arm_custom_bypass"),
    ("pl_disarm", "payload_disarm"),
    ("pl_trig", "payload_trigger"),
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
/// Les modes d'un thermostat quand l'appareil n'en annonce pas.
const CLIMATE_MODES: [&str; 6] = ["auto", "off", "cool", "heat", "dry", "fan_only"];
/// Les états d'une alarme (ceux de Home Assistant, reçus tels quels).
const ALARM_STATES: [&str; 10] = [
    "disarmed",
    "armed_home",
    "armed_away",
    "armed_night",
    "armed_vacation",
    "armed_custom_bypass",
    "pending",
    "triggered",
    "arming",
    "disarming",
];
/// Ce qu'une alarme sait faire, quand elle ne le dit pas : tout.
const ALARM_FEATURES: [&str; 6] = [
    "arm_home",
    "arm_away",
    "arm_night",
    "arm_vacation",
    "arm_custom_bypass",
    "trigger",
];
/// Les états d'un aspirateur (ceux de Home Assistant).
const VACUUM_STATES: [&str; 6] = ["cleaning", "docked", "paused", "idle", "returning", "error"];
/// Nombre maximal de choix annoncés par un appareil (modes, options…).
const MAX_OPTIONS: usize = 50;

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
    // Un thermostat non plus : son état est son mode (`heat`, `off`…).
    // Une caméra n'a qu'un sujet, celui de l'image (`topic`).
    let (state_key, command_key) = match component {
        Component::WaterHeater | Component::Climate => ("mode_state_topic", "mode_command_topic"),
        Component::Camera => ("topic", "command_topic"),
        _ => ("state_topic", "command_topic"),
    };
    let state_topic = topic(state_key)?;
    let command_topic = topic(command_key)?;

    // Pour une lumière, le modèle de l'état s'appelle `state_value_template`.
    let template_key = match component {
        Component::Light | Component::Fan => "state_value_template",
        Component::WaterHeater | Component::Climate => "mode_state_template",
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
    // ring-mqtt (sonnette Ring, étape 82) nomme ses entités en anglais : on
    // les traduit (« Ding » → « Sonnette »), seulement pour ses appareils.
    let ring = config
        .get("device")
        .and_then(|device| device.get("mf").or_else(|| device.get("manufacturer")))
        .and_then(Value::as_str)
        == Some("Ring");
    let entity_name = text(&config, "name")?.map(|name| {
        if ring {
            ring_name(&name).map_or(name, str::to_owned)
        } else {
            name
        }
    });
    let friendly_name = match (device_name, entity_name) {
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

/// Le nom français d'une entité de ring-mqtt.
fn ring_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "Ding" => "Sonnette",
        "Motion" => "Mouvement",
        "Snapshot" => "Photo",
        "Snapshot Mode" => "Mode photo",
        "Snapshot Interval" => "Intervalle des photos",
        "Take Snapshot" => "Prendre une photo",
        "Motion Detection" => "Détection de mouvement",
        "Motion Duration" => "Durée du mouvement",
        "Motion Warning" => "Alerte mouvement",
        "Ding Duration" => "Durée de la sonnerie",
        "Battery" => "Batterie",
        "Wireless" => "Wi-Fi",
        "Info" => "Infos",
        "Light" => "Lumière",
        "Siren" => "Sirène",
        "Live Stream" => "Vidéo en direct",
        "Event Stream" => "Vidéo d'un événement",
        "Event Select" => "Choix de l'événement",
        "Volume" => "Volume",
        _ => return None,
    })
}

/// Ce que sait faire chaque type d'appareil, avec les valeurs par défaut de
/// Home Assistant (`homeassistant/components/mqtt/*.py`).
#[allow(
    clippy::too_many_lines,
    reason = "une branche par type d'appareil : les séparer n'apporterait rien"
)]
fn profile(
    component: Component,
    config: &Map<String, Value>,
    topic: &dyn Fn(&'static str) -> Result<Option<String>, DiscoveryError>,
    state_topic: Option<&str>,
    payload_on: &str,
    payload_off: &str,
) -> Result<Profile, DiscoveryError> {
    let mut profile = Profile::default();
    let command_topic = topic(match component {
        Component::Climate | Component::WaterHeater => "mode_command_topic",
        _ => "command_topic",
    })?;
    // Un texte de la description, ou sa valeur par défaut.
    let or = |key: &'static str, default: &str| -> Result<String, DiscoveryError> {
        Ok(text(config, key)?.unwrap_or_else(|| default.to_owned()))
    };
    // Un service qui publie toujours le même texte sur le sujet de commande.
    let fixed = |service: &'static str, payload: String, optimistic: Optimistic| {
        command_topic.as_ref().map(|topic| ServiceCommand {
            service,
            topic: topic.clone(),
            payload: Payload::Fixed(payload),
            optimistic,
        })
    };
    // Un service sur son propre sujet (`set_position_topic`…), s'il est annoncé.
    let on_topic = |service: &'static str,
                    key: &'static str,
                    payload: Payload,
                    optimistic: Optimistic|
     -> Result<Option<ServiceCommand>, DiscoveryError> {
        Ok(topic(key)?.map(|topic| ServiceCommand {
            service,
            topic,
            payload,
            optimistic,
        }))
    };
    // Un sujet d'attribut, s'il est annoncé (sinon : le sujet d'état).
    let attribute = |name: &'static str,
                     topic_key: Option<&'static str>,
                     template_key: Option<&'static str>,
                     default_template: Template,
                     kind: AttrKind|
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
            kind,
        }))
    };
    let json_key = |key: &str| Template::Json(vec![Segment::Key(key.to_owned())]);
    let mut services: Vec<Option<ServiceCommand>> = Vec::new();
    let mut attributes: Vec<Option<AttributeTopic>> = Vec::new();
    let mut map_state = |received: String, state: &str| {
        profile.states.push((received, state.to_owned()));
    };

    match component {
        Component::Lock => {
            for (key, default, state) in [
                ("state_locked", "LOCKED", "locked"),
                ("state_unlocked", "UNLOCKED", "unlocked"),
                ("state_locking", "LOCKING", "locking"),
                ("state_unlocking", "UNLOCKING", "unlocking"),
                ("state_jammed", "JAMMED", "jammed"),
                ("state_open", "OPEN", "open"),
                ("state_opening", "OPENING", "opening"),
            ] {
                map_state(or(key, default)?, state);
            }
            services.push(fixed(
                "lock",
                or("payload_lock", "LOCK")?,
                Optimistic::State("locked"),
            ));
            services.push(fixed(
                "unlock",
                or("payload_unlock", "UNLOCK")?,
                Optimistic::State("unlocked"),
            ));
            // Ouvrir la porte (pas seulement déverrouiller) : seulement si
            // l'appareil le propose.
            if let Some(open) = text(config, "payload_open")? {
                services.push(fixed("open", open, Optimistic::State("open")));
            }
        }
        Component::Valve | Component::Cover => {
            let (open, closed) = if component == Component::Valve {
                ("open_valve", "close_valve")
            } else {
                ("open_cover", "close_cover")
            };
            for (key, state) in [
                ("state_open", "open"),
                ("state_closed", "closed"),
                ("state_opening", "opening"),
                ("state_closing", "closing"),
            ] {
                map_state(or(key, state)?, state);
            }
            // Un volet arrêté à mi-course est « ouvert », comme dans Home Assistant.
            if component == Component::Cover {
                map_state(or("state_stopped", "stopped")?, "open");
            }
            services.push(fixed(
                open,
                or("payload_open", "OPEN")?,
                Optimistic::State("open"),
            ));
            services.push(fixed(
                closed,
                or("payload_close", "CLOSE")?,
                Optimistic::State("closed"),
            ));
            let stop_default = (component == Component::Cover).then_some("STOP");
            if let Some(stop) = text(config, "payload_stop")?.or(stop_default.map(str::to_owned)) {
                let service = if component == Component::Valve {
                    "stop_valve"
                } else {
                    "stop_cover"
                };
                services.push(fixed(service, stop, Optimistic::Keep));
            }
            if component == Component::Cover {
                // La position, de `position_closed` à `position_open` côté appareil.
                let closed = number(config, "position_closed")?.unwrap_or(0.0);
                let open = number(config, "position_open")?.unwrap_or(100.0);
                if (closed - open).abs() < 1.0
                    || !(-1e6..=1e6).contains(&closed)
                    || !(-1e6..=1e6).contains(&open)
                {
                    return Err(DiscoveryError::InvalidField("position_open"));
                }
                services.push(on_topic(
                    "set_cover_position",
                    "set_position_topic",
                    Payload::Number {
                        field: "position",
                        min: 0.0,
                        max: 100.0,
                        scale: Some((closed, open)),
                    },
                    Optimistic::Keep,
                )?);
                attributes.push(attribute(
                    "current_position",
                    Some("position_topic"),
                    Some("position_template"),
                    Template::Raw,
                    AttrKind::Percent {
                        min: closed,
                        max: open,
                    },
                )?);
            }
        }
        Component::Vacuum => {
            for state in VACUUM_STATES {
                map_state(state.to_owned(), state);
            }
            for (service, key, default) in [
                ("start", "payload_start", "start"),
                ("pause", "payload_pause", "pause"),
                ("stop", "payload_stop", "stop"),
                ("return_to_base", "payload_return_to_base", "return_to_base"),
                ("locate", "payload_locate", "locate"),
                ("clean_spot", "payload_clean_spot", "clean_spot"),
            ] {
                services.push(fixed(service, or(key, default)?, Optimistic::Keep));
            }
            attributes.push(attribute(
                "battery_level",
                None,
                None,
                json_key("battery_level"),
                AttrKind::Number,
            )?);
            attributes.push(attribute(
                "fan_speed",
                None,
                None,
                json_key("fan_speed"),
                AttrKind::Text,
            )?);
        }
        Component::Humidifier => {
            let (min, max) = bounds(
                config,
                "min_humidity",
                "max_humidity",
                (0.0, 100.0),
                (0.0, 100.0),
            )?;
            services.push(on_topic(
                "set_humidity",
                "target_humidity_command_topic",
                Payload::Number {
                    field: "humidity",
                    min,
                    max,
                    scale: None,
                },
                Optimistic::Keep,
            )?);
            profile.fixed.push(("min_humidity", json!(min)));
            profile.fixed.push(("max_humidity", json!(max)));
            attributes.push(attribute(
                "humidity",
                Some("target_humidity_state_topic"),
                Some("target_humidity_state_template"),
                Template::Raw,
                AttrKind::Number,
            )?);
            attributes.push(attribute(
                "current_humidity",
                Some("current_humidity_topic"),
                Some("current_humidity_template"),
                Template::Raw,
                AttrKind::Number,
            )?);
        }
        Component::WaterHeater | Component::Climate => {
            let heater = component == Component::WaterHeater;
            let defaults: &[&str] = if heater {
                &WATER_HEATER_MODES
            } else {
                &CLIMATE_MODES
            };
            let modes = options(config, "modes", defaults)?;
            for mode in &modes {
                map_state(mode.clone(), mode);
            }
            let (min, max) = if heater {
                bounds(config, "min_temp", "max_temp", (43.3, 60.0), (-50.0, 150.0))?
            } else {
                bounds(config, "min_temp", "max_temp", (7.0, 35.0), (-50.0, 150.0))?
            };
            let (service, field) = if heater {
                ("set_operation_mode", "operation_mode")
            } else {
                ("set_hvac_mode", "hvac_mode")
            };
            services.push(on_topic(
                service,
                "mode_command_topic",
                Payload::Choice {
                    field,
                    options: modes.clone(),
                },
                Optimistic::Sent,
            )?);
            services.push(on_topic(
                "set_temperature",
                "temperature_command_topic",
                Payload::Number {
                    field: "temperature",
                    min,
                    max,
                    scale: None,
                },
                Optimistic::Keep,
            )?);
            services.push(on_topic(
                "turn_on",
                "power_command_topic",
                Payload::Fixed(payload_on.to_owned()),
                Optimistic::Keep,
            )?);
            services.push(on_topic(
                "turn_off",
                "power_command_topic",
                Payload::Fixed(payload_off.to_owned()),
                Optimistic::Keep,
            )?);
            profile.fixed.push((
                if heater {
                    "operation_list"
                } else {
                    "hvac_modes"
                },
                json!(modes),
            ));
            profile.fixed.push(("min_temp", json!(min)));
            profile.fixed.push(("max_temp", json!(max)));
            attributes.push(attribute(
                "temperature",
                Some("temperature_state_topic"),
                Some("temperature_state_template"),
                Template::Raw,
                AttrKind::Number,
            )?);
            attributes.push(attribute(
                "current_temperature",
                Some("current_temperature_topic"),
                Some("current_temperature_template"),
                Template::Raw,
                AttrKind::Number,
            )?);
            if !heater {
                let step = number(config, "temp_step")?.unwrap_or(1.0);
                if !(0.01..=10.0).contains(&step) {
                    return Err(DiscoveryError::InvalidField("temp_step"));
                }
                profile.fixed.push(("target_temp_step", json!(step)));
                attributes.push(attribute(
                    "hvac_action",
                    Some("action_topic"),
                    Some("action_template"),
                    Template::Raw,
                    AttrKind::Text,
                )?);
                attributes.push(attribute(
                    "current_humidity",
                    Some("current_humidity_topic"),
                    Some("current_humidity_template"),
                    Template::Raw,
                    AttrKind::Number,
                )?);
                preset_modes(
                    config,
                    &on_topic,
                    &attribute,
                    &mut services,
                    &mut attributes,
                    &mut profile.fixed,
                )?;
            }
        }
        Component::Fan => {
            let low = number(config, "speed_range_min")?.unwrap_or(1.0);
            let high = number(config, "speed_range_max")?.unwrap_or(100.0);
            if !(0.0..=1000.0).contains(&low) || !(low + 1.0..=1000.0).contains(&high) {
                return Err(DiscoveryError::InvalidField("speed_range_max"));
            }
            services.push(on_topic(
                "set_percentage",
                "percentage_command_topic",
                Payload::Number {
                    field: "percentage",
                    min: 0.0,
                    max: 100.0,
                    scale: Some((low - 1.0, high)),
                },
                Optimistic::Keep,
            )?);
            attributes.push(attribute(
                "percentage",
                Some("percentage_state_topic"),
                Some("percentage_value_template"),
                Template::Raw,
                AttrKind::Percent {
                    min: low - 1.0,
                    max: high,
                },
            )?);
            let on = or("payload_oscillation_on", "oscillate_on")?;
            let off = or("payload_oscillation_off", "oscillate_off")?;
            services.push(on_topic(
                "oscillate",
                "oscillation_command_topic",
                Payload::Flag {
                    field: "oscillating",
                    on: on.clone(),
                    off: off.clone(),
                },
                Optimistic::Keep,
            )?);
            attributes.push(attribute(
                "oscillating",
                Some("oscillation_state_topic"),
                Some("oscillation_value_template"),
                Template::Raw,
                AttrKind::Flag { on, off },
            )?);
            preset_modes(
                config,
                &on_topic,
                &attribute,
                &mut services,
                &mut attributes,
                &mut profile.fixed,
            )?;
        }
        Component::Button => {
            services.push(fixed(
                "press",
                or("payload_press", "PRESS")?,
                Optimistic::Now,
            ));
        }
        Component::Number => {
            let min = number(config, "min")?.unwrap_or(1.0);
            let max = number(config, "max")?.unwrap_or(100.0);
            let step = number(config, "step")?.unwrap_or(1.0);
            if min >= max
                || !(-1e9..=1e9).contains(&min)
                || !(-1e9..=1e9).contains(&max)
                || !(0.001..=1e6).contains(&step)
            {
                return Err(DiscoveryError::InvalidField("max"));
            }
            profile.free = FreeState::Number;
            services.push(fixed_number(command_topic.as_ref(), min, max));
            profile.fixed.push(("min", json!(min)));
            profile.fixed.push(("max", json!(max)));
            profile.fixed.push(("step", json!(step)));
            let mode = or("mode", "auto")?;
            if !["auto", "box", "slider"].contains(&mode.as_str()) {
                return Err(DiscoveryError::InvalidField("mode"));
            }
            profile.fixed.push(("mode", json!(mode)));
        }
        Component::Select => {
            let choices = options(config, "options", &[])?;
            for choice in &choices {
                map_state(choice.clone(), choice);
            }
            services.push(command_topic.as_ref().map(|topic| ServiceCommand {
                service: "select_option",
                topic: topic.clone(),
                payload: Payload::Choice {
                    field: "option",
                    options: choices.clone(),
                },
                optimistic: Optimistic::Sent,
            }));
            profile.fixed.push(("options", json!(choices)));
        }
        Component::Text => {
            let min = usize_field(config, "min")?.unwrap_or(0);
            let max = usize_field(config, "max")?.unwrap_or(255).min(255);
            if min > max {
                return Err(DiscoveryError::InvalidField("min"));
            }
            let mode = or("mode", "text")?;
            if !["text", "password"].contains(&mode.as_str()) {
                return Err(DiscoveryError::InvalidField("mode"));
            }
            profile.free = FreeState::Text { max };
            services.push(command_topic.as_ref().map(|topic| ServiceCommand {
                service: "set_value",
                topic: topic.clone(),
                payload: Payload::Text {
                    field: "value",
                    min,
                    max,
                },
                optimistic: Optimistic::Sent,
            }));
            profile.fixed.push(("min", json!(min)));
            profile.fixed.push(("max", json!(max)));
            profile.fixed.push(("mode", json!(mode)));
        }
        Component::Camera => {
            profile.image_base64 = match text(config, "image_encoding")?.as_deref() {
                None => false,
                Some("b64") => true,
                Some(_) => return Err(DiscoveryError::InvalidField("image_encoding")),
            };
        }
        Component::Event => {
            profile.event_types = options(config, "event_types", &[])?;
            profile
                .fixed
                .push(("event_types", json!(profile.event_types)));
        }
        Component::AlarmControlPanel => {
            for state in ALARM_STATES {
                map_state(state.to_owned(), state);
            }
            // Le modèle de commande : seulement `{{ action }}` (le défaut). Un
            // vrai modèle Jinja pourrait faire n'importe quoi ; il est refusé.
            if let Some(source) = text(config, "command_template")? {
                let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
                if compact != "{{action}}" {
                    return Err(DiscoveryError::UnsupportedTemplate(source));
                }
            }
            let supported = match config.get("supported_features") {
                None => ALARM_FEATURES.iter().map(|f| (*f).to_owned()).collect(),
                Some(_) => options(config, "supported_features", &[])?,
            };
            for (feature, service, key, default) in [
                ("arm_home", "alarm_arm_home", "payload_arm_home", "ARM_HOME"),
                ("arm_away", "alarm_arm_away", "payload_arm_away", "ARM_AWAY"),
                (
                    "arm_night",
                    "alarm_arm_night",
                    "payload_arm_night",
                    "ARM_NIGHT",
                ),
                (
                    "arm_vacation",
                    "alarm_arm_vacation",
                    "payload_arm_vacation",
                    "ARM_VACATION",
                ),
                (
                    "arm_custom_bypass",
                    "alarm_arm_custom_bypass",
                    "payload_arm_custom_bypass",
                    "ARM_CUSTOM_BYPASS",
                ),
                ("trigger", "alarm_trigger", "payload_trigger", "TRIGGER"),
            ] {
                if supported.iter().any(|f| f == feature) {
                    services.push(fixed(service, or(key, default)?, Optimistic::Keep));
                }
            }
            services.push(fixed(
                "alarm_disarm",
                or("payload_disarm", "DISARM")?,
                Optimistic::Keep,
            ));
            profile.fixed.push(("supported_features", json!(supported)));
        }
        Component::Sensor
        | Component::BinarySensor
        | Component::Switch
        | Component::Light
        | Component::Siren => {}
    }
    if component == Component::Lock {
        let supports_open = services.iter().flatten().any(|c| c.service == "open");
        profile.fixed.push(("supports_open", json!(supports_open)));
    }
    profile.services = services.into_iter().flatten().collect();
    profile.attributes = attributes.into_iter().flatten().collect();
    Ok(profile)
}

/// Fabrique un service sur son propre sujet (voir `profile`).
type CommandOn<'a> = dyn Fn(
        &'static str,
        &'static str,
        Payload,
        Optimistic,
    ) -> Result<Option<ServiceCommand>, DiscoveryError>
    + 'a;
/// Fabrique un sujet d'attribut (voir `profile`).
type AttributeOn<'a> = dyn Fn(
        &'static str,
        Option<&'static str>,
        Option<&'static str>,
        Template,
        AttrKind,
    ) -> Result<Option<AttributeTopic>, DiscoveryError>
    + 'a;

/// `set_value` d'une entité `number`, entre ses bornes.
fn fixed_number(command_topic: Option<&String>, min: f64, max: f64) -> Option<ServiceCommand> {
    command_topic.map(|topic| ServiceCommand {
        service: "set_value",
        topic: topic.clone(),
        payload: Payload::Number {
            field: "value",
            min,
            max,
            scale: None,
        },
        optimistic: Optimistic::Sent,
    })
}

/// Les préréglages d'un ventilateur ou d'un thermostat (« eco », « boost »).
fn preset_modes(
    config: &Map<String, Value>,
    on_topic: &CommandOn<'_>,
    attribute: &AttributeOn<'_>,
    services: &mut Vec<Option<ServiceCommand>>,
    attributes: &mut Vec<Option<AttributeTopic>>,
    fixed: &mut Vec<(&'static str, Value)>,
) -> Result<(), DiscoveryError> {
    if config.get("preset_modes").is_none() {
        return Ok(());
    }
    let presets = options(config, "preset_modes", &[])?;
    services.push(on_topic(
        "set_preset_mode",
        "preset_mode_command_topic",
        Payload::Choice {
            field: "preset_mode",
            options: presets.clone(),
        },
        Optimistic::Keep,
    )?);
    attributes.push(attribute(
        "preset_mode",
        Some("preset_mode_state_topic"),
        Some("preset_mode_value_template"),
        Template::Raw,
        AttrKind::Text,
    )?);
    fixed.push(("preset_modes", json!(presets)));
    Ok(())
}

/// Une liste de choix annoncée par l'appareil (modes, options…) : 1 à
/// `MAX_OPTIONS` textes courts. `defaults` si elle est absente (et qu'il y
/// en a).
fn options(
    config: &Map<String, Value>,
    key: &'static str,
    defaults: &[&str],
) -> Result<Vec<String>, DiscoveryError> {
    match config.get(key) {
        None if !defaults.is_empty() => Ok(defaults.iter().map(|m| (*m).to_owned()).collect()),
        Some(Value::Array(list)) if !list.is_empty() && list.len() <= MAX_OPTIONS => list
            .iter()
            .map(|item| {
                item.as_str()
                    .filter(|m| is_short_text(m))
                    .map(str::to_owned)
                    .ok_or(DiscoveryError::InvalidField(key))
            })
            .collect(),
        _ => Err(DiscoveryError::InvalidField(key)),
    }
}

/// Deux bornes (`min_temp`, `max_temp`…) : par défaut `defaults`, et dans
/// `limits`, avec min < max.
fn bounds(
    config: &Map<String, Value>,
    min_key: &'static str,
    max_key: &'static str,
    defaults: (f64, f64),
    limits: (f64, f64),
) -> Result<(f64, f64), DiscoveryError> {
    let min = number(config, min_key)?.unwrap_or(defaults.0);
    let max = number(config, max_key)?.unwrap_or(defaults.1);
    let inside = |v: f64| (limits.0..=limits.1).contains(&v);
    if inside(min) && inside(max) && min < max {
        Ok((min, max))
    } else {
        Err(DiscoveryError::InvalidField(max_key))
    }
}

/// Un entier positif facultatif.
fn usize_field(
    config: &Map<String, Value>,
    key: &'static str,
) -> Result<Option<usize>, DiscoveryError> {
    match config.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .map(Some)
            .ok_or(DiscoveryError::InvalidField(key)),
    }
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
    fn ring_mqtt_battery_with_default_filter() {
        let c = config(
            "sensor",
            r#"{"name": "Battery", "unique_id": "abc123_battery",
                "state_topic": "ring/lieu/camera/abc123/info/state",
                "device_class": "battery", "unit_of_measurement": "%",
                "value_template": "{{ value_json[\"batteryLevel\"] | default(\"\") }}",
                "device": {"ids": ["abc123"], "name": "Porte", "mf": "Ring"}}"#,
        );
        assert_eq!(c.unit_of_measurement.as_deref(), Some("%"));
        assert_eq!(
            c.value_template
                .render(r#"{"batteryLevel": 28}"#)
                .as_deref(),
            Some("28")
        );
        assert_eq!(c.value_template.render(r#"{"wirelessSignal": -54}"#), None);
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
