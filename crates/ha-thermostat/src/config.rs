//! Le fichier `thermostats.yaml`.
//!
//! ```yaml
//! - id: salon
//!   name: Chauffage du salon
//!   heater: switch.radiateur_salon      # la prise ou le relais du radiateur
//!   sensor: sensor.temperature_salon    # le thermomètre de la pièce
//!   window: binary_sensor.fenetre_salon # facultatif
//!   target: 19
//!   presets: {confort: 20, eco: 17, absent: 15, hors_gel: 7}
//! ```

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::time::Duration;

use ha_core::EntityId;
use serde::Deserialize;

const MAX_FILE_SIZE: u64 = 256 * 1024;
const MAX_THERMOSTATS: usize = 20;
const MAX_PRESETS: usize = 10;
/// Bornes absolues : jamais moins que le hors-gel, jamais plus qu'un sauna.
const LOWEST: f64 = 5.0;
const HIGHEST: f64 = 30.0;
const MAX_NAME_CHARS: usize = 60;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("lecture du fichier impossible : {0}")]
    Io(#[from] std::io::Error),

    #[error("fichier trop gros (256 Kio au plus)")]
    TooLarge,

    #[error("YAML invalide : {0}")]
    Yaml(String),

    #[error("thermostat « {id} » : {reason}")]
    Invalid { id: String, reason: String },

    #[error("{0}")]
    File(String),
}

/// Un thermostat, vérifié.
#[derive(Debug, Clone, PartialEq)]
pub struct Thermostat {
    pub id: String,
    pub name: String,
    pub heater: EntityId,
    pub sensor: EntityId,
    pub window: Option<EntityId>,
    pub min_temp: f64,
    pub max_temp: f64,
    /// Consigne au tout premier démarrage (ensuite, la dernière est gardée).
    pub target: f64,
    pub tolerance: f64,
    pub min_cycle: Duration,
    /// Nom → température, dans l'ordre alphabétique.
    pub presets: BTreeMap<String, f64>,
}

impl Thermostat {
    /// L'entité `climate.<id>`.
    #[must_use]
    pub fn entity_id(&self) -> Option<EntityId> {
        format!("climate.{}", self.id).parse().ok()
    }

