//! Les points d'entrée (lire un fichier, vérifier une liste) et la
//! vérification d'une automatisation entière, avec son budget de blocs.

use std::collections::HashSet;

use serde_json::Value;

use crate::error::AutomationError;

use super::actions::check_action_in;
use super::conditions::check_condition;
use super::model::Automation;
use super::read::{AutomationConfig, DEFAULT_MAX_RUNS, MAX_MAX_RUNS, RunMode};
use super::triggers::check_trigger;
use super::{MAX_AUTOMATIONS, MAX_FILE_SIZE, MAX_ITEMS};

/// Lit et vérifie le contenu d'un fichier `automations.yaml`. Les
/// automatisations désactivées sont vérifiées, mais pas renvoyées.
pub fn parse(yaml: &str) -> Result<Vec<Automation>, AutomationError> {
    check_all(&parse_configs(yaml)?)
}

/// Lit un fichier `automations.yaml` **sans** vérifier les valeurs : seule la
/// forme (champs connus, types) est contrôlée. Voir [`check_all`].
pub fn parse_configs(yaml: &str) -> Result<Vec<AutomationConfig>, AutomationError> {
    if yaml.len() > MAX_FILE_SIZE {
        return Err(AutomationError::TooLarge);
    }
    if yaml.trim().is_empty() {
        return Ok(Vec::new());
    }
    let configs: Vec<AutomationConfig> =
        serde_saphyr::from_str(yaml).map_err(|error| AutomationError::Yaml(error.to_string()))?;
    if configs.len() > MAX_AUTOMATIONS {
        return Err(AutomationError::TooMany(MAX_AUTOMATIONS));
    }
    Ok(configs)
}

/// Vérifie une liste d'automatisations (identifiants uniques compris), et
/// renvoie celles qui sont activées.
pub fn check_all(configs: &[AutomationConfig]) -> Result<Vec<Automation>, AutomationError> {
    let mut ids = HashSet::new();
    let mut automations = Vec::new();
    for config in configs {
        if !ids.insert(config.id.as_str()) {
            return Err(AutomationError::Invalid {
                id: config.id.clone(),
                reason: "cet identifiant est déjà utilisé".into(),
            });
        }
        let automation = config.check()?;
        if config.enabled {
            automations.push(automation);
        }
    }
    Ok(automations)
}

impl AutomationConfig {
    /// Lit **une** automatisation écrite en YAML (une liste d'un seul
    /// élément, copiée depuis `automations.yaml`, est aussi acceptée).
    pub fn from_yaml(yaml: &str) -> Result<Self, AutomationError> {
        if yaml.len() > MAX_FILE_SIZE {
            return Err(AutomationError::TooLarge);
        }
        let yaml_error = |error: serde_saphyr::Error| AutomationError::Yaml(error.to_string());
        if yaml.trim_start().starts_with('-') {
            let mut list: Vec<Self> = serde_saphyr::from_str(yaml).map_err(yaml_error)?;
            return match (list.pop(), list.is_empty()) {
                (Some(config), true) => Ok(config),
                _ => Err(AutomationError::Yaml(
                    "une seule automatisation à la fois".into(),
                )),
            };
        }
        serde_saphyr::from_str(yaml).map_err(yaml_error)
    }

    /// Lit une automatisation envoyée en JSON par le tableau de bord.
    pub fn from_json(value: Value) -> Result<Self, AutomationError> {
        serde_json::from_value(value).map_err(|error| AutomationError::Yaml(error.to_string()))
    }

