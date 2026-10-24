//! Le soleil, comme l'intégration `sun` de Home Assistant : heures de lever,
//! de coucher, d'aube et de crépuscule, et hauteur du soleil, calculées pour
//! la position de ta maison. Aucun réseau : c'est de l'astronomie.
//!
//! - [`Location`] : la position (latitude, longitude), vérifiée ;
//! - [`sun_times`] : les heures d'une journée ;
//! - [`position`] : hauteur et direction du soleil à un instant ;
//! - [`season`] : la saison astronomique (printemps, été, automne, hiver) ;
//! - [`SunTracker`] : publie les entités `sun.sun` et `sensor.saison` chaque
//!   minute.
//!
//! Formules : l'« équation du lever du soleil » (algorithme de la NOAA,
//! simplifié). Précision : une minute environ, largement assez pour fermer
//! des volets.

use std::sync::Arc;
use std::time::Duration;

use ha_core::{Attributes, Background, CancellationToken, Core, EntityId};
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use tracing::{info, warn};

/// Jour julien du 1er janvier 2000 à midi (époque « J2000 »).
const J2000: f64 = 2_451_545.0;
/// Jour julien du 1er janvier 1970 à minuit (époque Unix).
const UNIX_EPOCH_JD: f64 = 2_440_587.5;
const SECONDS_PER_DAY: f64 = 86_400.0;
/// Inclinaison de l'axe de la Terre, en degrés.
const OBLIQUITY: f64 = 23.4397;

/// Hauteur du soleil au lever et au coucher : un peu sous l'horizon, à cause
/// de la réfraction de l'air et de la taille du disque solaire.
const SUNRISE_ALTITUDE: f64 = -0.833;
/// Aube et crépuscule « civils » : soleil à 6° sous l'horizon.
const CIVIL_TWILIGHT_ALTITUDE: f64 = -6.0;

/// Position de la maison.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Location {
    latitude: f64,
    longitude: f64,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum LocationError {
    #[error("latitude invalide : {0} (entre -90 et 90, par exemple 48.85)")]
    Latitude(f64),
    #[error("longitude invalide : {0} (entre -180 et 180, par exemple 2.35)")]
    Longitude(f64),
}

impl Location {
    /// Vérifie et crée une position. Latitude : nord positif ; longitude :
    /// est positif (Paris : 48.85, 2.35).
    pub fn new(latitude: f64, longitude: f64) -> Result<Self, LocationError> {
        if !(-90.0..=90.0).contains(&latitude) {
            return Err(LocationError::Latitude(latitude));
        }
        if !(-180.0..=180.0).contains(&longitude) {
            return Err(LocationError::Longitude(longitude));
        }
        Ok(Self {
            latitude,
            longitude,
        })
    }

    #[must_use]
    pub fn latitude(&self) -> f64 {
        self.latitude
    }

    #[must_use]
    pub fn longitude(&self) -> f64 {
        self.longitude
    }
}

/// Les heures du soleil pour une journée. `None` quand l'événement n'a pas
/// lieu ce jour-là (jour ou nuit polaire).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SunTimes {
    pub dawn: Option<Timestamp>,
    pub sunrise: Option<Timestamp>,
    pub noon: Timestamp,
    pub sunset: Option<Timestamp>,
    pub dusk: Option<Timestamp>,
}

/// Un événement du soleil, pour les automatisations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SunEvent {
    Sunrise,
    Sunset,
    Dawn,
    Dusk,
}

impl SunEvent {
    /// `sunrise`, `sunset`, `dawn`, `dusk` (les noms de Home Assistant).
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "sunrise" => Some(Self::Sunrise),
            "sunset" => Some(Self::Sunset),
            "dawn" => Some(Self::Dawn),
            "dusk" => Some(Self::Dusk),
            _ => None,
        }
    }

    /// L'heure de cet événement dans `times`.
    #[must_use]
    pub fn time_in(self, times: &SunTimes) -> Option<Timestamp> {
        match self {
            Self::Sunrise => times.sunrise,
            Self::Sunset => times.sunset,
            Self::Dawn => times.dawn,
            Self::Dusk => times.dusk,
        }
    }
}

