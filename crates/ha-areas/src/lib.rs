//! Les **pièces** de la maison, comme les « zones » (areas) de Home
//! Assistant : quels appareils sont dans le salon, dans la chambre…
//!
//! Elles se décrivent dans un fichier `areas.yaml`, et on peut aussi y
//! donner un nom lisible à chaque appareil, et d'**autres noms** (étape 94,
//! les « alias » de Home Assistant) qu'Assist comprend comme le nom :
//!
//! ```yaml
//! - area: Salon
//!   entities:
//!     - light.salon
//!     - entity: cover.volet_salon
//!       name: Volet
//!       aliases: [le store, la baie]
//! - area: Chambre
//!   entities: [cover.volet_chambre, sensor.temperature_chambre]
//! ```
//!
//! Le tableau de bord affiche alors une section par pièce, dans l'ordre du
//! fichier. Les appareils qui ne sont dans aucune pièce restent rangés par
//! type, en dessous.

pub mod guard;

use std::collections::HashSet;
use std::path::Path;

use ha_core::EntityId;
use serde::Deserialize;

/// Taille maximale du fichier.
const MAX_FILE_SIZE: u64 = 256 * 1024;
const MAX_AREAS: usize = 50;
const MAX_ENTITIES_PER_AREA: usize = 100;
const MAX_NAME_CHARS: usize = 60;
/// Autres noms d'un appareil, au plus.
pub const MAX_ALIASES: usize = 10;

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
    /// Ses autres noms (« la lampe du canapé », « le store »), compris par
    /// Assist comme le nom (étape 94). Vide le plus souvent.
    pub aliases: Vec<String>,
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
    #[serde(default)]
    aliases: Vec<String>,
}

/// Lit le fichier des pièces. Un fichier absent donne une liste vide.
pub fn load(path: &Path) -> Result<Vec<Area>, AreaError> {
    let Some(text) = ha_core::config_file::read(path, MAX_FILE_SIZE)
        .map_err(|error| error.into_error(AreaError::TooLarge))?
    else {
        return Ok(Vec::new());
    };
    parse(&text)
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
            let (id, label, aliases) = match entity {
                EntityConfig::Id(id) => (id, None, Vec::new()),
                EntityConfig::Named(NamedEntity {
                    entity,
                    name,
                    aliases,
                }) => (entity, name, aliases),
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
            let aliases = check_aliases(&aliases).map_err(|reason| {
                AreaError::Invalid(format!("autres noms de {entity_id} : {reason}"))
            })?;
            entities.push(AreaEntity {
                entity_id,
                name: label,
                aliases,
            });
        }
        areas.push(Area { name, entities });
    }
    Ok(areas)
}

/// Les autres noms d'un appareil : chacun vérifié comme un nom, sans
/// doublon (majuscules comprises), dix au plus.
fn check_aliases(aliases: &[String]) -> Result<Vec<String>, String> {
    if aliases.len() > MAX_ALIASES {
        return Err(format!("{MAX_ALIASES} au plus"));
    }
    let mut seen = HashSet::new();
    let mut checked = Vec::with_capacity(aliases.len());
    for alias in aliases {
        let alias = check_name(alias).map_err(|reason| format!("{alias:?} : {reason}"))?;
        if !seen.insert(alias.to_lowercase()) {
            return Err(format!("« {alias} » en double"));
        }
        checked.push(alias);
    }
    Ok(checked)
}

/// Les autres noms portés par plusieurs appareils (« le store » pour deux
/// volets) : permis, mais Assist demandera alors « Lequel ? ». Pour
/// prévenir à la vérification du fichier (`ha-server areas check`).
#[must_use]
pub fn shared_aliases(areas: &[Area]) -> Vec<(String, Vec<EntityId>)> {
    let mut owners: Vec<(String, String, Vec<EntityId>)> = Vec::new();
    for entity in areas.iter().flat_map(|area| &area.entities) {
        for alias in &entity.aliases {
            let key = alias.to_lowercase();
            match owners.iter_mut().find(|(known, _, _)| *known == key) {
                Some((_, _, ids)) => ids.push(entity.entity_id.clone()),
                None => owners.push((key, alias.clone(), vec![entity.entity_id.clone()])),
            }
        }
    }
    owners
        .into_iter()
        .filter(|(_, _, ids)| ids.len() > 1)
        .map(|(_, alias, ids)| (alias, ids))
        .collect()
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
    fn aliases_are_read_and_checked() {
        let areas = parse(
            "
- area: Salon
  entities:
    - entity: light.salon
      name: Lampe
      aliases: [' la lampe du canapé ', La grande lumière]
    - entity: cover.volet_salon
      aliases: [le store]
- area: Chambre
  entities:
    - {entity: cover.volet_chambre, aliases: [Le Store]}
",
        )
        .unwrap();
        let lamp = &areas[0].entities[0];
        assert_eq!(lamp.aliases, ["la lampe du canapé", "La grande lumière"]);
        assert_eq!(areas[0].entities[1].name, None);
        // « le store » est à deux volets : permis, mais signalé.
        let shared = shared_aliases(&areas);
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].0, "le store");
        assert_eq!(shared[0].1.len(), 2);

        for yaml in [
            "- {area: Salon, entities: [{entity: light.a, aliases: ['']}]}",
            "- {area: Salon, entities: [{entity: light.a, aliases: [lampe, Lampe]}]}",
            "- {area: Salon, entities: [{entity: light.a, aliases: [\"a\\u001b\"]}]}",
            "- {area: Salon, entities: [{entity: light.a, alias: [lampe]}]}",
        ] {
            assert!(parse(yaml).is_err(), "aurait dû être refusé : {yaml}");
        }
        let many: Vec<String> = (0..=MAX_ALIASES).map(|i| format!("nom {i}")).collect();
        let yaml = format!(
            "- {{area: Salon, entities: [{{entity: light.a, aliases: [{}]}}]}}",
            many.join(", ")
        );
        assert!(parse(&yaml).is_err());
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
