//! Le fichier `automations.yaml` : lecture et vérification.
//!
//! Deux temps :
//! 1. **lecture** : le YAML est transformé en structures Rust (`*Config`), avec
//!    `deny_unknown_fields` pour refuser toute faute de frappe ;
//! 2. **vérification** : chaque valeur est contrôlée (identifiants d'entités,
//!    heures, durées, limites) et l'on obtient des [`Automation`] prêtes à
//!    l'emploi, où plus rien ne peut être invalide.

use std::collections::HashSet;
use std::time::Duration;

use ha_core::{Attributes, EntityId};
use jiff::civil::{Time, Weekday};
use serde::Deserialize;

use crate::error::AutomationError;

/// Taille maximale du fichier.
pub const MAX_FILE_SIZE: usize = 1024 * 1024;
/// Nombre maximal d'automatisations, et d'éléments dans chacune.
pub const MAX_AUTOMATIONS: usize = 200;
const MAX_ITEMS: usize = 20;
/// Durée maximale d'une attente (`delay`, `for`).
const MAX_DURATION: Duration = Duration::from_secs(24 * 3600);

// ───────────────────────────── 1. Ce qu'on lit ─────────────────────────────

/// Concept Rust : `#[serde(untagged)]` essaie chaque variante dans l'ordre.
/// Ici, cela permet d'écrire `entity_id: light.salon` **ou** une liste.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::One(value) => vec![value],
            Self::Many(values) => values,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AutomationConfig {
    id: String,
    #[serde(default)]
    alias: Option<String>,
    #[serde(default = "enabled_by_default")]
    enabled: bool,
    triggers: Vec<TriggerConfig>,
    #[serde(default)]
    conditions: Vec<ConditionConfig>,
    actions: Vec<ActionConfig>,
}

fn enabled_by_default() -> bool {
    true
}

/// Concept Rust : `#[serde(tag = "trigger")]` choisit la variante d'après la
/// valeur du champ `trigger:` (`state`, `numeric_state` ou `time`).
#[derive(Debug, Deserialize)]
#[serde(tag = "trigger", rename_all = "snake_case", deny_unknown_fields)]
enum TriggerConfig {
    State {
        entity_id: OneOrMany,
        #[serde(default)]
        from: Option<String>,
        #[serde(default)]
        to: Option<String>,
        #[serde(default, rename = "for")]
        for_: Option<String>,
    },
    NumericState {
        entity_id: OneOrMany,
        #[serde(default)]
        above: Option<f64>,
        #[serde(default)]
        below: Option<f64>,
    },
    Time {
        at: OneOrMany,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "condition", rename_all = "snake_case", deny_unknown_fields)]
