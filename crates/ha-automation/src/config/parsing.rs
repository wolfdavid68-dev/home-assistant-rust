//! Les petites lectures partagées : valeurs, entités, heures, durées,
//! jours, événements du soleil et décalages.

use std::time::Duration;

use ha_core::EntityId;
use ha_sun::SunEvent;
use jiff::SignedDuration;
use jiff::civil::{Time, Weekday};

use super::model::TimeAt;
use super::read::OneOrMany;
use super::{MAX_DURATION, MAX_ITEMS};

pub(super) fn parse_sun_event(value: &str) -> Result<SunEvent, String> {
    SunEvent::parse(value).ok_or_else(|| {
        format!("événement du soleil {value:?} inconnu (sunrise, sunset, dawn ou dusk)")
    })
}

/// Décalage signé : `-00:30:00` (30 min avant), `01:00:00`, `+15min`…
pub(super) fn parse_offset(value: &str) -> Result<SignedDuration, String> {
    let (negative, rest) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value.strip_prefix('+').unwrap_or(value)),
    };
    let duration = SignedDuration::try_from(parse_duration(rest)?)
        .map_err(|_| format!("décalage {value:?} invalide"))?;
    Ok(if negative { -duration } else { duration })
}

/// Une ou plusieurs valeurs (`to: Orange` ou `to: [Orange, Rouge]`).
pub(super) fn values(value: Option<OneOrMany>, name: &str) -> Result<Vec<String>, String> {
    let list = value.map(OneOrMany::into_vec).unwrap_or_default();
    if list.len() > MAX_ITEMS {
        return Err(format!("{name} : {MAX_ITEMS} valeurs au plus"));
    }
    if list.iter().any(|v| v.chars().count() > 255) {
        return Err(format!("{name} : valeur trop longue"));
    }
    Ok(list)
}

pub(super) fn entity_ids(value: OneOrMany) -> Result<Vec<EntityId>, String> {
    let ids = value.into_vec();
    if ids.is_empty() || ids.len() > MAX_ITEMS {
        return Err(format!("entity_id : 1 à {MAX_ITEMS} entités"));
    }
    ids.iter()
        .map(|id| EntityId::new(id).map_err(|error| error.to_string()))
        .collect()
}

pub(super) fn check_range(above: Option<f64>, below: Option<f64>) -> Result<(), String> {
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
/// `07:30`, ou une entrée `input_datetime.reveil`.
pub(super) fn parse_time_at(value: &str) -> Result<TimeAt, String> {
    if !value.contains('.') {
        return parse_time(value).map(TimeAt::Fixed);
    }
    let entity_id = EntityId::new(value).map_err(|error| error.to_string())?;
    if entity_id.domain() != "input_datetime" {
        return Err(format!(
            "time : {entity_id} n'est ni une heure (07:30) ni une entrée input_datetime"
        ));
    }
    Ok(TimeAt::Entity(entity_id))
}

pub(super) fn parse_time(value: &str) -> Result<Time, String> {
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
pub(super) fn parse_duration(value: &str) -> Result<Duration, String> {
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
pub(super) fn parse_weekday(value: &str) -> Result<Weekday, String> {
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
    use std::time::Duration;

    use super::*;

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
