//! Chaque déclencheur, condition et action, décrit en français (pour
//! l'historique des exécutions).

use std::time::Duration;

use ha_core::EntityId;
use ha_sun::SunEvent;
use jiff::civil::Weekday;
use serde_json::Value;

use super::model::{Action, CalendarEdge, Condition, Pattern, RepeatUntil, TimeAt, Trigger};

impl Trigger {
    /// La raison, en français : « Porte d'entrée passe à « on » », « il est
    /// 07:30 », « coucher du soleil »… Les noms des appareils viennent de
    /// leur `friendly_name`.
    #[must_use]
    pub fn describe(&self, states: &ha_core::StateMachine) -> String {
        let names = |ids: &[EntityId]| friendly_names(states, ids, " ou ");
        match self {
            Self::State {
                entity_ids,
                attribute,
                to,
                ..
            } => {
                let subject = match attribute {
                    Some(attribute) => format!("{attribute} de {}", names(entity_ids)),
                    None => names(entity_ids),
                };
                if to.is_empty() {
                    format!("{subject} change")
                } else {
                    format!("{subject} passe à {}", quoted(to))
                }
            }
            Self::NumericState { entity_ids, .. } => {
                format!("{} passe un seuil", names(entity_ids))
            }
            Self::Time { at } => {
                let times: Vec<String> = at
                    .iter()
                    .map(|t| match t {
                        TimeAt::Fixed(time) => time.strftime("%H:%M").to_string(),
                        TimeAt::Entity(id) => {
                            format!("l'heure de {}", names(std::slice::from_ref(id)))
                        }
                    })
                    .collect();
                format!("il est {}", times.join(" ou "))
            }
            Self::Sun { event, .. } => match event {
                SunEvent::Sunrise => "lever du soleil",
                SunEvent::Sunset => "coucher du soleil",
                SunEvent::Dawn => "aube",
                SunEvent::Dusk => "crépuscule",
            }
            .to_owned(),
            Self::Event { event_type, .. } => format!("événement {event_type}"),
            Self::Calendar {
                entity_id, edge, ..
            } => match edge {
                CalendarEdge::Start => format!(
                    "un rendez-vous de {} commence",
                    names(std::slice::from_ref(entity_id))
                ),
                CalendarEdge::End => format!(
                    "un rendez-vous de {} se termine",
                    names(std::slice::from_ref(entity_id))
                ),
            },
            Self::TimePattern { hours, minutes, .. } => match (hours, minutes) {
                (Pattern::Any, Pattern::Every(step)) => format!("toutes les {step} minutes"),
                (Pattern::Every(step), Pattern::At(0)) => format!("toutes les {step} heures"),
                (Pattern::Any, Pattern::At(0)) => "toutes les heures".to_owned(),
                _ => "motif horaire".to_owned(),
            },
        }
    }
}

/// Les noms des entités (leur `friendly_name`, sinon leur identifiant),
/// séparés par `separator`.
fn friendly_names(states: &ha_core::StateMachine, ids: &[EntityId], separator: &str) -> String {
    ids.iter()
        .map(|id| {
            states
                .get(id)
                .and_then(|s| {
                    s.attributes()
                        .get("friendly_name")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| id.as_str().to_owned())
        })
        .collect::<Vec<_>>()
        .join(separator)
}

fn sun_name(event: SunEvent) -> &'static str {
    match event {
        SunEvent::Sunrise => "le lever du soleil",
        SunEvent::Sunset => "le coucher du soleil",
        SunEvent::Dawn => "l'aube",
        SunEvent::Dusk => "le crépuscule",
    }
}

/// « 5 min », « 1 h 30 min », « 45 s ».
fn duration_text(duration: Duration) -> String {
    let total = duration.as_secs();
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    let mut parts = Vec::new();
    if hours > 0 {
        parts.push(format!("{hours} h"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes} min"));
    }
    if seconds > 0 || parts.is_empty() {
        parts.push(format!("{seconds} s"));
    }
    parts.join(" ")
}

/// « « Orange » ou « Rouge » ».
fn quoted(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("« {value} »"))
        .collect::<Vec<_>>()
        .join(" ou ")
}

/// Un nombre sans « .0 » inutile : `20` et non `20.0`.
fn number_text(value: f64) -> String {
    let text = value.to_string();
    text.strip_suffix(".0").map_or(text.clone(), str::to_owned)
}