    /// La forme JSON, pour la base et le tableau de bord.
    #[must_use]
    pub fn to_json(&self) -> Value {
        // Ne peut pas échouer : toutes les clés sont des chaînes, et les
        // nombres sont finis (vérifiés à la lecture).
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// La forme YAML, pour l'affichage et l'édition.
    #[must_use]
    pub fn to_yaml(&self) -> String {
        serde_saphyr::to_string(self).unwrap_or_default()
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Le nom affiché (l'identifiant s'il n'y en a pas).
    #[must_use]
    pub fn alias(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.id)
    }

    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Vérifie chaque valeur et renvoie l'automatisation prête à l'emploi.
    pub fn check(&self) -> Result<Automation, AutomationError> {
        check(self.clone()).map_err(|reason| AutomationError::Invalid {
            id: self.id.clone(),
            reason,
        })
    }
}

/// Vérifie une automatisation. L'erreur est un message en français.
fn check(config: AutomationConfig) -> Result<Automation, String> {
    if !ha_core::is_valid_name(&config.id) {
        return Err("id : 1 à 64 caractères parmi a-z, 0-9 et « _ »".into());
    }
    let alias = config.alias.unwrap_or_else(|| config.id.clone());
    if alias.chars().count() > 255 || alias.chars().any(char::is_control) {
        return Err("alias trop long ou avec des caractères de contrôle".into());
    }
    if config.triggers.is_empty() || config.actions.is_empty() {
        return Err("il faut au moins un déclencheur (triggers) et une action (actions)".into());
    }
    for (name, len) in [
        ("triggers", config.triggers.len()),
        ("conditions", config.conditions.len()),
        ("actions", config.actions.len()),
    ] {
        if len > MAX_ITEMS {
            return Err(format!("{name} : {MAX_ITEMS} éléments au plus"));
        }
    }

    let max = match (config.mode, config.max) {
        (RunMode::Queued | RunMode::Parallel, Some(max)) if (1..=MAX_MAX_RUNS).contains(&max) => {
            max
        }
        (RunMode::Queued | RunMode::Parallel, Some(_)) => {
            return Err(format!("max : entre 1 et {MAX_MAX_RUNS}"));
        }
        (RunMode::Queued | RunMode::Parallel, None) => DEFAULT_MAX_RUNS,
        (_, Some(_)) => return Err("max ne sert qu'avec mode: queued ou parallel".into()),
        (_, None) => 1,
    };

    let mut triggers = Vec::with_capacity(config.triggers.len());
    let mut trigger_ids = Vec::with_capacity(config.triggers.len());
    for (index, trigger) in config.triggers.into_iter().enumerate() {
        let (trigger, id) = check_trigger(trigger)?;
        trigger_ids.push(match id {
            Some(id) => check_label(&id, "id de déclencheur")?,
            None => index.to_string(),
        });
        triggers.push(trigger);
    }
    let mut budget = Budget::default();
    let conditions = config
        .conditions
        .into_iter()
        .map(|c| check_condition(c, 0, &mut budget))
        .collect::<Result<_, _>>()?;
    let actions = config
        .actions
        .into_iter()
        .map(|a| check_action_in(a, 0, &mut budget))
        .collect::<Result<_, _>>()?;
    Ok(Automation {
        id: config.id,
        alias,
        triggers,
        trigger_ids,
        conditions,
        actions,
        mode: config.mode,
        max: usize::from(max),
    })
}

/// Profondeur maximale des blocs imbriqués (`if` dans `choose` dans…), et
/// nombre maximal d'actions et de conditions dans une automatisation : un
/// fichier ne peut pas construire une automatisation démesurée.
const MAX_DEPTH: usize = 5;
const MAX_NODES: usize = 200;

#[derive(Default)]
pub(super) struct Budget {
    nodes: usize,
}

impl Budget {
    pub(super) fn take(&mut self, depth: usize) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Err(format!("blocs imbriqués : {MAX_DEPTH} niveaux au plus"));
        }
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(format!("{MAX_NODES} actions et conditions au plus"));
        }
        Ok(())
    }
}

