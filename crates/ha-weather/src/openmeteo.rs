//! L'API d'Open-Meteo : la requête, la lecture de la réponse, et la
//! traduction des codes météo en conditions Home Assistant.
//!
//! Documentation : <https://open-meteo.com/en/docs>. Pas de compte ni de clé ;
//! les données sont sous licence CC BY 4.0, d'où l'attribution affichée.

use serde::Deserialize;
use serde_json::{Value, json};

/// Ce qu'on demande à Open-Meteo.
const CURRENT: &str = "temperature_2m,relative_humidity_2m,apparent_temperature,is_day,\
    precipitation,weather_code,cloud_cover,pressure_msl,wind_speed_10m,wind_direction_10m,\
    wind_gusts_10m";
const DAILY: &str = "weather_code,temperature_2m_max,temperature_2m_min,precipitation_sum,\
    precipitation_probability_max";
/// Nombre de jours de prévisions.
pub const FORECAST_DAYS: usize = 5;

/// Le temps qu'il fait maintenant.
#[derive(Debug, Clone, PartialEq)]
pub struct Current {
    pub condition: &'static str,
    /// °C
    pub temperature: f64,
    pub apparent_temperature: Option<f64>,
    /// %
    pub humidity: f64,
    /// mm (sur le dernier quart d'heure)
    pub precipitation: f64,
    /// %
    pub cloud_cover: Option<f64>,
    /// hPa
    pub pressure: Option<f64>,
    /// km/h
    pub wind_speed: f64,
    pub wind_gust: Option<f64>,
    /// Degrés, d'où vient le vent (0 = nord).
    pub wind_bearing: Option<f64>,
}

/// Les prévisions d'un jour.
#[derive(Debug, Clone, PartialEq)]
pub struct Day {
    /// `2026-10-15`
    pub date: String,
    pub condition: &'static str,
    pub max: f64,
    pub min: f64,
    pub precipitation: Option<f64>,
    pub precipitation_probability: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub current: Current,
    pub days: Vec<Day>,
}

impl Day {
    /// Comme l'attribut `forecast` d'une entité `weather` de Home Assistant.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "datetime": self.date,
            "condition": self.condition,
            "temperature": self.max,
            "templow": self.min,
            "precipitation": self.precipitation,
            "precipitation_probability": self.precipitation_probability,
        })
    }
}

/// L'adresse de la requête. Latitude et longitude sont déjà arrondies.
#[must_use]
pub fn request_url(base: &url::Url, latitude: f64, longitude: f64) -> url::Url {
    let mut url = base.clone();
    url.query_pairs_mut()
        .append_pair("latitude", &format!("{latitude:.1}"))
        .append_pair("longitude", &format!("{longitude:.1}"))
        .append_pair("current", CURRENT)
        .append_pair("daily", DAILY)
        .append_pair("forecast_days", &FORECAST_DAYS.to_string())
        .append_pair("timezone", "auto")
        .append_pair("wind_speed_unit", "kmh");
    url
}

// Ce qu'Open-Meteo renvoie (seulement ce qui nous sert).

#[derive(Deserialize)]
struct Response {
    current: RawCurrent,
    daily: RawDaily,
}

#[derive(Deserialize)]
struct RawCurrent {
    temperature_2m: Option<f64>,
    relative_humidity_2m: Option<f64>,
    apparent_temperature: Option<f64>,
    is_day: Option<u8>,
    precipitation: Option<f64>,
    weather_code: Option<u8>,
    cloud_cover: Option<f64>,
    pressure_msl: Option<f64>,
    wind_speed_10m: Option<f64>,
    wind_direction_10m: Option<f64>,
    wind_gusts_10m: Option<f64>,
}

#[derive(Deserialize)]
struct RawDaily {
    time: Vec<String>,
    weather_code: Vec<Option<u8>>,
    temperature_2m_max: Vec<Option<f64>>,
    temperature_2m_min: Vec<Option<f64>>,
    #[serde(default)]
    precipitation_sum: Vec<Option<f64>>,
    #[serde(default)]
    precipitation_probability_max: Vec<Option<f64>>,
}

