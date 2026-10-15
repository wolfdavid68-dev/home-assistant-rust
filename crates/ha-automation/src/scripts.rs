//! Scripts et scènes, comme dans Home Assistant.
//!
//! - Un **script** est une suite d'actions qu'on lance d'un bouton (ou depuis
//!   une automatisation) : « Je pars » = fermer les volets, attendre, couper
//!   la cafetière. Fichier `scripts.yaml`.
//! - Une **scène** est un état voulu pour plusieurs appareils : « Soirée
//!   cinéma » = lampe du salon à 20 %, volets fermés. Fichier `scenes.yaml`.
//!
//! Les deux se lancent avec `script.turn_on` / `scene.turn_on`, depuis le
//! tableau de bord ou une automatisation.

use std::collections::{BTreeMap, HashSet};

use ha_core::{Attributes, EntityId};
use serde::Deserialize;
use serde_json::Value;

use crate::config::{Action, ActionConfig, MAX_FILE_SIZE, check_action};
use crate::error::AutomationError;

/// Nombre maximal de scripts, et de scènes.
const MAX_ENTRIES: usize = 100;
/// Nombre maximal d'actions d'un script, ou d'appareils d'une scène.
const MAX_STEPS: usize = 50;

/// Un script vérifié.
#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    pub id: String,
    pub alias: String,
    pub actions: Vec<Action>,
}

/// Une scène vérifiée, déjà traduite en actions.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub id: String,
    pub name: String,
    pub actions: Vec<Action>,
}

impl Script {
    /// L'entité `script.<id>`.
    #[must_use]
    pub fn entity_id(&self) -> Option<EntityId> {
        format!("script.{}", self.id).parse().ok()
    }
}

