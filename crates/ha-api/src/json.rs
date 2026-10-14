//! Conversion des états en JSON, au même format que l'API de Home Assistant.

use std::time::{SystemTime, UNIX_EPOCH};

use ha_core::State;
use serde_json::{Value, json};

/// `{"entity_id": …, "state": …, "attributes": {…}, "last_changed": …, "last_updated": …}`
pub fn state_to_json(state: &State) -> Value {
    json!({
        "entity_id": state.entity_id().as_str(),
        "state": state.state(),
        "attributes": state.attributes(),
        "last_changed": format_timestamp(state.last_changed()),
        "last_updated": format_timestamp(state.last_updated()),
    })
}

/// Date au format ISO 8601 / RFC 3339 en UTC : `2026-10-14T17:01:59.780320Z`.
///
/// On le calcule nous-mêmes (quelques lignes) plutôt que d'ajouter une
/// bibliothèque de dates entière : moins de dépendances, moins de risques.
pub fn format_timestamp(time: SystemTime) -> String {
    let since_epoch = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since_epoch.as_secs();
    let (year, month, day) = civil_from_days(i64::try_from(secs / 86_400).unwrap_or(0));
    let rest = secs % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:06}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60,
        since_epoch.subsec_micros()
    )
}

/// Nombre de jours depuis le 1er janvier 1970 → (année, mois, jour).
/// Algorithme de Howard Hinnant (calendrier grégorien).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn formats_known_dates() {
        let at = |secs: u64, micros: u64| {
            format_timestamp(UNIX_EPOCH + Duration::from_secs(secs) + Duration::from_micros(micros))
        };
        assert_eq!(at(0, 0), "1970-01-01T00:00:00.000000Z");
        // 29 février d'une année bissextile.
        assert_eq!(at(951_782_400, 0), "2000-02-29T00:00:00.000000Z");
        assert_eq!(at(1_700_000_000, 123_456), "2023-11-14T22:13:20.123456Z");
        assert_eq!(at(1_798_761_599, 0), "2026-12-31T23:59:59.000000Z");
    }
}
