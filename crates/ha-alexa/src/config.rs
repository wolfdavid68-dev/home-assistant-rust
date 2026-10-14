//! Le fichier `alexa.yaml` : la liste **explicite** des appareils qu'Alexa
//! peut piloter, avec le nom à prononcer.
//!
//! ```yaml
//! - entity: switch.prise_cafe
//!   name: Cafetière
//! - entity: cover.volet_salon
//!   name: Volet du salon
//! - light.salon          # sans nom : le nom de l'appareil est gardé
//! ```
//!
//! Sécurité : rien n'est exposé par défaut. Seuls les lampes, prises, volets,
//! scripts et scènes peuvent l'être ; jamais une serrure, une alarme, ni un
//! portail ou une porte de garage.

use std::collections::HashSet;
use std::path::Path;

use ha_core::EntityId;
use serde::Deserialize;

const MAX_FILE_SIZE: u64 = 256 * 1024;
/// Nombre maximal d'appareils exposés.
pub const MAX_DEVICES: usize = 100;
/// Un pont Hue accepte des noms de 32 caractères au plus.
const MAX_NAME_CHARS: usize = 32;

/// Les types d'appareils qu'Alexa peut piloter.
pub const ALLOWED_DOMAINS: [&str; 5] = ["light", "switch", "cover", "script", "scene"];

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("lecture du fichier impossible : {0}")]
    Io(#[from] std::io::Error),

    #[error("fichier trop gros (256 Kio au plus)")]
    TooLarge,

    #[error("YAML invalide : {0}")]
    Yaml(String),

    #[error("{0}")]
    Invalid(String),
}

/// Un appareil exposé à Alexa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub entity_id: EntityId,
    /// Le nom à prononcer (« Alexa, allume la **cafetière** »). `None` : le
    /// nom de l'appareil (`friendly_name`) est utilisé.
    pub name: Option<String>,
}

/// `light.salon` **ou** `{entity: light.salon, name: Lampe}`.
#[derive(Deserialize)]
#[serde(untagged)]
enum DeviceConfig {
    Id(String),
    Named(NamedDevice),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NamedDevice {
    entity: String,
    #[serde(default)]
    name: Option<String>,
}

/// Lit le fichier. Un fichier absent donne une liste vide.
pub fn load(path: &Path) -> Result<Vec<Device>, ConfigError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    if metadata.len() > MAX_FILE_SIZE {
        return Err(ConfigError::TooLarge);
    }
    parse(&std::fs::read_to_string(path)?)
}

/// Lit et vérifie le contenu du fichier.
pub fn parse(yaml: &str) -> Result<Vec<Device>, ConfigError> {
    if yaml.trim().is_empty() {
        return Ok(Vec::new());
    }
    let configs: Vec<DeviceConfig> =
        serde_saphyr::from_str(yaml).map_err(|error| ConfigError::Yaml(error.to_string()))?;
    if configs.len() > MAX_DEVICES {
        return Err(ConfigError::Invalid(format!(
            "{MAX_DEVICES} appareils au plus"
        )));
    }
    let mut seen = HashSet::new();
    let mut names = HashSet::new();
    let mut devices = Vec::with_capacity(configs.len());
    for config in configs {
        let (id, name) = match config {
            DeviceConfig::Id(id) => (id, None),
            DeviceConfig::Named(NamedDevice { entity, name }) => (entity, name),
        };
        let entity_id: EntityId = id.parse().map_err(|_| {
            ConfigError::Invalid(format!(
                "identifiant d'appareil invalide {id:?} (exemple : switch.prise_cafe)"
            ))
        })?;
        if !ALLOWED_DOMAINS.contains(&entity_id.domain()) {
            return Err(ConfigError::Invalid(format!(
                "{entity_id} : seuls les lampes, prises, volets, scripts et scènes peuvent \
                 être pilotés par Alexa ({})",
                ALLOWED_DOMAINS.join(", ")
            )));
        }
        if !seen.insert(entity_id.clone()) {
            return Err(ConfigError::Invalid(format!("{entity_id} est en double")));
        }
        if let Some(name) = &name {
            check_name(name)
                .map_err(|reason| ConfigError::Invalid(format!("{entity_id} : {reason}")))?;
            // Deux appareils du même nom : Alexa ne saurait pas lequel piloter.
            if !names.insert(name.to_lowercase()) {
                return Err(ConfigError::Invalid(format!(
                    "le nom « {name} » est utilisé deux fois"
                )));
            }
        }
        devices.push(Device { entity_id, name });
    }
    Ok(devices)
}

fn check_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("nom vide".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!(
            "nom trop long ({MAX_NAME_CHARS} caractères au plus)"
        ));
    }
    if name.chars().any(char::is_control) {
        return Err("nom avec des caractères de contrôle".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_are_read_in_order() {
        let devices = parse(
            "- entity: switch.prise_cafe\n  name: Cafetière\n- light.salon\n- {entity: cover.volet, name: Volet}\n",
        )
        .unwrap();
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].name.as_deref(), Some("Cafetière"));
        assert_eq!(devices[1].entity_id.as_str(), "light.salon");
        assert_eq!(devices[1].name, None);
        assert_eq!(parse("").unwrap(), Vec::new());
    }

    #[test]
    fn mistakes_and_dangerous_devices_are_refused() {
        for yaml in [
            "- lock.porte_entree",
            "- alarm_control_panel.maison",
            "- sensor.temperature",
            "- Lampe du salon",
            "- light.a\n- light.a",
            "- {entity: light.a, name: Lampe}\n- {entity: light.b, name: lampe}",
            "- {entity: light.a, name: ''}",
            "- {entity: light.a, name: \"un nom beaucoup trop long pour un pont hue\"}",
            "- {entity: light.a, nom: Lampe}",
            "pas une liste",
        ] {
            assert!(parse(yaml).is_err(), "aurait dû être refusé : {yaml}");
        }
        let many = (0..=MAX_DEVICES)
            .map(|n| format!("- light.l{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(parse(&many).is_err());
    }
}