impl Scene {
    /// L'entité `scene.<id>`.
    #[must_use]
    pub fn entity_id(&self) -> Option<EntityId> {
        format!("scene.{}", self.id).parse().ok()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptConfig {
    id: String,
    #[serde(default)]
    alias: Option<String>,
    sequence: Vec<ActionConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneConfig {
    id: String,
    #[serde(default)]
    name: Option<String>,
    /// Concept Rust : `BTreeMap` garde les appareils triés : l'ordre des
    /// actions ne dépend pas du hasard.
    entities: BTreeMap<String, SceneTarget>,
}

/// `light.salon: "on"` **ou** `light.salon: {state: "on", brightness: 50}`.
#[derive(Deserialize)]
#[serde(untagged)]
enum SceneTarget {
    State(String),
    Detailed(DetailedTarget),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DetailedTarget {
    #[serde(default)]
    state: Option<String>,
    /// Luminosité d'une lampe, de 1 à 255.
    #[serde(default)]
    brightness: Option<u8>,
    /// Position d'un volet, de 0 (fermé) à 100 (ouvert).
    #[serde(default)]
    position: Option<u8>,
}

/// Lit et vérifie le contenu de `scripts.yaml`.
pub fn parse_scripts(yaml: &str) -> Result<Vec<Script>, AutomationError> {
    let configs: Vec<ScriptConfig> = read(yaml)?;
    let mut ids = HashSet::new();
    configs
        .into_iter()
        .map(|config| {
            let id = config.id.clone();
            let invalid = |reason: String| AutomationError::InvalidEntry {
                kind: "script",
                id: id.clone(),
                reason,
            };
            check_id(&config.id, &mut ids).map_err(invalid)?;
            if config.sequence.is_empty() || config.sequence.len() > MAX_STEPS {
                return Err(invalid(format!("sequence : 1 à {MAX_STEPS} actions")));
            }
            let actions = config
                .sequence
                .into_iter()
                .map(check_action)
                .collect::<Result<Vec<_>, _>>()
                .map_err(invalid)?;
            Ok(Script {
                alias: display_name(config.alias, &config.id).map_err(invalid)?,
                id: config.id,
                actions,
            })
        })
        .collect()
}

/// Lit et vérifie le contenu de `scenes.yaml`.
pub fn parse_scenes(yaml: &str) -> Result<Vec<Scene>, AutomationError> {
    let configs: Vec<SceneConfig> = read(yaml)?;
    let mut ids = HashSet::new();
    configs
        .into_iter()
        .map(|config| {
            let id = config.id.clone();
            let invalid = |reason: String| AutomationError::InvalidEntry {
                kind: "scène",
                id: id.clone(),
                reason,
            };
            check_id(&config.id, &mut ids).map_err(invalid)?;
            if config.entities.is_empty() || config.entities.len() > MAX_STEPS {
                return Err(invalid(format!("entities : 1 à {MAX_STEPS} appareils")));
            }
            let actions = config
                .entities
                .into_iter()
                .map(|(entity, target)| scene_action(&entity, target))
                .collect::<Result<Vec<_>, _>>()
                .map_err(invalid)?;
            Ok(Scene {
                name: display_name(config.name, &config.id).map_err(invalid)?,
                id: config.id,
                actions,
            })
        })
        .collect()
}

/// Lit un fichier de scripts ou de scènes. Un fichier absent donne une liste vide.
pub fn load_file<T>(
    path: &std::path::Path,
    parse: impl Fn(&str) -> Result<Vec<T>, AutomationError>,
) -> Result<Vec<T>, AutomationError> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.len() > MAX_FILE_SIZE as u64 => Err(AutomationError::TooLarge),
        Ok(_) => parse(&std::fs::read_to_string(path)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

fn read<T: serde::de::DeserializeOwned>(yaml: &str) -> Result<Vec<T>, AutomationError> {
    if yaml.len() > MAX_FILE_SIZE {
        return Err(AutomationError::TooLarge);
    }
    if yaml.trim().is_empty() {
        return Ok(Vec::new());
    }
    let configs: Vec<T> =
        serde_saphyr::from_str(yaml).map_err(|error| AutomationError::Yaml(error.to_string()))?;
    if configs.len() > MAX_ENTRIES {
        return Err(AutomationError::TooMany(MAX_ENTRIES));
    }
    Ok(configs)
}

/// L'identifiant devient le nom de l'entité (`script.depart`) : minuscules,
/// chiffres et « _ » seulement, et pas de doublon.
fn check_id(id: &str, seen: &mut HashSet<String>) -> Result<(), String> {
    if !ha_core::is_valid_name(id) {
        return Err(
            "identifiant invalide (minuscules, chiffres et « _ », ex. soiree_cinema)".into(),
        );
    }
    if !seen.insert(id.to_owned()) {
        return Err("cet identifiant est déjà utilisé".into());
    }
    Ok(())
}

fn display_name(name: Option<String>, id: &str) -> Result<String, String> {
    let name = name.unwrap_or_else(|| id.replace('_', " "));
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 60 || name.chars().any(char::is_control) {
        return Err("nom invalide (1 à 60 caractères, sans caractère de contrôle)".into());
    }
    Ok(name.to_owned())
}

/// Traduit « cet appareil dans cet état » en action à exécuter.
fn scene_action(entity: &str, target: SceneTarget) -> Result<Action, String> {
    let entity_id: EntityId = entity
        .parse()
        .map_err(|_| format!("appareil {entity:?} invalide (exemple : light.salon)"))?;
    let (state, brightness, position) = match target {
        SceneTarget::State(state) => (Some(state), None, None),
        SceneTarget::Detailed(detail) => (detail.state, detail.brightness, detail.position),
    };
    let mut data = Attributes::new();
    let invalid = |what: &str| Err(format!("{entity_id} : {what}"));
    let service = match (entity_id.domain(), state.as_deref(), brightness, position) {
        ("light", Some("on") | None, Some(brightness), None) => {
            if brightness == 0 {
                return invalid("luminosité entre 1 et 255");
            }
            data.insert("brightness".into(), Value::from(brightness));
            "turn_on"
        }
        ("light" | "switch", Some("on"), None, None) => "turn_on",
        ("light" | "switch", Some("off"), None, None) => "turn_off",
        ("cover", Some("open"), None, None) => "open_cover",
        ("cover", Some("closed"), None, None) => "close_cover",
        ("cover", None, None, Some(position)) => {
            if position > 100 {
                return invalid("position entre 0 et 100");
            }
            data.insert("position".into(), Value::from(position));
            "set_cover_position"
        }
        ("light" | "switch" | "cover", _, _, _) => {
            return invalid(
                "état non compris (lampe : on/off et brightness, prise : on/off, \
                 volet : open/closed ou position)",
            );
        }
        _ => {
            return invalid(
                "une scène ne règle que des lampes, des prises et des volets \
                 (pas d'alarme ni de porte, par sécurité)",
            );
        }
    };
    Ok(Action::Service {
        domain: entity_id.domain().to_owned(),
        service: service.to_owned(),
        entity_ids: vec![entity_id],
        data,
        templates: crate::config::DataTemplates::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_are_read() {
        let scripts = parse_scripts(
            r"
- id: depart
  alias: Je pars
  sequence:
    - action: cover.close_cover
      target: {entity_id: [cover.volet_salon, cover.volet_chambre]}
    - delay: 5s
    - action: switch.turn_off
      target: {entity_id: switch.prise_cafe}
",
        )
        .unwrap();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].alias, "Je pars");
        assert_eq!(scripts[0].actions.len(), 3);
        assert_eq!(scripts[0].entity_id().unwrap().as_str(), "script.depart");
    }

    #[test]
    fn scenes_become_actions() {
        let scenes = parse_scenes(
            r#"
- id: soiree_cinema
  name: Soirée cinéma
  entities:
    light.salon: {brightness: 50}
    light.cuisine: "off"
    switch.prise_tv: "on"
    cover.volet_salon: closed
    cover.volet_chambre: {position: 30}
"#,
        )
        .unwrap();
        let scene = &scenes[0];
        assert_eq!(scene.name, "Soirée cinéma");
        let calls: Vec<String> = scene
            .actions
            .iter()
            .map(|action| match action {
                Action::Service {
                    domain,
                    service,
                    entity_ids,
                    data,
                    ..
                } => format!(
                    "{domain}.{service} {} {}",
                    entity_ids[0],
                    serde_json::Value::Object(data.clone())
                ),
                _ => "autre".into(),
            })
            .collect();
        assert_eq!(
            calls,
            [
                "cover.set_cover_position cover.volet_chambre {\"position\":30}",
                "cover.close_cover cover.volet_salon {}",
                "light.turn_off light.cuisine {}",
                "light.turn_on light.salon {\"brightness\":50}",
                "switch.turn_on switch.prise_tv {}",
            ]
        );
    }

    #[test]
    fn mistakes_are_reported() {
        for yaml in [
            // Identifiant invalide ou en double.
            "- {id: Départ, sequence: [{delay: 1s}]}",
            "- {id: a, sequence: [{delay: 1s}]}\n- {id: a, sequence: [{delay: 1s}]}",
            // Script vide, champ inconnu, action sans cible.
            "- {id: a, sequence: []}",
            "- {id: a, sequense: [{delay: 1s}]}",
            "- {id: a, sequence: [{action: light.turn_on}]}",
        ] {
            assert!(parse_scripts(yaml).is_err(), "script accepté : {yaml}");
        }
        for yaml in [
            // Domaine interdit dans une scène (alarme, serrure…).
            "- {id: a, entities: {alarm_control_panel.maison: disarmed}}",
            "- {id: a, entities: {lock.porte: unlocked}}",
            // États incompris ou hors limites.
            "- {id: a, entities: {light.salon: allumee}}",
            "- {id: a, entities: {light.salon: {brightness: 0}}}",
            "- {id: a, entities: {light.salon: {brightness: 300}}}",
            "- {id: a, entities: {cover.volet: {position: 150}}}",
            "- {id: a, entities: {cover.volet: \"on\"}}",
            "- {id: a, entities: {}}",
            "- {id: a, entities: {Salon: \"on\"}}",
        ] {
            assert!(parse_scenes(yaml).is_err(), "scène acceptée : {yaml}");
        }
    }
}