    /// Ramène une consigne dans les bornes, au demi-degré.
    #[must_use]
    pub fn clamp(&self, temperature: f64) -> f64 {
        ((temperature * 2.0).round() / 2.0).clamp(self.min_temp, self.max_temp)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThermostatConfig {
    id: String,
    #[serde(default)]
    name: Option<String>,
    heater: String,
    sensor: String,
    #[serde(default)]
    window: Option<String>,
    #[serde(default = "default_min")]
    min_temp: f64,
    #[serde(default = "default_max")]
    max_temp: f64,
    #[serde(default = "default_target")]
    target: f64,
    #[serde(default = "default_tolerance")]
    tolerance: f64,
    #[serde(default)]
    min_cycle: Option<String>,
    #[serde(default)]
    presets: BTreeMap<String, f64>,
}

fn default_min() -> f64 {
    7.0
}
fn default_max() -> f64 {
    25.0
}
fn default_target() -> f64 {
    19.0
}
fn default_tolerance() -> f64 {
    0.3
}

/// Lit le fichier. Un fichier absent donne une liste vide.
pub fn load(path: &Path) -> Result<Vec<Thermostat>, ConfigError> {
    let Some(text) = ha_core::config_file::read(path, MAX_FILE_SIZE)
        .map_err(|error| error.into_error(ConfigError::TooLarge))?
    else {
        return Ok(Vec::new());
    };
    parse(&text)
}

/// Lit et vérifie le contenu du fichier.
pub fn parse(yaml: &str) -> Result<Vec<Thermostat>, ConfigError> {
    if yaml.trim().is_empty() {
        return Ok(Vec::new());
    }
    let configs: Vec<ThermostatConfig> =
        serde_saphyr::from_str(yaml).map_err(|error| ConfigError::Yaml(error.to_string()))?;
    if configs.len() > MAX_THERMOSTATS {
        return Err(ConfigError::File(format!(
            "{MAX_THERMOSTATS} thermostats au plus"
        )));
    }
    let mut ids = HashSet::new();
    let mut heaters = HashSet::new();
    let mut thermostats = Vec::with_capacity(configs.len());
    for config in configs {
        let id = config.id.clone();
        let thermostat = check(config).map_err(|reason| ConfigError::Invalid {
            id: id.clone(),
            reason,
        })?;
        if !ids.insert(id.clone()) {
            return Err(ConfigError::Invalid {
                id,
                reason: "identifiant en double".into(),
            });
        }
        // Deux thermostats qui commandent le même radiateur se battraient.
        if !heaters.insert(thermostat.heater.clone()) {
            return Err(ConfigError::Invalid {
                id,
                reason: format!(
                    "{} est déjà commandé par un autre thermostat",
                    thermostat.heater
                ),
            });
        }
        thermostats.push(thermostat);
    }
    Ok(thermostats)
}

fn check(config: ThermostatConfig) -> Result<Thermostat, String> {
    if !ha_core::is_valid_name(&config.id) {
        return Err("id : 1 à 64 caractères parmi a-z, 0-9 et « _ »".into());
    }
    let name = config.name.unwrap_or_else(|| config.id.replace('_', " "));
    if name.trim().is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(format!(
            "nom vide, trop long ({MAX_NAME_CHARS} caractères au plus) ou invalide"
        ));
    }
    let entity = |value: &str, field: &str, domain: &str| -> Result<EntityId, String> {
        let id: EntityId = value
            .parse()
            .map_err(|_| format!("{field} : identifiant invalide {value:?}"))?;
        if id.domain() != domain {
            return Err(format!("{field} : un {domain}.… est attendu, pas {id}"));
        }
        Ok(id)
    };
    let heater = entity(&config.heater, "heater", "switch")?;
    let sensor = entity(&config.sensor, "sensor", "sensor")?;
    let window = config
        .window
        .as_deref()
        .map(|w| entity(w, "window", "binary_sensor"))
        .transpose()?;

    let in_bounds = |value: f64| (LOWEST..=HIGHEST).contains(&value);
    if !in_bounds(config.min_temp) || !in_bounds(config.max_temp) {
        return Err(format!(
            "min_temp et max_temp : entre {LOWEST} et {HIGHEST} °C"
        ));
    }
    if config.min_temp >= config.max_temp {
        return Err("min_temp doit être inférieur à max_temp".into());
    }
    let in_range = |value: f64| (config.min_temp..=config.max_temp).contains(&value);
    if !in_range(config.target) {
        return Err(format!(
            "target : entre {} et {} °C",
            config.min_temp, config.max_temp
        ));
    }
    if !(0.1..=2.0).contains(&config.tolerance) {
        return Err("tolerance : entre 0,1 et 2 °C".into());
    }
    let min_cycle = match config.min_cycle.as_deref() {
        None => Duration::from_mins(5),
        Some(value) => parse_duration(value)
            .filter(|d| *d <= Duration::from_mins(30))
            .ok_or_else(|| {
                format!("min_cycle {value:?} invalide (exemples : 5min, 90s ; 30 min au plus)")
            })?,
    };
    if config.presets.len() > MAX_PRESETS {
        return Err(format!("{MAX_PRESETS} préréglages au plus"));
    }
    for (preset, value) in &config.presets {
        if !ha_core::is_valid_name(preset) {
            return Err(format!("préréglage {preset:?} : a-z, 0-9 et « _ »"));
        }
        if !in_range(*value) {
            return Err(format!(
                "préréglage {preset} : entre {} et {} °C",
                config.min_temp, config.max_temp
            ));
        }
    }
    Ok(Thermostat {
        id: config.id,
        name,
        heater,
        sensor,
        window,
        min_temp: config.min_temp,
        max_temp: config.max_temp,
        target: config.target,
        tolerance: config.tolerance,
        min_cycle,
        presets: config.presets,
    })
}

/// `90s`, `5min`.
fn parse_duration(value: &str) -> Option<Duration> {
    let split = value.find(|c: char| !c.is_ascii_digit())?;
    let amount: u64 = value[..split].parse().ok()?;
    match &value[split..] {
        "s" => Some(Duration::from_secs(amount)),
        "min" => Some(Duration::from_secs(amount.checked_mul(60)?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SALON: &str = "
- id: salon
  name: Chauffage du salon
  heater: switch.radiateur_salon
  sensor: sensor.temperature_salon
  window: binary_sensor.fenetre_salon
  presets: {confort: 20, eco: 17, hors_gel: 7}
";

    #[test]
    fn a_thermostat_is_read_with_defaults() {
        let thermostats = parse(SALON).unwrap();
        let t = &thermostats[0];
        assert_eq!(t.name, "Chauffage du salon");
        assert_eq!(t.entity_id().unwrap().as_str(), "climate.salon");
        assert!((t.target - 19.0).abs() < 1e-9);
        assert_eq!(t.min_cycle, Duration::from_mins(5));
        assert_eq!(
            t.presets.keys().collect::<Vec<_>>(),
            ["confort", "eco", "hors_gel"]
        );
        assert!((t.clamp(19.26) - 19.5).abs() < 1e-9);
        assert!((t.clamp(40.0) - 25.0).abs() < 1e-9);
        assert!((t.clamp(-3.0) - 7.0).abs() < 1e-9);
    }

    #[test]
    fn mistakes_are_refused() {
        for (from, to) in [
            ("switch.radiateur_salon", "light.radiateur_salon"),
            ("sensor.temperature_salon", "binary_sensor.temperature"),
            ("binary_sensor.fenetre_salon", "sensor.fenetre"),
            ("presets: {confort: 20", "presets: {confort: 35"),
            ("presets: {confort: 20", "presets: {Confort: 20"),
            ("  name: Chauffage du salon", "  name: ''"),
            ("  window:", "  fenetre:"),
            ("id: salon", "id: Salon"),
        ] {
            assert!(parse(&SALON.replace(from, to)).is_err(), "{to}");
        }
        for extra in [
            "  max_temp: 40",
            "  min_temp: 2",
            "  target: 28",
            "  tolerance: 5",
            "  min_cycle: 2h",
            "  min_cycle: 5",
        ] {
            assert!(parse(&format!("{SALON}{extra}\n")).is_err(), "{extra}");
        }
        // Deux thermostats sur le même radiateur.
        let twice = format!("{SALON}{}", SALON.replace("id: salon", "id: salon_bis"));
        assert!(parse(&twice).is_err());
        assert_eq!(parse("").unwrap(), Vec::new());
    }
}