fn sin_deg(degrees: f64) -> f64 {
    degrees.to_radians().sin()
}

fn cos_deg(degrees: f64) -> f64 {
    degrees.to_radians().cos()
}

/// Jour julien → instant.
fn timestamp_from_julian(julian: f64) -> Option<Timestamp> {
    let seconds = (julian - UNIX_EPOCH_JD) * SECONDS_PER_DAY;
    let duration = SignedDuration::try_from_secs_f64(seconds).ok()?;
    // À la seconde près : la précision du calcul est d'environ une minute.
    Timestamp::from_duration(duration)
        .ok()?
        .round(jiff::Unit::Second)
        .ok()
}

/// Instant → jour julien.
fn julian_from_timestamp(timestamp: Timestamp) -> f64 {
    let seconds = timestamp.as_duration().as_secs_f64();
    seconds / SECONDS_PER_DAY + UNIX_EPOCH_JD
}

/// Jour julien (à midi UTC) d'une date.
fn julian_noon(date: Date) -> Option<f64> {
    let midnight = date.to_zoned(TimeZone::UTC).ok()?.timestamp();
    Some(julian_from_timestamp(midnight) + 0.5)
}

/// Ce qui ne dépend que du jour : midi solaire, déclinaison du soleil.
struct SolarDay {
    transit: f64,
    declination: f64,
    /// Position du soleil sur l'écliptique, en degrés : 0° à l'équinoxe de
    /// mars, 90° au solstice de juin, 180° en septembre, 270° en décembre.
    ecliptic_longitude: f64,
}

fn solar_day(date: Date, location: Location) -> Option<SolarDay> {
    // Concept : ce sont les formules de l'« équation du lever du soleil »,
    // écrites telles quelles. Les noms suivent ceux des livres d'astronomie.
    let n = (julian_noon(date)? - J2000 + 0.0008).round();
    let mean_noon = n - location.longitude / 360.0;
    let anomaly = (357.5291 + 0.985_600_28 * mean_noon).rem_euclid(360.0);
    let center = 1.9148 * sin_deg(anomaly)
        + 0.0200 * sin_deg(2.0 * anomaly)
        + 0.0003 * sin_deg(3.0 * anomaly);
    let ecliptic_longitude = (anomaly + center + 180.0 + 102.9372).rem_euclid(360.0);
    let transit =
        J2000 + mean_noon + 0.0053 * sin_deg(anomaly) - 0.0069 * sin_deg(2.0 * ecliptic_longitude);
    let declination = (sin_deg(ecliptic_longitude) * sin_deg(OBLIQUITY))
        .asin()
        .to_degrees();
    Some(SolarDay {
        transit,
        declination,
        ecliptic_longitude,
    })
}

/// Demi-durée (en jours) pendant laquelle le soleil est au-dessus de
/// `altitude`. `None` s'il ne la franchit pas ce jour-là.
fn half_day(day: &SolarDay, location: Location, altitude: f64) -> Option<f64> {
    let cos_hour_angle = (sin_deg(altitude)
        - sin_deg(location.latitude) * sin_deg(day.declination))
        / (cos_deg(location.latitude) * cos_deg(day.declination));
    // Hors de [-1, 1] : le soleil reste toute la journée au-dessus (jour
    // polaire) ou au-dessous (nuit polaire) de cette hauteur.
    if !(-1.0..=1.0).contains(&cos_hour_angle) {
        return None;
    }
    Some(cos_hour_angle.acos().to_degrees() / 360.0)
}