/// Erreur renvoyée par Open-Meteo : `{"error": true, "reason": "…"}`.
#[derive(Deserialize)]
struct ApiError {
    reason: String,
}

/// Lit une réponse d'Open-Meteo. Toutes les valeurs sont vérifiées : un
/// serveur en panne (ou malveillant) ne peut pas afficher -500 °C.
pub fn parse(body: &[u8]) -> Result<Report, String> {
    if let Ok(error) = serde_json::from_slice::<ApiError>(body) {
        let reason: String = error
            .reason
            .chars()
            .filter(|c| !c.is_control())
            .take(200)
            .collect();
        return Err(format!("Open-Meteo refuse : {reason}"));
    }
    let response: Response =
        serde_json::from_slice(body).map_err(|error| format!("réponse illisible : {error}"))?;
    let c = response.current;
    let need = |value: Option<f64>, name: &str, range: std::ops::RangeInclusive<f64>| {
        value
            .filter(|v| range.contains(v))
            .ok_or_else(|| format!("réponse invalide : {name}"))
    };
    let optional = |value: Option<f64>, range: std::ops::RangeInclusive<f64>| {
        value.filter(|v| range.contains(v))
    };
    let code = c.weather_code.ok_or("réponse invalide : weather_code")?;
    let current = Current {
        condition: condition(code, c.is_day != Some(0)),
        temperature: need(c.temperature_2m, "temperature_2m", TEMPERATURE)?,
        apparent_temperature: optional(c.apparent_temperature, TEMPERATURE),
        humidity: need(c.relative_humidity_2m, "relative_humidity_2m", 0.0..=100.0)?,
        precipitation: optional(c.precipitation, 0.0..=500.0).unwrap_or(0.0),
        cloud_cover: optional(c.cloud_cover, 0.0..=100.0),
        pressure: optional(c.pressure_msl, 800.0..=1_100.0),
        wind_speed: need(c.wind_speed_10m, "wind_speed_10m", 0.0..=500.0)?,
        wind_gust: optional(c.wind_gusts_10m, 0.0..=500.0),
        wind_bearing: optional(c.wind_direction_10m, 0.0..=360.0),
    };

    let d = response.daily;
    let count = d.time.len().min(FORECAST_DAYS);
    let mut days = Vec::with_capacity(count);
    for i in 0..count {
        let get = |list: &[Option<f64>]| list.get(i).copied().flatten();
        let date = d.time.get(i).cloned().unwrap_or_default();
        if !is_date(&date) {
            return Err("réponse invalide : date".into());
        }
        let code = d
            .weather_code
            .get(i)
            .copied()
            .flatten()
            .ok_or("réponse invalide : prévisions")?;
        days.push(Day {
            date,
            condition: condition(code, true),
            max: need(
                get(&d.temperature_2m_max),
                "temperature_2m_max",
                TEMPERATURE,
            )?,
            min: need(
                get(&d.temperature_2m_min),
                "temperature_2m_min",
                TEMPERATURE,
            )?,
            precipitation: optional(get(&d.precipitation_sum), 0.0..=2_000.0),
            precipitation_probability: optional(get(&d.precipitation_probability_max), 0.0..=100.0),
        });
    }
    Ok(Report { current, days })
}

/// Températures plausibles sur Terre.
const TEMPERATURE: std::ops::RangeInclusive<f64> = -90.0..=60.0;

/// `2026-10-15`, et rien d'autre (la date finit dans l'interface).
fn is_date(value: &str) -> bool {
    value.len() == 10
        && value.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
}

