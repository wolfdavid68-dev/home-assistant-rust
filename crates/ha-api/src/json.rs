//! Conversion des états en JSON, au même format que l'API de Home Assistant.

use std::time::{SystemTime, UNIX_EPOCH};

use ha_core::{Attributes, State};
use serde::{Serialize, Serializer};

/// L'ancien chemin, par un arbre `serde_json::Value` : gardé seulement comme
/// référence, pour vérifier que [`StateJson`] écrit exactement le même texte.
#[cfg(test)]
pub(crate) fn state_to_json(state: &State) -> serde_json::Value {
    serde_json::json!({
        "entity_id": state.entity_id().as_str(),
        "state": state.state(),
        "attributes": state.attributes(),
        "last_changed": format_timestamp(state.last_changed()),
        "last_updated": format_timestamp(state.last_updated()),
    })
}

/// Un état vu en JSON, écrit directement depuis l'état : rien n'est copié
/// dans un arbre `serde_json::Value` intermédiaire.
///
/// Les champs suivent l'ordre alphabétique, celui que donnait jusqu'ici
/// `serde_json::Value` (ses objets sont triés) : la sortie reste identique,
/// octet pour octet.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct StateJson<'a> {
    attributes: &'a Attributes,
    entity_id: &'a str,
    #[serde(serialize_with = "timestamp")]
    last_changed: SystemTime,
    #[serde(serialize_with = "timestamp")]
    last_updated: SystemTime,
    state: &'a str,
}

impl<'a> StateJson<'a> {
    #[must_use]
    pub fn new(state: &'a State) -> Self {
        Self {
            attributes: state.attributes(),
            entity_id: state.entity_id().as_str(),
            last_changed: state.last_changed(),
            last_updated: state.last_updated(),
            state: state.state(),
        }
    }
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "signature imposée par `serialize_with`"
)]
fn timestamp<S: Serializer>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&format_timestamp(*time))
}

/// Date au format ISO 8601 / RFC 3339 en UTC : `2026-10-14T17:01:59.780320Z`.
///
/// On le calcule nous-mêmes (quelques lignes) plutôt que d'ajouter une
/// bibliothèque de dates entière : moins de dépendances, moins de risques.
#[must_use]
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

    use serde_json::{Value, json};

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

    /// La vue écrite directement donne exactement le même texte que l'ancien
    /// passage par `serde_json::Value`, sur des états variés.
    #[test]
    fn the_direct_view_writes_the_same_text_as_the_value_tree() {
        use ha_core::{EntityId, EventBus, StateMachine};

        let states = StateMachine::new(EventBus::new(16));
        let at = |secs: u64, micros: u64| {
            UNIX_EPOCH + Duration::from_secs(secs) + Duration::from_micros(micros)
        };
        let cases = [
            (
                "light.salon",
                "on",
                json!({"brightness": 200, "friendly_name": "Salon"}),
                at(1_700_000_000, 123_456),
                at(1_700_000_005, 1),
            ),
            ("sensor.vide", "unknown", json!({}), at(0, 0), at(0, 0)),
            (
                "sensor.temperature",
                "21.5",
                json!({
                    "unit_of_measurement": "°C",
                    "zéro": 0.0,
                    "pi": std::f64::consts::PI,
                    "petit": 1e-7,
                    "grand": 1.5e300,
                    "négatif": -42,
                    "énorme": u64::MAX,
                }),
                at(951_782_400, 999_999),
                at(1_798_761_599, 0),
            ),
            (
                "cover.volet_chambre",
                "ouvert \"à moitié\"\n\ttab \\ \u{1}",
                json!({
                    "liste": [1, "deux", null, true, {"z": 1, "a": [2.5, []]}],
                    "objet": {"b": {"c": {"d": "profond"}}, "a": null},
                    "emoji": "🏠 maison",
                    "Majuscule": false,
                    "": "clé vide",
                    "\u{7f}contrôle": "\u{0}</script>",
                }),
                at(1_234_567_890, 500_000),
                at(1_234_567_891, 0),
            ),
        ];
        for (id, value, attributes, changed, updated) in cases {
            let id = EntityId::new(id).unwrap();
            let Value::Object(attributes) = attributes else {
                unreachable!()
            };
            states
                .restore(id.clone(), value, attributes, changed, updated)
                .unwrap();
            let state = states.get(&id).unwrap();
            let before = state_to_json(&state).to_string();
            let after = serde_json::to_string(&StateJson::new(&state)).unwrap();
            assert_eq!(after, before, "{id}");
        }
        // Une liste d'états aussi, telle que la renvoient REST et WebSocket.
        let all = states.all();
        let before = Value::Array(all.iter().map(|s| state_to_json(s)).collect()).to_string();
        let views: Vec<StateJson<'_>> = all.iter().map(|s| StateJson::new(s)).collect();
        assert_eq!(serde_json::to_string(&views).unwrap(), before);
    }
}