impl Condition {
    /// La condition, en français : « Porte d'entrée est « off » », « après
    /// 07:00 »… (pour l'historique des exécutions).
    #[must_use]
    pub fn describe(&self, states: &ha_core::StateMachine) -> String {
        match self {
            Self::State {
                entity_ids,
                attribute,
                states: expected,
            } => {
                let names = friendly_names(states, entity_ids, " et ");
                let subject = match attribute {
                    Some(attribute) => format!("{attribute} de {names}"),
                    None => names,
                };
                format!("{subject} est {}", quoted(expected))
            }
            Self::NumericState {
                entity_ids,
                above,
                below,
            } => {
                let names = friendly_names(states, entity_ids, " et ");
                match (above, below) {
                    (Some(above), Some(below)) => format!(
                        "{names} est entre {} et {}",
                        number_text(*above),
                        number_text(*below)
                    ),
                    (Some(above), None) => {
                        format!("{names} est au-dessus de {}", number_text(*above))
                    }
                    (None, Some(below)) => {
                        format!("{names} est en dessous de {}", number_text(*below))
                    }
                    (None, None) => format!("{names} est un nombre"),
                }
            }
            Self::Time {
                after,
                before,
                weekdays,
            } => {
                let mut parts = Vec::new();
                if let Some(after) = after {
                    parts.push(format!("après {}", after.strftime("%H:%M")));
                }
                if let Some(before) = before {
                    parts.push(format!("avant {}", before.strftime("%H:%M")));
                }
                if !weekdays.is_empty() {
                    let days: Vec<&str> = weekdays.iter().map(|day| weekday_name(*day)).collect();
                    parts.push(format!("le {}", days.join(", ")));
                }
                if parts.is_empty() {
                    "à toute heure".to_owned()
                } else {
                    parts.join(", ")
                }
            }
            Self::Sun { after, before } => {
                let mut parts = Vec::new();
                if let Some((event, _)) = after {
                    parts.push(format!("après {}", sun_name(*event)));
                }
                if let Some((event, _)) = before {
                    parts.push(format!("avant {}", sun_name(*event)));
                }
                parts.join(", ")
            }
            Self::And(list) => format!("toutes ces conditions ({})", list.len()),
            Self::Or(list) => format!("l'une de ces conditions ({})", list.len()),
            Self::Not(list) => format!("aucune de ces conditions ({})", list.len()),
            Self::Trigger(ids) => {
                let ids: Vec<String> = ids.iter().map(|id| format!("« {id} »")).collect();
                format!("lancée par le déclencheur {}", ids.join(" ou "))
            }
            Self::Template { source, .. } => format!("le modèle {} est vrai", source.trim()),
        }
    }
}

fn weekday_name(day: Weekday) -> &'static str {
    match day {
        Weekday::Monday => "lundi",
        Weekday::Tuesday => "mardi",
        Weekday::Wednesday => "mercredi",
        Weekday::Thursday => "jeudi",
        Weekday::Friday => "vendredi",
        Weekday::Saturday => "samedi",
        Weekday::Sunday => "dimanche",
    }
}

impl Action {
    /// L'action, en français (pour l'historique des exécutions). Les données
    /// envoyées (`data`) ne sont **pas** reprises : elles peuvent contenir un
    /// code d'alarme.
    #[must_use]
    pub fn describe(&self, states: &ha_core::StateMachine) -> String {
        match self {
            Self::Service {
                domain,
                service,
                entity_ids,
                ..
            } => format!(
                "{domain}.{service} sur {}",
                friendly_names(states, entity_ids, ", ")
            ),
            Self::Delay(duration) => format!("attendre {}", duration_text(*duration)),
            Self::If { .. } => "si… alors… sinon".to_owned(),
            Self::Choose { options, .. } => format!("choisir parmi {} option(s)", options.len()),
            Self::Condition(condition) => {
                format!("continuer si : {}", condition.describe(states))
            }
            Self::Repeat { until, .. } => match until {
                RepeatUntil::Count(count) => format!("répéter {count} fois"),
                RepeatUntil::While(_) => "répéter tant que…".to_owned(),
                RepeatUntil::Until(_) => "répéter jusqu'à ce que…".to_owned(),
            },
            Self::WaitForTrigger { timeout, .. } => match timeout {
                Some(timeout) => format!(
                    "attendre un déclencheur (au plus {})",
                    duration_text(*timeout)
                ),
                None => "attendre un déclencheur".to_owned(),
            },
        }
    }
}
