//! Jours ouvrés et jours fériés (étape 56), comme l'intégration « Workday »
//! de Home Assistant, pour la France : calculés sur place, sans réseau.
//!
//! Trois entités :
//!
//! | Entité | État |
//! |--------|------|
//! | `binary_sensor.jour_ouvre` | `on` un jour de travail (lundi à vendredi par défaut), `off` un week-end ou un jour férié |
//! | `binary_sensor.demain_jour_ouvre` | pareil pour demain : pour régler le réveil la veille |
//! | `sensor.prochain_jour_ferie` | la date du prochain jour férié ; son nom et dans combien de jours en attributs |
//!
//! ```yaml
//! - id: reveil
//!   alias: Volets du matin, seulement les jours ouvrés
//!   triggers:
//!     - trigger: time
//!       at: "07:00:00"
//!   conditions:
//!     - condition: state
//!       entity_id: binary_sensor.jour_ouvre
//!       state: "on"
//!   actions:
//!     - action: cover.open_cover
//!       target: {entity_id: cover.chambre}
//! ```
//!
//! Les jours fériés : les onze du Code du travail, plus le Vendredi saint et
//! la Saint-Étienne (26 décembre) en **Alsace-Moselle** (Bas-Rhin, Haut-Rhin,
//! Moselle). Pâques est calculée (algorithme de Meeus, Jones et Butcher).

use std::sync::Arc;
use std::time::Duration;

use ha_core::{Attributes, Background, CancellationToken, Core, EntityId};
use jiff::Zoned;
use jiff::civil::{Date, Weekday, date};
use serde_json::json;
use tracing::{info, warn};

/// Quels jours fériés.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Regime {
    France,
    /// Plus le Vendredi saint et le 26 décembre.
    AlsaceMoselle,
}

impl Regime {
    /// `HA_JOURS_FERIES` (`france` ou `alsace-moselle`) ; sans réglage,
    /// d'après le département de la vigilance météo (57, 67, 68 : Alsace-
    /// Moselle).
    pub fn from_settings(setting: Option<&str>, department: Option<&str>) -> Result<Self, String> {
        match setting.map(str::trim) {
            Some("france") => Ok(Self::France),
            Some("alsace-moselle" | "alsace" | "moselle") => Ok(Self::AlsaceMoselle),
            Some(other) => Err(format!(
                "HA_JOURS_FERIES « {other} » inconnu : france ou alsace-moselle"
            )),
            None => Ok(match department.map(str::trim) {
                Some("57" | "67" | "68") => Self::AlsaceMoselle,
                _ => Self::France,
            }),
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::France => "France",
            Self::AlsaceMoselle => "Alsace-Moselle",
        }
    }
}

/// Les jours de travail de la semaine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Week([bool; 7]);

impl Default for Week {
    /// Du lundi au vendredi.
    fn default() -> Self {
        Self([true, true, true, true, true, false, false])
    }
}

impl Week {
    /// `HA_JOURS_OUVRES` : « lun,mar,mer,jeu,ven » (par défaut).
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut days = [false; 7];
        for word in text.split(',').map(str::trim).filter(|w| !w.is_empty()) {
            let index = match word {
                "lun" | "lundi" => 0,
                "mar" | "mardi" => 1,
                "mer" | "mercredi" => 2,
                "jeu" | "jeudi" => 3,
                "ven" | "vendredi" => 4,
                "sam" | "samedi" => 5,
                "dim" | "dimanche" => 6,
                other => {
                    return Err(format!(
                        "HA_JOURS_OUVRES : jour « {other} » inconnu (lun, mar, mer, jeu, ven, sam, dim)"
                    ));
                }
            };
            days[index] = true;
        }
        if days.iter().all(|day| !day) {
            return Err("HA_JOURS_OUVRES : aucun jour".into());
        }
        Ok(Self(days))
    }

    fn contains(self, weekday: Weekday) -> bool {
        self.0[usize::from(weekday.to_monday_zero_offset().unsigned_abs())]
    }

    fn names(self) -> Vec<&'static str> {
        [
            "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche",
        ]
        .into_iter()
        .zip(self.0)
        .filter_map(|(name, on)| on.then_some(name))
        .collect()
    }
}

/// Le dimanche de Pâques (calendrier grégorien).
#[must_use]
#[allow(
    clippy::many_single_char_names,
    reason = "les lettres de l'algorithme publié, pour pouvoir le comparer"
)]
pub fn easter(year: i16) -> Option<Date> {
    let y = i32::from(year);
    let a = y % 19;
    let b = y / 100;
    let c = y % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = i8::try_from((h + l - 7 * m + 114) / 31).ok()?;
    let day = i8::try_from((h + l - 7 * m + 114) % 31 + 1).ok()?;
    Date::new(year, month, day).ok()
}