enum ConditionConfig {
    State {
        entity_id: OneOrMany,
        state: OneOrMany,
    },
    NumericState {
        entity_id: OneOrMany,
        #[serde(default)]
        above: Option<f64>,
        #[serde(default)]
        below: Option<f64>,
    },
    Time {
        #[serde(default)]
        after: Option<String>,
        #[serde(default)]
        before: Option<String>,
        #[serde(default)]
        weekday: Option<OneOrMany>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ActionConfig {
    Service(ServiceActionConfig),
    Delay(DelayActionConfig),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServiceActionConfig {
    /// `light.turn_on` (Home Assistant accepte aussi l'ancien nom `service`).
    #[serde(alias = "service")]
    action: String,
    #[serde(default)]
    target: Option<TargetConfig>,
    #[serde(default)]
    data: Attributes,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetConfig {
    entity_id: OneOrMany,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DelayActionConfig {
    delay: String,
}

// ─────────────────────── 2. Ce qu'on obtient, vérifié ───────────────────────

/// Une automatisation vérifiée.
#[derive(Debug, Clone, PartialEq)]
pub struct Automation {
    pub id: String,
    pub alias: String,
    pub triggers: Vec<Trigger>,
    pub conditions: Vec<Condition>,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Trigger {
    /// Une entité change de valeur (`from` → `to`), éventuellement depuis `for`.
    State {
        entity_ids: Vec<EntityId>,
        from: Option<String>,
        to: Option<String>,
        for_: Option<Duration>,
    },
    /// Une valeur numérique **entre** dans l'intervalle `]above, below[`.
    NumericState {
        entity_ids: Vec<EntityId>,
        above: Option<f64>,
        below: Option<f64>,
    },
    /// Tous les jours à une heure précise (heure locale).
    Time { at: Vec<Time> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    State {
        entity_ids: Vec<EntityId>,
        states: Vec<String>,
    },
    NumericState {
        entity_ids: Vec<EntityId>,
        above: Option<f64>,
        below: Option<f64>,
    },
    Time {
        after: Option<Time>,
        before: Option<Time>,
        weekdays: Vec<Weekday>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Service {
        domain: String,
        service: String,
        entity_ids: Vec<EntityId>,
        data: Attributes,
    },
    Delay(Duration),
}

/// Lit et vérifie le contenu d'un fichier `automations.yaml`.
pub fn parse(yaml: &str) -> Result<Vec<Automation>, AutomationError> {
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

    let mut ids = HashSet::new();
    let mut automations = Vec::new();
    for config in configs {
        let id = config.id.clone();
        if !ids.insert(id.clone()) {
            return Err(AutomationError::Invalid {
                id,
                reason: "cet identifiant est déjà utilisé".into(),
            });
        }
        let enabled = config.enabled;
        let automation = check(config).map_err(|reason| AutomationError::Invalid {
            id: id.clone(),
            reason,
        })?;
        if enabled {
            automations.push(automation);
        }
    }
    Ok(automations)
}

/// Vérifie une automatisation. L'erreur est un message en français.
fn check(config: AutomationConfig) -> Result<Automation, String> {
    let id_ok = (1..=64).contains(&config.id.len())
        && config
            .id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if !id_ok {
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

    let triggers = config
        .triggers
        .into_iter()
        .map(check_trigger)
        .collect::<Result<_, _>>()?;
    let conditions = config
        .conditions
        .into_iter()
        .map(check_condition)
        .collect::<Result<_, _>>()?;
    let actions = config
        .actions
        .into_iter()
        .map(check_action)
        .collect::<Result<_, _>>()?;
    Ok(Automation {
        id: config.id,
        alias,
        triggers,
        conditions,
        actions,
    })
}

fn check_trigger(config: TriggerConfig) -> Result<Trigger, String> {
    Ok(match config {
        TriggerConfig::State {
            entity_id,
            from,
            to,
            for_,
        } => Trigger::State {
            entity_ids: entity_ids(entity_id)?,
            from,
            to,
            for_: for_.as_deref().map(parse_duration).transpose()?,
        },
        TriggerConfig::NumericState {
            entity_id,
            above,
            below,
        } => {
            check_range(above, below)?;
            Trigger::NumericState {
                entity_ids: entity_ids(entity_id)?,
                above,
                below,
            }
        }
        TriggerConfig::Time { at } => Trigger::Time {
            at: at
                .into_vec()
                .iter()
                .map(|t| parse_time(t))
                .collect::<Result<_, _>>()?,
        },
    })
}

fn check_condition(config: ConditionConfig) -> Result<Condition, String> {
    Ok(match config {
        ConditionConfig::State { entity_id, state } => Condition::State {
            entity_ids: entity_ids(entity_id)?,
            states: state.into_vec(),
        },
        ConditionConfig::NumericState {
            entity_id,
            above,
            below,
        } => {
            check_range(above, below)?;
            Condition::NumericState {
                entity_ids: entity_ids(entity_id)?,
                above,
                below,
            }
        }
        ConditionConfig::Time {
            after,
            before,
            weekday,
        } => {
            if after.is_none() && before.is_none() && weekday.is_none() {
                return Err("condition time : il faut after, before ou weekday".into());
            }
            Condition::Time {
                after: after.as_deref().map(parse_time).transpose()?,
                before: before.as_deref().map(parse_time).transpose()?,
                weekdays: weekday
                    .map(OneOrMany::into_vec)
                    .unwrap_or_default()
                    .iter()
                    .map(|day| parse_weekday(day))
                    .collect::<Result<_, _>>()?,
            }
        }
    })
}

fn check_action(config: ActionConfig) -> Result<Action, String> {
    match config {
        ActionConfig::Delay(delay) => Ok(Action::Delay(parse_duration(&delay.delay)?)),
        ActionConfig::Service(service) => {
            let Some((domain, name)) = service.action.split_once('.') else {
                return Err(format!(
                    "action {:?} : format attendu domaine.service, par exemple light.turn_on",
                    service.action
                ));
            };
            for part in [domain, name] {
                let valid = (1..=64).contains(&part.len())
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
                if !valid {
                    return Err(format!("action {:?} invalide", service.action));
                }
            }
            let Some(target) = service.target else {
                return Err(format!(
                    "action {} : il faut une cible (target: entity_id: …)",
                    service.action
                ));
            };
            Ok(Action::Service {
                domain: domain.to_owned(),
                service: name.to_owned(),
                entity_ids: entity_ids(target.entity_id)?,
                data: service.data,
            })
        }
    }
}

fn entity_ids(value: OneOrMany) -> Result<Vec<EntityId>, String> {
    let ids = value.into_vec();
    if ids.is_empty() || ids.len() > MAX_ITEMS {
        return Err(format!("entity_id : 1 à {MAX_ITEMS} entités"));
    }
    ids.iter()
        .map(|id| EntityId::new(id).map_err(|error| error.to_string()))
        .collect()
}

fn check_range(above: Option<f64>, below: Option<f64>) -> Result<(), String> {
    match (above, below) {
        (None, None) => Err("numeric_state : il faut above ou below".into()),
        (Some(a), Some(b)) if a >= b => {
            Err("numeric_state : above doit être inférieur à below".into())
        }
        (a, b) if a.is_some_and(|v| !v.is_finite()) || b.is_some_and(|v| !v.is_finite()) => {
            Err("numeric_state : nombre invalide".into())
        }
        _ => Ok(()),
    }
}

/// `"07:30"` ou `"07:30:15"`.
fn parse_time(value: &str) -> Result<Time, String> {
    let parts: Vec<&str> = value.split(':').collect();
    let numbers: Option<Vec<i8>> = parts.iter().map(|p| p.parse().ok()).collect();
    let time = match (parts.len(), numbers) {
        (2, Some(n)) => Time::new(n[0], n[1], 0, 0),
        (3, Some(n)) => Time::new(n[0], n[1], n[2], 0),
        _ => {
            return Err(format!(
                "heure {value:?} invalide (format HH:MM ou HH:MM:SS)"
            ));
        }
    };
    time.map_err(|_| format!("heure {value:?} invalide"))
}

/// `"00:05:00"` (heures:minutes:secondes) ou `"30s"`, `"5min"`, `"2h"`.
fn parse_duration(value: &str) -> Result<Duration, String> {
    let invalid = || format!("durée {value:?} invalide (exemples : 00:05:00, 30s, 5min, 2h)");
    let duration = if value.contains(':') {
        let time = parse_time(value).map_err(|_| invalid())?;
        Duration::from_secs(
            u64::try_from(time.hour()).unwrap_or(0) * 3600
                + u64::try_from(time.minute()).unwrap_or(0) * 60
                + u64::try_from(time.second()).unwrap_or(0),
        )
    } else {
        let split = value
            .find(|c: char| !c.is_ascii_digit())
            .ok_or_else(invalid)?;
        let amount: u64 = value[..split].parse().map_err(|_| invalid())?;
        let seconds = match &value[split..] {
            "s" => amount,
            "min" => amount.saturating_mul(60),
            "h" => amount.saturating_mul(3600),
            _ => return Err(invalid()),
        };
        Duration::from_secs(seconds)
    };
    if duration > MAX_DURATION {
        return Err(format!("durée {value:?} trop longue (24 h au plus)"));
    }
    Ok(duration)
}

/// Jours en anglais (comme Home Assistant) ou en français.
fn parse_weekday(value: &str) -> Result<Weekday, String> {
    Ok(match value {
        "mon" | "lun" => Weekday::Monday,
        "tue" | "mar" => Weekday::Tuesday,
        "wed" | "mer" => Weekday::Wednesday,
        "thu" | "jeu" => Weekday::Thursday,
        "fri" | "ven" => Weekday::Friday,
        "sat" | "sam" => Weekday::Saturday,
        "sun" | "dim" => Weekday::Sunday,
        _ => return Err(format!("jour {value:?} invalide (mon…sun ou lun…dim)")),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

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
        assert_eq!(first.actions[1], Action::Delay(Duration::from_secs(600)));
        let Action::Service { entity_ids, .. } = &first.actions[2] else {
            panic!("service attendu");
        };
        assert_eq!(entity_ids.len(), 2);
        let second = &automations[1];
        assert_eq!(second.alias, "chauffage");
        let Trigger::Time { at } = &second.triggers[1] else {
            panic!("time attendu");
        };
        assert_eq!(at[0], Time::new(6, 30, 0, 0).unwrap());
    }

    #[test]
    fn mistakes_are_reported() {
        let cases = [
            // Faute de frappe dans un champ.
            "- id: a\n  triggers: [{trigger: state, entity_id: light.a, too: 'on'}]\n  actions: [{delay: 1s}]",
            // Déclencheur inconnu.
            "- id: a\n  triggers: [{trigger: sun, event: sunset}]\n  actions: [{delay: 1s}]",
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

    #[test]
    fn durations_and_times() {
        assert_eq!(
            parse_duration("00:05:00").unwrap(),
            Duration::from_secs(300)
        );
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(7200));
        for bad in ["", "5", "5 min", "-1s", "1d", "99:00:00"] {
            assert!(parse_duration(bad).is_err(), "{bad:?}");
        }
        assert!(parse_time("7:05").is_ok());
        for bad in ["7", "24:00", "07:60", "a:b"] {
            assert!(parse_time(bad).is_err(), "{bad:?}");
        }
    }
}