/// Les heures du soleil le jour `date`, à la position `location`.
#[must_use]
pub fn sun_times(date: Date, location: Location) -> Option<SunTimes> {
    let day = solar_day(date, location)?;
    let around_noon = |altitude: f64| {
        half_day(&day, location, altitude).map(|half| {
            (
                timestamp_from_julian(day.transit - half),
                timestamp_from_julian(day.transit + half),
            )
        })
    };
    let (sunrise, sunset) = around_noon(SUNRISE_ALTITUDE).unwrap_or((None, None));
    let (dawn, dusk) = around_noon(CIVIL_TWILIGHT_ALTITUDE).unwrap_or((None, None));
    Some(SunTimes {
        dawn,
        sunrise,
        noon: timestamp_from_julian(day.transit)?,
        sunset,
        dusk,
    })
}

/// Hauteur au-dessus de l'horizon et direction (azimut, 0° = nord, 90° =
/// est) du soleil à l'instant `at`, en degrés.
#[must_use]
pub fn position(at: Timestamp, location: Location) -> Option<(f64, f64)> {
    let date = at.to_zoned(TimeZone::UTC).date();
    let day = solar_day(date, location)?;
    let hour_angle = (julian_from_timestamp(at) - day.transit) * 360.0;
    let (lat, dec) = (location.latitude, day.declination);
    let elevation = (sin_deg(lat) * sin_deg(dec)
        + cos_deg(lat) * cos_deg(dec) * cos_deg(hour_angle))
    .asin()
    .to_degrees();
    let azimuth = (sin_deg(hour_angle)
        .atan2(cos_deg(hour_angle) * sin_deg(lat) - dec.to_radians().tan() * cos_deg(lat))
        .to_degrees()
        + 180.0)
        .rem_euclid(360.0);
    Some((elevation, azimuth))
}

/// Une saison astronomique : elle change aux équinoxes et aux solstices
/// (vers le 20 mars, le 21 juin, le 22 septembre et le 21 décembre).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Season {
    Spring,
    Summer,
    Autumn,
    Winter,
}

impl Season {
    /// L'état de `sensor.saison` : `printemps`, `ete`, `automne`, `hiver`
    /// (sans accent, comme les noms d'ambiance du dessin).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Spring => "printemps",
            Self::Summer => "ete",
            Self::Autumn => "automne",
            Self::Winter => "hiver",
        }
    }
}

/// La saison le jour `date` à la position `location`. Dans l'hémisphère sud,
/// les saisons sont inversées : décembre y est l'été.
#[must_use]
pub fn season(date: Date, location: Location) -> Option<Season> {
    let mut longitude = solar_day(date, location)?.ecliptic_longitude;
    if location.latitude < 0.0 {
        longitude = (longitude + 180.0).rem_euclid(360.0);
    }
    Some(match longitude {
        l if l < 90.0 => Season::Spring,
        l if l < 180.0 => Season::Summer,
        l if l < 270.0 => Season::Autumn,
        _ => Season::Winter,
    })
}

/// L'état de `sensor.saison` à l'instant `now`.
#[must_use]
pub fn season_state(now: Timestamp, location: Location) -> Option<(&'static str, Attributes)> {
    let season = season(now.to_zoned(TimeZone::UTC).date(), location)?;
    let mut attributes = Attributes::new();
    attributes.insert("friendly_name".into(), json!("Saison"));
    Some((season.name(), attributes))
}

/// Le prochain `event` après `after` (aujourd'hui, demain…, jusqu'à un an :
/// au-delà, c'est la nuit ou le jour polaire).
#[must_use]
pub fn next_event(event: SunEvent, after: Timestamp, location: Location) -> Option<Timestamp> {
    let mut date = after.to_zoned(TimeZone::UTC).date().yesterday().ok()?;
    for _ in 0..370 {
        if let Some(time) = sun_times(date, location).and_then(|t| event.time_in(&t))
            && time > after
        {
            return Some(time);
        }
        date = date.tomorrow().ok()?;
    }
    None
}

/// Publie l'entité `sun.sun` (`above_horizon` / `below_horizon`), comme
/// Home Assistant, et la saison (`sensor.saison`), et les met à jour chaque
/// minute.
#[derive(Debug)]
pub struct SunTracker(Background);

