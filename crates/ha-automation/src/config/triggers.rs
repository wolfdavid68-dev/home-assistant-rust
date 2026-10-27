//! La vérification des déclencheurs (`triggers:`), dont les motifs
//! horaires.

use ha_core::EntityId;

use super::MAX_ITEMS;
use super::check::check_label;
use super::model::{CalendarEdge, MAX_CALENDAR_OFFSET, Pattern, Trigger};
use super::parsing::{
    check_range, entity_ids, parse_duration, parse_offset, parse_sun_event, parse_time_at, values,
};
use super::read::{Text, TriggerConfig};

pub(super) fn check_trigger(config: TriggerConfig) -> Result<(Trigger, Option<String>), String> {
    let id = match &config {
        TriggerConfig::State { id, .. }
        | TriggerConfig::NumericState { id, .. }
        | TriggerConfig::Time { id, .. }
        | TriggerConfig::Sun { id, .. }
        | TriggerConfig::TimePattern { id, .. }
        | TriggerConfig::Event { id, .. }
        | TriggerConfig::Calendar { id, .. } => id.clone(),
    };
    let trigger = match config {
        TriggerConfig::State {
            entity_id,
            attribute,
            from,
            to,
            not_from,
            not_to,
            for_,
            ..
        } => Trigger::State {
            entity_ids: entity_ids(entity_id)?,
            attribute: attribute
                .map(|name| check_label(&name, "attribute"))
                .transpose()?,
            from: values(from, "from")?,
            to: values(to, "to")?,
            not_from: values(not_from, "not_from")?,
            not_to: values(not_to, "not_to")?,
            for_: for_.as_deref().map(parse_duration).transpose()?,
        },
        TriggerConfig::NumericState {
            entity_id,
            above,
            below,
            ..
        } => {
            check_range(above, below)?;
            Trigger::NumericState {
                entity_ids: entity_ids(entity_id)?,
                above,
                below,
            }
        }
        TriggerConfig::Time { at, .. } => Trigger::Time {
            at: at
                .into_vec()
                .iter()
                .map(|t| parse_time_at(t))
                .collect::<Result<_, _>>()?,
        },
        TriggerConfig::Sun { event, offset, .. } => Trigger::Sun {
            event: parse_sun_event(&event)?,
            offset: offset
                .as_deref()
                .map(parse_offset)
                .transpose()?
                .unwrap_or_default(),
        },
        TriggerConfig::TimePattern {
            hours,
            minutes,
            seconds,
            ..
        } => check_time_pattern(hours, minutes, seconds)?,
        TriggerConfig::Calendar {
            entity_id,
            event,
            offset,
            ..
        } => {
            let entity_id = EntityId::new(&entity_id).map_err(|error| error.to_string())?;
            if entity_id.domain() != "calendar" {
                return Err(format!("calendar : {entity_id} n'est pas un agenda"));
            }
            let edge = match event.as_str() {
                "start" => CalendarEdge::Start,
                "end" => CalendarEdge::End,
                _ => return Err(format!("calendar : event {event:?} (start ou end)")),
            };
            let offset = offset
                .as_deref()
                .map(parse_offset)
                .transpose()?
                .unwrap_or_default();
            if offset.abs() > MAX_CALENDAR_OFFSET {
                return Err("calendar : le décalage est d'une semaine au plus".into());
            }
            Trigger::Calendar {
                entity_id,
                edge,
                offset,
            }
        }
        TriggerConfig::Event {
            event_type,
            event_data,
            ..
        } => {
            if !ha_core::is_valid_event_type(&event_type) {
                return Err(format!(
                    "event_type {event_type:?} : 1 à 64 caractères parmi a-z, 0-9, « _ » et « . »"
                ));
            }
            if event_data.len() > MAX_ITEMS {
                return Err(format!("event_data : {MAX_ITEMS} valeurs au plus"));
            }
            Trigger::Event {
                event_type,
                event_data,
            }
        }
    };
    Ok((trigger, id))
}

/// Un motif horaire, comme dans Home Assistant : les morceaux non indiqués
/// **plus petits** que le plus petit indiqué valent 0, les **plus grands**
/// valent `*`. `minutes: "/15"` : toutes les heures, aux minutes 0, 15, 30
/// et 45, à la seconde 0.
fn check_time_pattern(
    hours: Option<Text>,
    minutes: Option<Text>,
    seconds: Option<Text>,
) -> Result<Trigger, String> {
    let given = [hours.is_some(), minutes.is_some(), seconds.is_some()];
    let Some(smallest) = given.iter().rposition(|&g| g) else {
        return Err("time_pattern : indique hours, minutes ou seconds".into());
    };
    let part = |value: Option<Text>, position: usize, max: i8, name: &str| match value {
        Some(text) => parse_pattern(&text.0, max).ok_or_else(|| {
            format!(
                "time_pattern : {name} {:?} invalide (exemples : 7, \"/15\", \"*\")",
                text.0
            )
        }),
        None if position > smallest => Ok(Pattern::At(0)),
        None => Ok(Pattern::Any),
    };
    let hours = part(hours, 0, 23, "hours")?;
    let minutes = part(minutes, 1, 59, "minutes")?;
    // Au plus une fois par minute : la protection contre les boucles arrête
    // une automatisation lancée plus de 10 fois par minute.
    let Pattern::At(second) = part(seconds, 2, 59, "seconds")? else {
        return Err(
            "time_pattern : seconds doit être une valeur fixe (0 à 59) : \
                    une automatisation se lance au plus une fois par minute"
                .into(),
        );
    };
    Ok(Trigger::TimePattern {
        hours,
        minutes,
        second,
    })
}

fn parse_pattern(text: &str, max: i8) -> Option<Pattern> {
    let text = text.trim();
    if text == "*" {
        return Some(Pattern::Any);
    }
    let (every, number) = match text.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    if number.is_empty() || number.len() > 2 || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let value: i8 = number.parse().ok()?;
    match every {
        true if (1..=max).contains(&value) => Some(Pattern::Every(value)),
        false if (0..=max).contains(&value) => Some(Pattern::At(value)),
        _ => None,
    }
}
