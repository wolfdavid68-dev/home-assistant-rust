//! Vérification de ce qui arrive avec un appel de service : textes, dates,
//! durées. Rien ici ne touche à la base : ce sont des fonctions simples, faciles
//! à tester.
//!
//! Sécurité : tout vient de l'extérieur (tableau de bord, API, automatisation).
//! Un texte trop long, avec des caractères de contrôle, ou une date absurde est
//! refusé avec une raison claire, jamais « corrigé » en silence.

use ha_core::Attributes;
use jiff::civil::{Date, DateTime};
use jiff::{Span, ToSpan};
use serde_json::Value;

/// Longueur maximale d'un élément de liste ou d'un titre de rendez-vous.
pub const MAX_SUMMARY: usize = 200;
/// Longueur maximale d'une description de rendez-vous.
pub const MAX_DESCRIPTION: usize = 1000;
/// Un rendez-vous dure au plus un an.
const MAX_EVENT_DAYS: i64 = 366;

/// Pourquoi un appel est refusé.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CheckError {
    #[error("« {0} » manquant")]
    Missing(&'static str),
    #[error("« {0} » doit être un texte de 1 à {1} caractères, sans caractère de contrôle")]
    Text(&'static str, usize),
    #[error("« {0} » : date invalide (attendu 2026-10-15)")]
    Date(&'static str),
    #[error("« {0} » : date et heure invalides (attendu 2026-10-15 18:30)")]
    DateTime(&'static str),
    #[error("la fin doit être après le début, et un rendez-vous dure au plus un an")]
    Duration,
    #[error("dates entre 2000 et 2100 seulement")]
    OutOfRange,
    #[error("« status » : needs_action ou completed")]
    Status,
}

/// Un texte obligatoire (élément de liste, titre) : débarrassé des espaces
/// autour, de 1 à `max` caractères, sans caractère de contrôle.
pub fn required_text(
    data: &Attributes,
    key: &'static str,
    max: usize,
) -> Result<String, CheckError> {
    match data.get(key) {
        None | Some(Value::Null) => Err(CheckError::Missing(key)),
        Some(value) => text(value, key, max),
    }
}

/// Un texte facultatif.
pub fn optional_text(
    data: &Attributes,
    key: &'static str,
    max: usize,
) -> Result<Option<String>, CheckError> {
    match data.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => text(value, key, max).map(Some),
    }
}

fn text(value: &Value, key: &'static str, max: usize) -> Result<String, CheckError> {
    let text = value.as_str().map(str::trim).unwrap_or_default();
    let count = text.chars().count();
    if count == 0 || count > max || text.chars().any(char::is_control) {
        return Err(CheckError::Text(key, max));
    }
    Ok(text.to_owned())
}

/// Une description : les retours à la ligne sont permis, pas les autres
/// caractères de contrôle. Vide si absente.
pub fn description(data: &Attributes) -> Result<String, CheckError> {
    let Some(value) = data.get("description").filter(|v| !v.is_null()) else {
        return Ok(String::new());
    };
    let text = value
        .as_str()
        .map(str::trim)
        .ok_or(CheckError::Text("description", MAX_DESCRIPTION))?;
    let bad = text.chars().any(|c| c.is_control() && c != '\n');
    if text.chars().count() > MAX_DESCRIPTION || bad {
        return Err(CheckError::Text("description", MAX_DESCRIPTION));
    }
    Ok(text.to_owned())
}

/// Une date seule : « 2026-10-15 ».
pub fn date(value: &Value, key: &'static str) -> Result<Date, CheckError> {
    let date: Date = value
        .as_str()
        .and_then(|text| text.trim().parse().ok())
        .ok_or(CheckError::Date(key))?;
    in_range(date)?;
    Ok(date)
}

/// Une date et une heure : « 2026-10-15 18:30 », « 2026-10-15T18:30:00 ».
pub fn date_time(value: &Value, key: &'static str) -> Result<DateTime, CheckError> {
    let moment: DateTime = value
        .as_str()
        .and_then(|text| text.trim().replacen(' ', "T", 1).parse().ok())
        .ok_or(CheckError::DateTime(key))?;
    in_range(moment.date())?;
    // À la seconde près : pas de fractions de seconde dans l'agenda.
    Ok(moment.with().subsec_nanosecond(0).build().unwrap_or(moment))
}

fn in_range(date: Date) -> Result<(), CheckError> {
    if (2000..=2100).contains(&date.year()) {
        Ok(())
    } else {
        Err(CheckError::OutOfRange)
    }
}

/// Le début et la fin d'un rendez-vous, comme Home Assistant :
///
/// - `start_date_time` (+ `end_date_time`, sinon une heure plus tard) ;
/// - ou `start_date` (+ `end_date`, **exclue** comme dans Home Assistant :
///   un jour entier le 15 va du 15 au 16). C'est alors « toute la journée ».
pub fn event_period(data: &Attributes) -> Result<(DateTime, DateTime, bool), CheckError> {
    let (start, end, all_day) = if let Some(start) = data.get("start_date_time") {
        let start = date_time(start, "start_date_time")?;
        let end = match data.get("end_date_time") {
            Some(end) => date_time(end, "end_date_time")?,
            None => start
                .checked_add(1.hour())
                .map_err(|_| CheckError::OutOfRange)?,
        };
        (start, end, false)
    } else if let Some(start) = data.get("start_date") {
        let start = date(start, "start_date")?;
        let end = match data.get("end_date") {
            Some(end) => date(end, "end_date")?,
            None => start.tomorrow().map_err(|_| CheckError::OutOfRange)?,
        };
        (
            start.to_datetime(jiff::civil::Time::midnight()),
            end.to_datetime(jiff::civil::Time::midnight()),
            true,
        )
    } else {
        return Err(CheckError::Missing("start_date_time"));
    };
    let longest = start
        .checked_add(Span::new().days(MAX_EVENT_DAYS))
        .map_err(|_| CheckError::OutOfRange)?;
    if end <= start || end > longest {
        return Err(CheckError::Duration);
    }
    Ok((start, end, all_day))
}

/// « 2026-10-15T18:30:00 » : le format rangé dans la base. Comparer deux
/// textes de ce format revient à comparer les dates.
#[must_use]
pub fn stored(moment: DateTime) -> String {
    moment.strftime("%Y-%m-%dT%H:%M:%S").to_string()
}

/// L'état voulu d'un élément de liste.
pub fn status(data: &Attributes) -> Result<Option<bool>, CheckError> {
    match data.get("status") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => match value.as_str() {
            Some("completed") => Ok(Some(true)),
            Some("needs_action") => Ok(Some(false)),
            _ => Err(CheckError::Status),
        },
    }
}

/// Ce qu'un appel demande pour la date d'échéance d'un élément.
///
/// Concept Rust : trois cas nommés plutôt qu'un `Option<Option<String>>`,
/// où `None` et `Some(None)` seraient faciles à confondre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Due {
    /// `due_date` absent : on n'y touche pas.
    Keep,
    /// `due_date: null` : on l'efface.
    Clear,
    /// `due_date: "2026-10-20"`.
    Set(String),
}