impl SunTracker {
    #[must_use]
    pub fn start(location: Location, core: Arc<Core>) -> Self {
        Self(Background::spawn("le suivi du soleil", move |token| {
            run(location, core, token)
        }))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

async fn run(location: Location, core: Arc<Core>, token: CancellationToken) {
    let (Ok(entity_id), Ok(season_id)) = (
        "sun.sun".parse::<EntityId>(),
        "sensor.saison".parse::<EntityId>(),
    ) else {
        return;
    };
    info!("soleil : suivi démarré");
    let mut ticker = tokio::time::interval(Duration::from_mins(1));
    loop {
        tokio::select! {
            () = token.cancelled() => break,
            _ = ticker.tick() => {
                let now = Timestamp::now();
                let (state, attributes) = sun_state(now, location);
                if let Err(error) = core.states.set(entity_id.clone(), state, attributes) {
                    warn!("soleil : état refusé ({error})");
                }
                // Inchangée, la saison ne publie rien : `set` ne prévient que
                // d'un vrai changement.
                if let Some((state, attributes)) = season_state(now, location)
                    && let Err(error) = core.states.set(season_id.clone(), state, attributes)
                {
                    warn!("saison : état refusé ({error})");
                }
            }
        }
    }
}

/// L'état de `sun.sun` à l'instant `now`.
#[must_use]
pub fn sun_state(now: Timestamp, location: Location) -> (&'static str, Attributes) {
    let (elevation, azimuth) = position(now, location).unwrap_or((0.0, 0.0));
    let state = if elevation > SUNRISE_ALTITUDE {
        "above_horizon"
    } else {
        "below_horizon"
    };
    let iso = |event| next_event(event, now, location).map(|t| t.to_string());
    let attributes = json!({
        "friendly_name": "Soleil",
        "next_rising": iso(SunEvent::Sunrise),
        "next_setting": iso(SunEvent::Sunset),
        "next_dawn": iso(SunEvent::Dawn),
        "next_dusk": iso(SunEvent::Dusk),
        "elevation": (elevation * 10.0).round() / 10.0,
        "azimuth": (azimuth * 10.0).round() / 10.0,
    });
    let attributes = match attributes {
        serde_json::Value::Object(map) => map,
        _ => Attributes::new(),
    };
    (state, attributes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    const PARIS: (f64, f64) = (48.8566, 2.3522);

    fn paris() -> Location {
        Location::new(PARIS.0, PARIS.1).unwrap()
    }

    /// Vérifie qu'un instant est à `tolerance` minutes près de `expected`
    /// (heure UTC « HH:MM »).
    fn assert_near(actual: Option<Timestamp>, day: Date, expected: &str, tolerance: i64) {
        let actual = actual.unwrap();
        let expected: jiff::civil::Time = expected.parse().unwrap();
        let expected = day
            .to_datetime(expected)
            .to_zoned(TimeZone::UTC)
            .unwrap()
            .timestamp();
        let gap = actual.duration_since(expected).as_secs().abs();
        assert!(
            gap <= tolerance * 60,
            "{actual} au lieu de {expected} (écart {gap} s)"
        );
    }

    #[test]
    fn paris_summer_and_winter_solstices() {
        // Références (heures UTC) : éphémérides publiées pour Paris.
        // 21 juin 2024 : lever 05 h 47, coucher 21 h 58 (heure d'été).
        let summer = date(2024, 6, 21);
        let times = sun_times(summer, paris()).unwrap();
        assert_near(times.sunrise, summer, "03:47", 2);
        assert_near(times.sunset, summer, "19:58", 2);
        // 21 décembre 2024 : lever 08 h 41, coucher 16 h 56 (heure d'hiver).
        let winter = date(2024, 12, 21);
        let times = sun_times(winter, paris()).unwrap();
        assert_near(times.sunrise, winter, "07:41", 2);
        assert_near(times.sunset, winter, "15:56", 2);
        // L'aube précède le lever, le crépuscule suit le coucher.
        assert!(times.dawn.unwrap() < times.sunrise.unwrap());
        assert!(times.dusk.unwrap() > times.sunset.unwrap());
        assert!(times.sunrise.unwrap() < times.noon && times.noon < times.sunset.unwrap());
    }

    #[test]
    fn polar_day_and_night() {
        // Tromsø (Norvège) : soleil de minuit en juin, nuit polaire en décembre.
        let tromso = Location::new(69.65, 18.96).unwrap();
        let june = sun_times(date(2024, 6, 21), tromso).unwrap();
        assert_eq!((june.sunrise, june.sunset), (None, None));
        let december = sun_times(date(2024, 12, 21), tromso).unwrap();
        assert_eq!((december.sunrise, december.sunset), (None, None));
        // Le prochain coucher après le 21 juin existe quand même (fin juillet).
        let after: Timestamp = "2024-06-21T12:00:00Z".parse().unwrap();
        let next = next_event(SunEvent::Sunset, after, tromso).unwrap();
        assert!(next > after);
        assert!(next < "2024-08-15T00:00:00Z".parse().unwrap());
    }

    #[test]
    fn elevation_and_state() {
        // Midi solaire en été à Paris : soleil haut (~64,6°), au sud.
        let noon = sun_times(date(2024, 6, 21), paris()).unwrap().noon;
        let (elevation, azimuth) = position(noon, paris()).unwrap();
        assert!((elevation - 64.6).abs() < 1.0, "{elevation}");
        assert!((azimuth - 180.0).abs() < 2.0, "{azimuth}");
        assert_eq!(sun_state(noon, paris()).0, "above_horizon");
        // Minuit : sous l'horizon.
        let midnight: Timestamp = "2024-06-21T23:00:00Z".parse().unwrap();
        let (state, attributes) = sun_state(midnight, paris());
        assert_eq!(state, "below_horizon");
        assert!(
            attributes["next_rising"]
                .as_str()
                .unwrap()
                .starts_with("2024-06-22T03:4")
        );
    }

    #[test]
    fn locations_are_checked() {
        assert!(Location::new(48.85, 2.35).is_ok());
        assert_eq!(Location::new(91.0, 0.0), Err(LocationError::Latitude(91.0)));
        assert_eq!(
            Location::new(0.0, -181.0),
            Err(LocationError::Longitude(-181.0))
        );
        assert!(Location::new(f64::NAN, 0.0).is_err());
    }

    #[test]
    fn seasons_change_at_equinoxes_and_solstices() {
        // 2024 : équinoxe le 20 mars, solstice le 20 juin, équinoxe le
        // 22 septembre, solstice le 21 décembre (dates en temps universel).
        let at = |m, d| season(date(2024, m, d), paris()).unwrap();
        assert_eq!(at(1, 15), Season::Winter);
        assert_eq!(at(3, 19), Season::Winter);
        assert_eq!(at(3, 21), Season::Spring);
        assert_eq!(at(6, 19), Season::Spring);
        assert_eq!(at(6, 22), Season::Summer);
        assert_eq!(at(9, 21), Season::Summer);
        assert_eq!(at(9, 23), Season::Autumn);
        assert_eq!(at(10, 24), Season::Autumn);
        assert_eq!(at(12, 20), Season::Autumn);
        assert_eq!(at(12, 22), Season::Winter);
        // Hémisphère sud (Sydney) : saisons inversées.
        let sydney = Location::new(-33.87, 151.21).unwrap();
        assert_eq!(season(date(2024, 1, 15), sydney), Some(Season::Summer));
        assert_eq!(season(date(2024, 10, 24), sydney), Some(Season::Spring));
        // L'entité.
        let now: Timestamp = "2024-10-24T12:00:00Z".parse().unwrap();
        let (state, attributes) = season_state(now, paris()).unwrap();
        assert_eq!(state, "automne");
        assert_eq!(attributes["friendly_name"], "Saison");
        assert_eq!(Season::Summer.name(), "ete");
    }

    #[test]
    fn events() {
        assert_eq!(SunEvent::parse("sunset"), Some(SunEvent::Sunset));
        assert_eq!(SunEvent::parse("coucher"), None);
    }
}