/// Les jours fériés d'une année, dans l'ordre.
#[must_use]
pub fn holidays(year: i16, regime: Regime) -> Vec<(Date, &'static str)> {
    let mut days = vec![
        (date(year, 1, 1), "Jour de l'an"),
        (date(year, 5, 1), "Fête du Travail"),
        (date(year, 5, 8), "Victoire 1945"),
        (date(year, 7, 14), "Fête nationale"),
        (date(year, 8, 15), "Assomption"),
        (date(year, 11, 1), "Toussaint"),
        (date(year, 11, 11), "Armistice 1918"),
        (date(year, 12, 25), "Noël"),
    ];
    if let Some(easter) = easter(year) {
        let shifted = |days: i64| easter.checked_add(jiff::Span::new().days(days)).ok();
        for (offset, name) in [
            (1, "Lundi de Pâques"),
            (39, "Ascension"),
            (50, "Lundi de Pentecôte"),
        ] {
            if let Some(day) = shifted(offset) {
                days.push((day, name));
            }
        }
        if regime == Regime::AlsaceMoselle
            && let Some(day) = shifted(-2)
        {
            days.push((day, "Vendredi saint"));
        }
    }
    if regime == Regime::AlsaceMoselle {
        days.push((date(year, 12, 26), "Saint-Étienne"));
    }
    days.sort_unstable();
    days
}

/// Le nom du jour férié, si `day` en est un.
#[must_use]
pub fn holiday(day: Date, regime: Regime) -> Option<&'static str> {
    holidays(day.year(), regime)
        .into_iter()
        .find_map(|(date, name)| (date == day).then_some(name))
}

/// Un jour de travail : dans la semaine choisie, et pas férié.
#[must_use]
pub fn is_workday(day: Date, regime: Regime, week: Week) -> bool {
    week.contains(day.weekday()) && holiday(day, regime).is_none()
}

/// Le prochain jour férié à partir de `day` (compris).
#[must_use]
pub fn next_holiday(day: Date, regime: Regime) -> Option<(Date, &'static str)> {
    [day.year(), day.year().checked_add(1)?]
        .into_iter()
        .flat_map(|year| holidays(year, regime))
        .find(|(date, _)| *date >= day)
}

/// « 14/05/2026 ».
fn french(day: Date) -> String {
    day.strftime("%d/%m/%Y").to_string()
}

fn attributes(value: serde_json::Value) -> Attributes {
    match value {
        serde_json::Value::Object(map) => map,
        _ => Attributes::new(),
    }
}

/// Les trois entités pour le jour `today`.
#[must_use]
pub fn states(today: Date, regime: Regime, week: Week) -> Vec<(&'static str, String, Attributes)> {
    let tomorrow = today.tomorrow().unwrap_or(today);
    let on_off = |on: bool| if on { "on" } else { "off" }.to_owned();
    let workdays = week.names();
    let mut out = vec![
        (
            "binary_sensor.jour_ouvre",
            on_off(is_workday(today, regime, week)),
            attributes(json!({
                "friendly_name": "Jour ouvré",
                "icon": "mdi:briefcase",
                "jour_ferie": holiday(today, regime),
                "jours_ouvres": workdays,
                "regime": regime.label(),
            })),
        ),
        (
            "binary_sensor.demain_jour_ouvre",
            on_off(is_workday(tomorrow, regime, week)),
            attributes(json!({
                "friendly_name": "Demain, jour ouvré",
                "icon": "mdi:briefcase-clock",
                "jour_ferie": holiday(tomorrow, regime),
                "jours_ouvres": workdays,
                "regime": regime.label(),
            })),
        ),
    ];
    if let Some((day, name)) = next_holiday(today, regime) {
        let days = (day - today).get_days();
        out.push((
            "sensor.prochain_jour_ferie",
            day.to_string(),
            attributes(json!({
                "friendly_name": "Prochain jour férié",
                "device_class": "date",
                "icon": "mdi:calendar-star",
                "nom": name,
                "date": french(day),
                "dans_jours": days,
                "regime": regime.label(),
            })),
        ));
    }
    out
}

/// Le service : met les entités à jour chaque minute (le jour change à
/// minuit, heure locale).
#[derive(Debug)]
pub struct Workday(Background);