/// Un identifiant libre (id de déclencheur) : court et sans caractère de contrôle.
pub(super) fn check_label(value: &str, what: &str) -> Result<String, String> {
    if value.is_empty() || value.chars().count() > 64 || value.chars().any(char::is_control) {
        return Err(format!("{what} {value:?} : 1 à 64 caractères"));
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use jiff::civil::Time;

    use super::*;
    use crate::config::{Action, MAX_FILE_SIZE, TimeAt, Trigger};

    const EXAMPLE: &str = r#"
- id: salon_au_retour
  alias: Allumer le salon quand on rentre le soir
  triggers:
    - trigger: state
      entity_id: binary_sensor.porte_entree
      to: "on"
  conditions:
    - condition: time
      after: "18:00"
      before: "23:30"
    - condition: state
      entity_id: light.salon
      state: "off"
  actions:
    - action: light.turn_on
      target:
        entity_id: light.salon
      data:
        brightness: 180
    - delay: 10min
    - action: light.turn_off
      target:
        entity_id: [light.salon, light.couloir]

- id: chauffage
  triggers:
    - trigger: numeric_state
      entity_id: sensor.temperature_salon
      below: 18.5
    - trigger: time
      at: ["06:30", "18:00:00"]
  conditions:
    - condition: time
      weekday: [lun, mar, wed]
  actions:
    - service: switch.turn_on
      target:
        entity_id: switch.radiateur

- id: desactivee
  enabled: false
  triggers: [{trigger: time, at: "12:00"}]
  actions: [{delay: 1s}]
"#;

    #[test]
    fn example_file_is_understood() {
        let automations = parse(EXAMPLE).unwrap();
        assert_eq!(
            automations.len(),
            2,
            "l'automatisation désactivée est ignorée"
        );
        let first = &automations[0];
        assert_eq!(first.alias, "Allumer le salon quand on rentre le soir");
        assert_eq!(first.actions[1], Action::Delay(Duration::from_mins(10)));
        let Action::Service { entity_ids, .. } = &first.actions[2] else {
            panic!("service attendu");
        };
        assert_eq!(entity_ids.len(), 2);
        let second = &automations[1];
        assert_eq!(second.alias, "chauffage");
        let Trigger::Time { at } = &second.triggers[1] else {
            panic!("time attendu");
        };
        assert_eq!(at[0], TimeAt::Fixed(Time::new(6, 30, 0, 0).unwrap()));
    }

    #[test]
    fn mistakes_are_reported() {
        let cases = [
            // Faute de frappe dans un champ.
            "- id: a\n  triggers: [{trigger: state, entity_id: light.a, too: 'on'}]\n  actions: [{delay: 1s}]",
            // Déclencheur inconnu.
            "- id: a\n  triggers: [{trigger: moon, event: moonrise}]\n  actions: [{delay: 1s}]",
            // Identifiant d'entité invalide.
            "- id: a\n  triggers: [{trigger: state, entity_id: Salon}]\n  actions: [{delay: 1s}]",
            // Heure invalide.
            "- id: a\n  triggers: [{trigger: time, at: '25:00'}]\n  actions: [{delay: 1s}]",
            // Durée trop longue.
            "- id: a\n  triggers: [{trigger: time, at: '12:00'}]\n  actions: [{delay: 48h}]",
            // Action sans cible.
            "- id: a\n  triggers: [{trigger: time, at: '12:00'}]\n  actions: [{action: light.turn_on}]",
            // Action mal écrite.
            "- id: a\n  triggers: [{trigger: time, at: '12:00'}]\n  actions: [{action: allume, target: {entity_id: light.a}}]",
            // Pas d'action.
            "- id: a\n  triggers: [{trigger: time, at: '12:00'}]\n  actions: []",
            // Identifiant d'automatisation invalide.
            "- id: Mon Automatisation\n  triggers: [{trigger: time, at: '12:00'}]\n  actions: [{delay: 1s}]",
            // Intervalle impossible.
            "- id: a\n  triggers: [{trigger: numeric_state, entity_id: sensor.t, above: 20, below: 10}]\n  actions: [{delay: 1s}]",
            // Doublon d'identifiant.
            "- id: a\n  triggers: [{trigger: time, at: '12:00'}]\n  actions: [{delay: 1s}]\n- id: a\n  triggers: [{trigger: time, at: '13:00'}]\n  actions: [{delay: 1s}]",
            // Pas du YAML attendu.
            "ceci n'est pas une liste",
        ];
        for yaml in cases {
            assert!(parse(yaml).is_err(), "aurait dû être refusé :\n{yaml}");
        }
    }

    #[test]
    fn yaml_bombs_are_refused() {
        // « Billion laughs » : des alias qui se multiplient.
        let bomb = "a: &a [x,x,x,x,x,x,x,x,x,x]\nb: &b [*a,*a,*a,*a,*a,*a,*a,*a,*a,*a]\nc: &c [*b,*b,*b,*b,*b,*b,*b,*b,*b,*b]\nd: &d [*c,*c,*c,*c,*c,*c,*c,*c,*c,*c]\ne: &e [*d,*d,*d,*d,*d,*d,*d,*d,*d,*d]\nf: &f [*e,*e,*e,*e,*e,*e,*e,*e,*e,*e]\ng: [*f,*f,*f,*f,*f,*f,*f,*f,*f,*f]\n";
        // Même lu comme une valeur quelconque, il est refusé par la bibliothèque.
        let generic: Result<serde_json::Value, _> = serde_saphyr::from_str(bomb);
        let error = generic.unwrap_err().to_string();
        assert!(
            error.contains("budget") || error.contains("alias"),
            "{error}"
        );
        assert!(parse(bomb).is_err());
        assert!(matches!(
            parse(&"#".repeat(MAX_FILE_SIZE + 1)),
            Err(AutomationError::TooLarge)
        ));
    }
}
