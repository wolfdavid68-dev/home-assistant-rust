//! Le fichier `guard.yaml` : les pièces protégées et les heures calmes
//! (voir `ha_core::guard`).
//!
//! ```yaml
//! protected_areas: [Chambre, Bureau]     # des pièces de areas.yaml
//! protected_entities: [climate.bureau]   # et/ou des appareils
//! quiet_hours:
//!   from: "22:00"
//!   to: "07:00"
//! ```
//!
//! Sans pièce ni appareil protégés, les heures calmes valent pour toute la
//! maison. Un fichier absent : la garde ne fait rien.

use std::collections::HashMap;
use std::path::Path;

use ha_core::{EntityId, Policy, QuietHours};
use serde::Deserialize;

use crate::{Area, AreaError, MAX_FILE_SIZE};

const MAX_PROTECTED: usize = 200;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuardConfig {
    #[serde(default)]
    protected_areas: Vec<String>,
    #[serde(default)]
    protected_entities: Vec<String>,
    #[serde(default)]
    quiet_hours: Option<HoursConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HoursConfig {
    from: String,
    to: String,
}

/// Lit `guard.yaml`. Un fichier absent donne une garde qui ne fait rien.
pub fn load(path: &Path, areas: &[Area]) -> Result<Policy, AreaError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Policy::default());
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.len() > MAX_FILE_SIZE {
        return Err(AreaError::TooLarge);
    }
    parse(&std::fs::read_to_string(path)?, areas)
}

/// Lit et vérifie le contenu de `guard.yaml`, avec les pièces connues.
pub fn parse(yaml: &str, areas: &[Area]) -> Result<Policy, AreaError> {
    if yaml.trim().is_empty() {
        return Ok(Policy::default());
    }
    let config: GuardConfig =
        serde_saphyr::from_str(yaml).map_err(|error| AreaError::Yaml(error.to_string()))?;
    let mut protected = HashMap::new();
    for name in &config.protected_areas {
        let area = areas
            .iter()
            .find(|area| area.name.eq_ignore_ascii_case(name.trim()))
            .ok_or_else(|| {
                AreaError::Invalid(format!(
                    "pièce protégée inconnue : « {name} » (elle doit être dans areas.yaml)"
                ))
            })?;
        for entity in &area.entities {
            protected.insert(entity.entity_id.clone(), area.name.clone());
        }
    }
    for id in &config.protected_entities {
        let entity_id: EntityId = id.parse().map_err(|_| {
            AreaError::Invalid(format!(
                "appareil protégé invalide {id:?} (exemple : light.chambre)"
            ))
        })?;
        // Le nom de sa pièce s'il en a une, pour le message.
        let room = areas
            .iter()
            .find(|area| area.entities.iter().any(|e| e.entity_id == entity_id))
            .map_or_else(|| entity_id.to_string(), |area| area.name.clone());
        protected.entry(entity_id).or_insert(room);
    }
    if protected.len() > MAX_PROTECTED {
        return Err(AreaError::Invalid(format!(
            "{MAX_PROTECTED} appareils protégés au plus"
        )));
    }
    let quiet_hours = config
        .quiet_hours
        .map(|hours| -> Result<QuietHours, AreaError> {
            let quiet = QuietHours {
                from: minute(&hours.from)?,
                to: minute(&hours.to)?,
            };
            if quiet.from == quiet.to {
                return Err(AreaError::Invalid(
                    "heures calmes : le début et la fin sont identiques".into(),
                ));
            }
            Ok(quiet)
        })
        .transpose()?;
    Ok(Policy {
        protected,
        quiet_hours,
    })
}

/// « 22:00 » → 1320.
fn minute(text: &str) -> Result<u16, AreaError> {
    let invalid = || {
        AreaError::Invalid(format!(
            "heure invalide {text:?} (format HH:MM, par exemple \"22:00\")"
        ))
    };
    let (hours, minutes) = text.trim().split_once(':').ok_or_else(invalid)?;
    let hours: u16 = hours.parse().map_err(|_| invalid())?;
    let minutes: u16 = minutes.parse().map_err(|_| invalid())?;
    if hours > 23 || minutes > 59 {
        return Err(invalid());
    }
    Ok(hours * 60 + minutes)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    fn areas() -> Vec<Area> {
        crate::parse(
            "- area: Chambre\n  entities: [light.chambre, cover.volet_chambre]\n\
             - area: Salon\n  entities: [light.salon]\n",
        )
        .unwrap()
    }

    fn id(text: &str) -> EntityId {
        text.parse().unwrap()
    }

    #[test]
    fn protected_rooms_and_quiet_hours_are_read() {
        let policy = parse(
            "protected_areas: [chambre]\n\
             protected_entities: [climate.bureau, light.salon]\n\
             quiet_hours: {from: \"22:00\", to: \"07:30\"}\n",
            &areas(),
        )
        .unwrap();
        assert_eq!(policy.protected.len(), 4);
        assert_eq!(policy.protected[&id("cover.volet_chambre")], "Chambre");
        assert_eq!(policy.protected[&id("light.salon")], "Salon");
        assert_eq!(policy.protected[&id("climate.bureau")], "climate.bureau");
        assert_eq!(
            policy.quiet_hours,
            Some(QuietHours {
                from: 1320,
                to: 450
            })
        );
    }

    #[test]
    fn mistakes_are_reported() {
        for (yaml, expected) in [
            ("protected_areas: [Grenier]", "pièce protégée inconnue"),
            ("protected_entities: [chambre]", "appareil protégé invalide"),
            (
                "quiet_hours: {from: \"25:00\", to: \"07:00\"}",
                "heure invalide",
            ),
            (
                "quiet_hours: {from: \"7h\", to: \"08:00\"}",
                "heure invalide",
            ),
            (
                "quiet_hours: {from: \"07:00\", to: \"07:00\"}",
                "identiques",
            ),
            ("pieces: [Chambre]", "YAML invalide"),
        ] {
            let error = parse(yaml, &areas()).unwrap_err().to_string();
            assert!(error.contains(expected), "{yaml} : {error}");
        }
    }

    #[test]
    fn the_example_file_is_valid() {
        let areas = crate::parse(include_str!("../../../areas.example.yaml")).unwrap();
        let policy = parse(include_str!("../../../guard.example.yaml"), &areas).unwrap();
        assert!(policy.protected.contains_key(&id("climate.bureau")));
        assert!(policy.quiet_hours.is_some());
    }

    #[test]
    fn no_file_means_no_guard() {
        assert!(
            load(Path::new("/nonexistent/guard.yaml"), &[])
                .unwrap()
                .is_off()
        );
        assert!(parse("  \n", &[]).unwrap().is_off());
    }
}
