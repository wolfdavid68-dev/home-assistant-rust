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
use ha_sun::SunEvent;
use jiff::SignedDuration;
use jiff::civil::{Time, Weekday};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AutomationError;

/// Taille maximale du fichier.
pub const MAX_FILE_SIZE: usize = 1024 * 1024;
/// Nombre maximal d'automatisations, et d'éléments dans chacune.
pub const MAX_AUTOMATIONS: usize = 200;
const MAX_ITEMS: usize = 20;
/// Durée maximale d'une attente (`delay`, `for`).
const MAX_DURATION: Duration = Duration::from_hours(24);

// ───────────────────────────── 1. Ce qu'on lit ─────────────────────────────

/// Une valeur écrite comme du texte : `"on"`, `home`, `21`…
///
/// Un nombre sans guillemets (`state: 3`) est accepté et devient du texte,
/// comme dans Home Assistant. Mais `on`, `off`, `yes`, `true` sans guillemets
/// sont lus par YAML comme « vrai » ou « faux » : on les refuse avec un
/// message clair, plutôt que de deviner ce qui était voulu.
///
/// Concept Rust : on écrit ici la lecture « à la main », avec un `Visitor`.
/// Serde appelle la méthode qui correspond à ce qu'il a trouvé dans le
/// fichier (`visit_str` pour du texte, `visit_i64` pour un entier…).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct Text(String);

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = Text;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("du texte")
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Text, E> {
                Ok(Text(value.to_owned()))
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Text, E> {
                Ok(Text(value.to_string()))
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Text, E> {
                Ok(Text(value.to_string()))
            }

            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Text, E> {
                Ok(Text(value.to_string()))
            }

            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Text, E> {
                Err(E::custom(
                    "valeur lue comme vrai/faux : écris-la entre guillemets, par exemple \"on\" ou \"off\"",
                ))
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Concept Rust : `#[serde(untagged)]` essaie chaque variante dans l'ordre.
/// Ici, cela permet d'écrire `entity_id: light.salon` **ou** une liste.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum OneOrMany {
    One(Text),
    Many(Vec<Text>),
}

impl OneOrMany {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::One(value) => vec![value.0],
            Self::Many(values) => values.into_iter().map(|value| value.0).collect(),
        }
    }
}

/// Une automatisation **telle qu'elle est écrite**, avant vérification.
///
/// Concept Rust : la même structure sait se lire (`Deserialize`) et s'écrire
/// (`Serialize`). Une automatisation lue en YAML, ou envoyée en JSON par le
/// tableau de bord, passe toujours par elle : elle est donc réécrite sous une
/// forme unique (« canonique »), quel que soit le format d'origine.
///
/// Les champs sont privés : on ne peut obtenir une [`Automation`] qu'en
/// passant par [`AutomationConfig::check`].
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationConfig {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    alias: Option<String>,
    #[serde(default = "enabled_by_default", skip_serializing_if = "is_true")]
    enabled: bool,
    triggers: Vec<TriggerConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    conditions: Vec<ConditionConfig>,
    actions: Vec<ActionConfig>,
}

fn enabled_by_default() -> bool {
    true
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde passe toujours une référence"
)]
fn is_true(value: &bool) -> bool {
    *value
}

/// Concept Rust : `#[serde(tag = "trigger")]` choisit la variante d'après la
/// valeur du champ `trigger:` (`state`, `numeric_state` ou `time`).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "trigger", rename_all = "snake_case", deny_unknown_fields)]
enum TriggerConfig {
    State {
        entity_id: OneOrMany,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Text>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<Text>,
        #[serde(default, rename = "for", skip_serializing_if = "Option::is_none")]
        for_: Option<String>,
    },
    NumericState {
        entity_id: OneOrMany,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        above: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        below: Option<f64>,
    },
    Time {
        at: OneOrMany,
    },
    Sun {
        event: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "condition", rename_all = "snake_case", deny_unknown_fields)]