impl Workday {
    #[must_use]
    pub fn start(regime: Regime, week: Week, core: Arc<Core>) -> Self {
        Self(Background::spawn("les jours ouvrés", move |token| {
            run(regime, week, core, token)
        }))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

async fn run(regime: Regime, week: Week, core: Arc<Core>, token: CancellationToken) {
    info!("jours ouvrés : jours fériés de {}", regime.label());
    let mut ticker = tokio::time::interval(Duration::from_mins(1));
    let mut published: Option<Date> = None;
    loop {
        tokio::select! {
            () = token.cancelled() => break,
            _ = ticker.tick() => {
                let today = Zoned::now().date();
                if published == Some(today) {
                    continue;
                }
                for (id, state, attributes) in states(today, regime, week) {
                    let Ok(entity_id) = id.parse::<EntityId>() else { continue };
                    if let Err(error) = core.states.set(entity_id, state, attributes) {
                        warn!("jours ouvrés : état refusé ({error})");
                    }
                }
                published = Some(today);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn easter_is_computed() {
        for (year, month, day) in [
            (2000, 4, 23),
            (2019, 4, 21),
            (2024, 3, 31),
            (2025, 4, 20),
            (2026, 4, 5),
            (2027, 3, 28),
            (2038, 4, 25),
        ] {
            assert_eq!(easter(year), Some(date(year, month, day)), "{year}");
        }
    }

    #[test]
    fn french_public_holidays() {
        let names = |year, regime| -> Vec<String> {
            holidays(year, regime)
                .into_iter()
                .map(|(day, name)| format!("{} {name}", day.strftime("%d/%m")))
                .collect()
        };
        assert_eq!(
            names(2026, Regime::France),
            [
                "01/01 Jour de l'an",
                "06/04 Lundi de Pâques",
                "01/05 Fête du Travail",
                "08/05 Victoire 1945",
                "14/05 Ascension",
                "25/05 Lundi de Pentecôte",
                "14/07 Fête nationale",
                "15/08 Assomption",
                "01/11 Toussaint",
                "11/11 Armistice 1918",
                "25/12 Noël",
            ]
        );
        let alsace = names(2026, Regime::AlsaceMoselle);
        assert_eq!(alsace.len(), 13);
        assert!(alsace.contains(&"03/04 Vendredi saint".to_owned()));
        assert!(alsace.contains(&"26/12 Saint-Étienne".to_owned()));
    }

    #[test]
    fn workdays() {
        let week = Week::default();
        // Jeudi de l'Ascension 2026 : férié.
        assert!(!is_workday(date(2026, 5, 14), Regime::France, week));
        assert!(is_workday(date(2026, 5, 15), Regime::France, week));
        // Samedi.
        assert!(!is_workday(date(2026, 5, 16), Regime::France, week));
        // Vendredi saint : travaillé en France, férié en Alsace.
        assert!(is_workday(date(2026, 4, 3), Regime::France, week));
        assert!(!is_workday(date(2026, 4, 3), Regime::AlsaceMoselle, week));
        // Le samedi travaillé, si on le dit.
        let six = Week::parse("lun, mar,mer,jeu,ven,sam").unwrap();
        assert!(is_workday(date(2026, 5, 16), Regime::France, six));
        assert!(Week::parse("lun,funday").is_err());
        assert!(Week::parse(" , ").is_err());
    }

    #[test]
    fn the_next_holiday_and_the_entities() {
        assert_eq!(
            next_holiday(date(2026, 12, 27), Regime::France),
            Some((date(2027, 1, 1), "Jour de l'an"))
        );
        assert_eq!(
            next_holiday(date(2026, 12, 25), Regime::France),
            Some((date(2026, 12, 25), "Noël"))
        );
        // La veille de l'Ascension 2026 (un mercredi).
        let states = states(date(2026, 5, 13), Regime::France, Week::default());
        let get = |id: &str| states.iter().find(|(entity, _, _)| *entity == id).unwrap();
        assert_eq!(get("binary_sensor.jour_ouvre").1, "on");
        let tomorrow = get("binary_sensor.demain_jour_ouvre");
        assert_eq!(tomorrow.1, "off");
        assert_eq!(tomorrow.2["jour_ferie"], "Ascension");
        let next = get("sensor.prochain_jour_ferie");
        assert_eq!(next.1, "2026-05-14");
        assert_eq!(next.2["nom"], "Ascension");
        assert_eq!(next.2["dans_jours"], 1);
        assert_eq!(next.2["date"], "14/05/2026");
    }

    #[test]
    fn the_regime_follows_the_settings() {
        assert_eq!(
            Regime::from_settings(None, Some("68")),
            Ok(Regime::AlsaceMoselle)
        );
        assert_eq!(Regime::from_settings(None, Some("75")), Ok(Regime::France));
        assert_eq!(Regime::from_settings(None, None), Ok(Regime::France));
        assert_eq!(
            Regime::from_settings(Some("france"), Some("67")),
            Ok(Regime::France)
        );
        assert!(Regime::from_settings(Some("belgique"), None).is_err());
    }
}