impl Due {
    /// La même chose, pour la base.
    #[must_use]
    pub fn as_change(&self) -> ha_storage::DueChange<'_> {
        match self {
            Self::Keep => ha_storage::DueChange::Keep,
            Self::Clear => ha_storage::DueChange::Clear,
            Self::Set(date) => ha_storage::DueChange::Set(date),
        }
    }

    /// La date à donner à un nouvel élément.
    #[must_use]
    pub fn date(&self) -> Option<&str> {
        match self {
            Self::Set(date) => Some(date),
            Self::Keep | Self::Clear => None,
        }
    }
}

/// La date d'échéance demandée.
pub fn due(data: &Attributes) -> Result<Due, CheckError> {
    match data.get("due_date") {
        None => Ok(Due::Keep),
        Some(Value::Null) => Ok(Due::Clear),
        Some(value) => Ok(Due::Set(date(value, "due_date")?.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn data(value: Value) -> Attributes {
        let Value::Object(map) = value else {
            panic!("un objet est attendu")
        };
        map
    }

    #[test]
    fn texts() {
        let ok = data(json!({"item": "  Lait demi-écrémé  "}));
        assert_eq!(
            required_text(&ok, "item", MAX_SUMMARY).unwrap(),
            "Lait demi-écrémé"
        );
        for bad in [
            json!({}),
            json!({"item": ""}),
            json!({"item": "   "}),
            json!({"item": 12}),
            json!({"item": "ligne\nnouvelle"}),
            json!({"item": "x".repeat(MAX_SUMMARY + 1)}),
        ] {
            assert!(
                required_text(&data(bad.clone()), "item", MAX_SUMMARY).is_err(),
                "{bad}"
            );
        }
        assert_eq!(
            description(&data(json!({"description": "Ligne 1\nLigne 2"}))).unwrap(),
            "Ligne 1\nLigne 2"
        );
        assert!(description(&data(json!({"description": "bip\u{7}"}))).is_err());
    }

    #[test]
    fn periods() {
        let (start, end, all_day) = event_period(&data(json!({
            "start_date_time": "2026-10-15 18:30", "end_date_time": "2026-10-15T19:15:00"
        })))
        .unwrap();
        assert_eq!(stored(start), "2026-10-15T18:30:00");
        assert_eq!(stored(end), "2026-10-15T19:15:00");
        assert!(!all_day);

        // Sans fin : une heure.
        let (_, end, _) =
            event_period(&data(json!({"start_date_time": "2026-10-15 23:30"}))).unwrap();
        assert_eq!(stored(end), "2026-10-16T00:30:00");

        // Toute la journée : la fin est exclue, comme dans Home Assistant.
        let (start, end, all_day) = event_period(&data(
            json!({"start_date": "2026-10-20", "end_date": "2026-10-27"}),
        ))
        .unwrap();
        assert!(all_day);
        assert_eq!(
            (stored(start), stored(end)),
            ("2026-10-20T00:00:00".into(), "2026-10-27T00:00:00".into())
        );

        for bad in [
            json!({}),
            json!({"start_date_time": "demain"}),
            json!({"start_date_time": "2026-10-15 19:00", "end_date_time": "2026-10-15 18:00"}),
            json!({"start_date_time": "2026-10-15 19:00", "end_date_time": "2026-10-15 19:00"}),
            json!({"start_date": "2026-01-01", "end_date": "2028-01-01"}),
            json!({"start_date": "1850-01-01"}),
            json!({"start_date": "2026-02-30"}),
        ] {
            assert!(event_period(&data(bad.clone())).is_err(), "{bad}");
        }
    }

    #[test]
    fn status_and_due() {
        assert_eq!(
            status(&data(json!({"status": "completed"}))),
            Ok(Some(true))
        );
        assert_eq!(status(&data(json!({}))), Ok(None));
        assert!(status(&data(json!({"status": "fini"}))).is_err());
        assert_eq!(due(&data(json!({}))), Ok(Due::Keep));
        assert_eq!(due(&data(json!({"due_date": null}))), Ok(Due::Clear));
        assert_eq!(
            due(&data(json!({"due_date": "2026-10-20"}))),
            Ok(Due::Set("2026-10-20".into()))
        );
        assert!(due(&data(json!({"due_date": "20/10/2026"}))).is_err());
    }
}