enum ConditionConfig {
    State {
        entity_id: OneOrMany,
        state: OneOrMany,
    },
    NumericState {
        entity_id: OneOrMany,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        above: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        below: Option<f64>,
    },
    Time {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        weekday: Option<OneOrMany>,
    },
    Sun {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after_offset: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before_offset: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum ActionConfig {
    Service(ServiceActionConfig),
    Delay(DelayActionConfig),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServiceActionConfig {
    /// `light.turn_on` (Home Assistant accepte aussi l'ancien nom `service`).
    #[serde(alias = "service")]
    action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target: Option<TargetConfig>,
    #[serde(default, skip_serializing_if = "Attributes::is_empty")]
    data: Attributes,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TargetConfig {
    entity_id: OneOrMany,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DelayActionConfig {
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
    /// Au lever, au coucher du soleil… avec un décalage (`-00:30:00` :
    /// une demi-heure avant).
    Sun {
        event: SunEvent,
        offset: SignedDuration,
    },
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
    /// Après / avant un événement du soleil (avec décalage).
    Sun {
        after: Option<(SunEvent, SignedDuration)>,
        before: Option<(SunEvent, SignedDuration)>,
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
            from: from.map(|text| text.0),
            to: to.map(|text| text.0),
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
        TriggerConfig::Sun { event, offset } => Trigger::Sun {
            event: parse_sun_event(&event)?,
            offset: offset
                .as_deref()
                .map(parse_offset)
                .transpose()?
                .unwrap_or_default(),
        },
    })
}

fn parse_sun_event(value: &str) -> Result<SunEvent, String> {
    SunEvent::parse(value).ok_or_else(|| {
        format!("événement du soleil {value:?} inconnu (sunrise, sunset, dawn ou dusk)")
    })
}

/// Décalage signé : `-00:30:00` (30 min avant), `01:00:00`, `+15min`…
fn parse_offset(value: &str) -> Result<SignedDuration, String> {
    let (negative, rest) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value.strip_prefix('+').unwrap_or(value)),
    };
    let duration = SignedDuration::try_from(parse_duration(rest)?)
        .map_err(|_| format!("décalage {value:?} invalide"))?;
    Ok(if negative { -duration } else { duration })
}

/// Les automatisations utilisent-elles le soleil ? (Il faut alors la
/// position de la maison.)
#[must_use]
pub fn uses_sun(automations: &[Automation]) -> bool {
    automations.iter().any(|automation| {
        automation
            .triggers
            .iter()
            .any(|t| matches!(t, Trigger::Sun { .. }))
            || automation
                .conditions
                .iter()
                .any(|c| matches!(c, Condition::Sun { .. }))
    })
}

impl Automation {
    /// Toutes les entités citées (déclencheurs, conditions, actions), triées.
    #[must_use]
    pub fn entity_ids(&self) -> Vec<&EntityId> {
        let mut ids: Vec<&EntityId> = Vec::new();
        for trigger in &self.triggers {
            if let Trigger::State { entity_ids, .. } | Trigger::NumericState { entity_ids, .. } =
                trigger
            {
                ids.extend(entity_ids);
            }
        }
        for condition in &self.conditions {
            if let Condition::State { entity_ids, .. }
            | Condition::NumericState { entity_ids, .. } = condition
            {
                ids.extend(entity_ids);
            }
        }
        for action in &self.actions {
            if let Action::Service { entity_ids, .. } = action {
                ids.extend(entity_ids);
            }
        }
        ids.sort_unstable();
        ids.dedup();
        ids
    }
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
        ConditionConfig::Sun {
            after,
            before,
            after_offset,
            before_offset,
        } => {
            if after.is_none() && before.is_none() {
                return Err("condition sun : il faut after ou before".into());
            }
            let with_offset = |event: Option<String>, offset: Option<String>| {
                if event.is_none() && offset.is_some() {
                    return Err("condition sun : décalage sans événement".to_owned());
                }
                event
                    .map(|event| {
                        Ok((
                            parse_sun_event(&event)?,
                            offset
                                .as_deref()
                                .map(parse_offset)
                                .transpose()?
                                .unwrap_or_default(),
                        ))
                    })
                    .transpose()
            };
            Condition::Sun {
                after: with_offset(after, after_offset)?,
                before: with_offset(before, before_offset)?,
            }
        }
    })
}

pub(crate) fn check_action(config: ActionConfig) -> Result<Action, String> {
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
                if !ha_core::is_valid_name(part) {
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
        assert_eq!(at[0], Time::new(6, 30, 0, 0).unwrap());
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

    #[test]
    fn durations_and_times() {
        assert_eq!(parse_duration("00:05:00").unwrap(), Duration::from_mins(5));
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_hours(2));
        for bad in ["", "5", "5 min", "-1s", "1d", "99:00:00"] {
            assert!(parse_duration(bad).is_err(), "{bad:?}");
        }
        assert!(parse_time("7:05").is_ok());
        for bad in ["7", "24:00", "07:60", "a:b"] {
            assert!(parse_time(bad).is_err(), "{bad:?}");
        }
    }
}