/// Code météo de l'OMM (utilisé par Open-Meteo) → condition de Home Assistant.
#[must_use]
pub fn condition(code: u8, is_day: bool) -> &'static str {
    match code {
        0 | 1 if is_day => "sunny",
        0 | 1 => "clear-night",
        2 => "partlycloudy",
        3 => "cloudy",
        45 | 48 => "fog",
        51 | 53 | 55 | 61 | 63 | 80 | 81 => "rainy",
        65 | 82 => "pouring",
        56 | 57 | 66 | 67 => "snowy-rainy",
        71 | 73 | 75 | 77 | 85 | 86 => "snowy",
        95 | 96 | 99 => "lightning-rainy",
        _ => "exceptional",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "latitude": 48.86, "longitude": 2.34, "timezone": "Europe/Paris",
      "current_units": {"temperature_2m": "°C"},
      "current": {"time": "2026-10-15T01:45", "interval": 900, "temperature_2m": 11.2,
        "relative_humidity_2m": 87, "apparent_temperature": 9.8, "is_day": 0,
        "precipitation": 0.0, "weather_code": 3, "cloud_cover": 100, "pressure_msl": 1018.4,
        "wind_speed_10m": 9.4, "wind_direction_10m": 225, "wind_gusts_10m": 20.5},
      "daily": {"time": ["2026-10-15", "2026-10-16", "2026-10-17", "2026-10-18", "2026-10-19", "2026-10-20"],
        "weather_code": [61, 0, 2, 95, 71, 3],
        "temperature_2m_max": [16.1, 18.0, 17.2, 14.9, 3.0, 12.0],
        "temperature_2m_min": [9.8, 8.1, 10.0, 9.0, -2.5, 6.0],
        "precipitation_sum": [3.2, 0.0, 0.1, 12.5, 4.0, 0.0],
        "precipitation_probability_max": [80, 5, 20, 90, null, 10]}
    }"#;

    #[test]
    fn a_real_looking_response_is_understood() {
        let report = parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(report.current.condition, "cloudy");
        assert!((report.current.temperature - 11.2).abs() < 1e-9);
        assert_eq!(report.current.wind_bearing, Some(225.0));
        assert_eq!(report.days.len(), FORECAST_DAYS, "6 jours reçus, 5 gardés");
        assert_eq!(report.days[0].condition, "rainy");
        assert_eq!(
            report.days[1].condition, "sunny",
            "en journée, pas clear-night"
        );
        assert_eq!(report.days[3].condition, "lightning-rainy");
        assert_eq!(report.days[4].precipitation_probability, None);
        assert_eq!(report.days[4].to_json()["templow"], -2.5);
    }

    #[test]
    fn errors_and_absurd_values_are_refused() {
        let error = parse(br#"{"error": true, "reason": "Daily API request limit exceeded."}"#)
            .unwrap_err();
        assert!(error.contains("limit exceeded"), "{error}");
        for (from, to) in [
            ("\"temperature_2m\": 11.2", "\"temperature_2m\": -500"),
            ("\"temperature_2m\": 11.2", "\"temperature_2m\": null"),
            (
                "\"relative_humidity_2m\": 87",
                "\"relative_humidity_2m\": 150",
            ),
            ("\"weather_code\": 3,", "\"weather_code\": 300,"),
            ("\"2026-10-16\"", "\"<img src=x>\""),
            ("[16.1, 18.0", "[160.1, 18.0"),
        ] {
            assert!(parse(SAMPLE.replace(from, to).as_bytes()).is_err(), "{to}");
        }
        assert!(parse(b"pas du json").is_err());
        // Valeur facultative absurde : simplement ignorée.
        let odd = parse(SAMPLE.replace("1018.4", "5").as_bytes()).unwrap();
        assert_eq!(odd.current.pressure, None);
    }

    #[test]
    fn weather_codes() {
        assert_eq!(condition(0, true), "sunny");
        assert_eq!(condition(0, false), "clear-night");
        assert_eq!(condition(45, true), "fog");
        assert_eq!(condition(65, true), "pouring");
        assert_eq!(condition(67, true), "snowy-rainy");
        assert_eq!(condition(86, false), "snowy");
        assert_eq!(condition(42, true), "exceptional");
    }

    #[test]
    fn the_request_only_carries_rounded_coordinates() {
        let base = url::Url::parse("https://api.open-meteo.com/v1/forecast").unwrap();
        let url = request_url(&base, 48.9, 2.3);
        let query = url.query().unwrap();
        assert!(
            query.starts_with("latitude=48.9&longitude=2.3&current="),
            "{query}"
        );
        assert!(query.contains("forecast_days=5"));
    }
}
