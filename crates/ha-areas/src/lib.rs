//! Les **pièces** de la maison, comme les « zones » (areas) de Home
//! Assistant : quels appareils sont dans le salon, dans la chambre…
//!
//! Elles se décrivent dans un fichier `areas.yaml`, et on peut aussi y
//! donner un nom lisible à chaque appareil :
//!
//! ```yaml
//! - area: Salon
//!   entities:
//!     - light.salon
//!     - entity: cover.volet_salon
//!       name: Volet
//! - area: Chambre
//!   entities: [cover.volet_chambre, sensor.temperature_chambre]
//! ```
//!
//! Le tableau de bord affiche alors une section par pièce, dans l'ordre du
//! fichier. Les appareils qui ne sont dans aucune pièce restent rangés par
//! type, en dessous.

use std::collections::HashSet;
use std::path::Path;

use ha_core::EntityId;
use serde::Deserialize;

/// Taille maximale du fichier.
const MAX_FILE_SIZE: u64 = 256 * 1024;
const MAX_AREAS: usize = 50;
const MAX_ENTITIES_PER_AREA: usize = 100;
const MAX_NAME_CHARS: usize = 60;

#[derive(Debug, thiserror::Error)]
pub enum AreaError {
    #[error("lecture du fichier impossible : {0}")]
    Io(#[from] std::io::Error),

    #[error("fichier trop gros (256 Kio au plus)")]
    TooLarge,

    #[error("YAML invalide : {0}")]
    Yaml(String),

    #[error("{0}")]
    Invalid(String),
}

/// Une pièce, vérifiée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Area {
    pub name: String,
    pub entities: Vec<AreaEntity>,
}

/// Un appareil d'une pièce, avec son nom lisible éventuel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AreaEntity {
    pub entity_id: EntityId,
    pub name: Option<String>,
}

// Ce qu'on lit dans le fichier, avant vérification.

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AreaConfig {
    area: String,
    #[serde(default)]
    entities: Vec<EntityConfig>,
}

/// Concept Rust : `untagged` accepte `light.salon` **ou**
/// `{entity: light.salon, name: Lampe}`.
#[derive(Deserialize)]
#[serde(untagged)]
enum EntityConfig {
    Id(String),
    Named(NamedEntity),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NamedEntity {
    entity: String,
    #[serde(default)]
    name: Option<String>,
}

/// Lit le fichier des pièces. Un fichier absent donne une liste vide.
pub fn load(path: &Path) -> Result<Vec<Area>, AreaError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    if metadata.len() > MAX_FILE_SIZE {
        return Err(AreaError::TooLarge);
    }
    parse(&std::fs::read_to_string(path)?)
}

/// Lit et vérifie le contenu d'un fichier de pièces.
pub fn parse(yaml: &str) -> Result<Vec<Area>, AreaError> {
    if yaml.trim().is_empty() {
        return Ok(Vec::new());
    }
    let configs: Vec<AreaConfig> =
        serde_saphyr::from_str(yaml).map_err(|error| AreaError::Yaml(error.to_string()))?;
    if configs.len() > MAX_AREAS {
        return Err(AreaError::Invalid(format!("{MAX_AREAS} pièces au plus")));
    }
    let mut area_names = HashSet::new();
    let mut seen = HashSet::new();
    let mut areas = Vec::with_capacity(configs.len());
    for config in configs {
        let name = check_name(&config.area)
            .map_err(|reason| AreaError::Invalid(format!("pièce {:?} : {reason}", config.area)))?;
        if !area_names.insert(name.to_lowercase()) {
            return Err(AreaError::Invalid(format!("pièce « {name} » en double")));
        }
        if config.entities.len() > MAX_ENTITIES_PER_AREA {
            return Err(AreaError::Invalid(format!(
                "pièce « {name} » : {MAX_ENTITIES_PER_AREA} appareils au plus"
            )));
        }
        let mut entities = Vec::with_capacity(config.entities.len());
        for entity in config.entities {
            let (id, label) = match entity {
                EntityConfig::Id(id) => (id, None),
                EntityConfig::Named(NamedEntity { entity, name }) => (entity, name),
            };
            let entity_id: EntityId = id.parse().map_err(|_| {
                AreaError::Invalid(format!(
                    "pièce « {name} » : identifiant d'appareil invalide {id:?} \
                     (exemple : light.salon)"
                ))
            })?;
            // Un appareil n'est que dans une pièce : sinon, lequel afficher ?
            if !seen.insert(entity_id.clone()) {
                return Err(AreaError::Invalid(format!(
                    "{entity_id} est dans deux pièces"
                )));
            }
            let label = label
                .map(|label| check_name(&label))
                .transpose()
                .map_err(|reason| AreaError::Invalid(format!("nom de {entity_id} : {reason}")))?;
            entities.push(AreaEntity {
                entity_id,
                name: label,
            });
        }
        areas.push(Area { name, entities });
    }
    Ok(areas)
}

/// Un nom affiché : non vide, pas trop long, sans caractère de contrôle.
fn check_name(name: &str) -> Result<String, &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("nom vide");
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err("nom trop long (60 caractères au plus)");
    }
    if name.chars().any(char::is_control) {
        return Err("caractère interdit dans le nom");
    }
    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn areas_and_names_are_read_in_order() {
        let areas = parse(
            "
- area: Salon
  entities:
    - light.salon
    - entity: cover.volet_salon
      name: Volet
- area: Chambre
  entities: [sensor.temperature_chambre]
- area: Garage
",
        )
        .unwrap();
        assert_eq!(areas.len(), 3);
        assert_eq!(areas[0].name, "Salon");
        assert_eq!(areas[0].entities[1].name.as_deref(), Some("Volet"));
        assert_eq!(
            areas[1].entities[0].entity_id.as_str(),
            "sensor.temperature_chambre"
        );
        assert_eq!(areas[2].entities, Vec::new());
        assert_eq!(parse("").unwrap(), Vec::new());
    }

    #[test]
    fn mistakes_are_reported() {
        for yaml in [
            // Faute de frappe dans un champ.
            "- {area: Salon, entites: [light.salon]}",
            "- {area: Salon, entities: [{entity: light.salon, nom: Lampe}]}",
            // Identifiant invalide.
            "- {area: Salon, entities: [Lampe du salon]}",
            // Nom vide, trop long, avec caractère de contrôle.
            "- {area: '  '}",
            "- {area: Salon, entities: [{entity: light.salon, name: \"a\\u001b[2J\"}]}",
            // Pièce en double, appareil dans deux pièces.
            "- {area: Salon}\n- {area: salon}",
            "- {area: Salon, entities: [light.a]}\n- {area: Cuisine, entities: [light.a]}",
        ] {
            assert!(parse(yaml).is_err(), "aurait dû être refusé : {yaml}");
        }
        let long = format!("- {{area: {}}}", "x".repeat(61));
        assert!(parse(&long).is_err());
    }

    #[test]
    fn missing_file_means_no_areas() {
        let path = std::env::temp_dir().join("ha-areas-inexistant-123456.yaml");
        assert_eq!(load(&path).unwrap(), Vec::new());
    }
}
