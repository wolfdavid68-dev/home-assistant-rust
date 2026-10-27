//! Le fichier `inputs.yaml` : lecture et vérification.
//!
//! ```yaml
//! input_boolean:
//!   mode_vacances:
//!     name: Mode vacances
//! input_number:
//!   consigne_chambre: {name: Consigne chambre, min: 15, max: 23, step: 0.5, unit_of_measurement: "°C"}
//! input_select:
//!   mode_maison: {name: Mode de la maison, options: [Normal, Invités, Nuit]}
//! input_button:
//!   test_sirene: {name: Tester la sirène}
//! timer:
//!   lessive: {name: Lessive, duration: "01:30:00"}
//! counter:
//!   lessives_du_mois: {name: Lessives du mois, step: 1, minimum: 0}
//! input_text:
//!   message_ecran: {name: Message de l'écran, max: 40}
//! input_datetime:
//!   reveil: {name: Réveil, has_date: false, has_time: true}
//! ```

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::time::Duration;

use ha_core::EntityId;
use serde::Deserialize;
use serde_json::Value;

const MAX_FILE_SIZE: u64 = 256 * 1024;
/// Entrées de chaque sorte, au plus.
const MAX_PER_KIND: usize = 50;
const MAX_OPTIONS: usize = 50;
const MAX_NAME_CHARS: usize = 60;
const MAX_OPTION_CHARS: usize = 100;
const MAX_UNIT_CHARS: usize = 20;
/// Positions d'un curseur, au plus (`(max - min) / step`).
const MAX_STEPS: f64 = 1_000_000.0;
/// Durée maximale d'un minuteur.
pub const MAX_TIMER: Duration = Duration::from_hours(24);

#[derive(Debug, thiserror::Error)]
pub enum InputError {
    #[error("lecture du fichier impossible : {0}")]
    Io(#[from] std::io::Error),

    #[error("fichier trop gros (256 Kio au plus)")]
    TooLarge,

    #[error("YAML invalide : {0}")]
    Yaml(String),

    #[error("{0}")]
    Invalid(String),
}

// ───────────────────────────── Ce qu'on lit ─────────────────────────────

/// Concept Rust : `Option<BTreeMap<…>>` accepte une section vide
/// (`input_boolean:` sans rien dessous), et `Option<…Config>` une entrée
/// sans réglages (`mode_vacances:`), comme dans Home Assistant.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    #[serde(default)]
    input_boolean: Option<BTreeMap<String, Option<BooleanConfig>>>,
    #[serde(default)]
    input_number: Option<BTreeMap<String, NumberConfig>>,
    #[serde(default)]
    input_select: Option<BTreeMap<String, SelectConfig>>,
    #[serde(default)]
    input_button: Option<BTreeMap<String, Option<ButtonConfig>>>,
    #[serde(default)]
    timer: Option<BTreeMap<String, Option<TimerConfig>>>,
    #[serde(default)]
    counter: Option<BTreeMap<String, Option<CounterConfig>>>,
    #[serde(default)]
    input_text: Option<BTreeMap<String, Option<TextConfig>>>,
    #[serde(default)]
    input_datetime: Option<BTreeMap<String, Option<DateTimeConfig>>>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct CounterConfig {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    initial: Option<i64>,
    #[serde(default)]
    step: Option<i64>,
    #[serde(default)]
    minimum: Option<i64>,
    #[serde(default)]
    maximum: Option<i64>,
    /// Garder la valeur après un redémarrage (oui par défaut).
    #[serde(default)]
    restore: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct TextConfig {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    min: Option<usize>,
    #[serde(default)]
    max: Option<usize>,
    #[serde(default)]
    mode: Option<TextMode>,
    #[serde(default)]
    initial: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct DateTimeConfig {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    has_date: Option<bool>,
    #[serde(default)]
    has_time: Option<bool>,
    #[serde(default)]
    initial: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct BooleanConfig {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    initial: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NumberConfig {
    #[serde(default)]
    name: Option<String>,
    min: f64,
    max: f64,
    #[serde(default)]
    step: Option<f64>,
    #[serde(default)]
    unit_of_measurement: Option<String>,
    #[serde(default)]
    mode: Option<NumberMode>,
    #[serde(default)]
    initial: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectConfig {
    #[serde(default)]
    name: Option<String>,
    options: Vec<String>,
    #[serde(default)]
    initial: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ButtonConfig {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct TimerConfig {
    #[serde(default)]
    name: Option<String>,
    /// `"00:05:00"`, `"5min"`, ou un nombre de secondes.
    #[serde(default)]
    duration: Option<Value>,
}

// ─────────────────────────── Ce qu'on obtient ───────────────────────────

/// Comment le tableau de bord affiche un nombre : un curseur ou une case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberMode {
    #[default]
    Slider,
    Box,
}

impl NumberMode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Slider => "slider",
            Self::Box => "box",
        }
    }
}

/// Comment le tableau de bord affiche un texte : en clair, ou masqué.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextMode {
    #[default]
    Text,
    Password,
}

impl TextMode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Password => "password",
        }
    }
}

/// Un compteur entier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Counter {
    pub initial: i64,
    pub step: i64,
    pub minimum: Option<i64>,
    pub maximum: Option<i64>,
    pub restore: bool,
}

/// Une date, une heure, ou les deux.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moment {
    Date,
    Time,
    Both,
}

/// Un nombre réglable, entre `min` et `max`, par pas de `step`.
#[derive(Debug, Clone, PartialEq)]
pub struct Number {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub unit: Option<String>,
    pub mode: NumberMode,
    pub initial: Option<f64>,
}

/// La sorte d'entrée, et ses réglages.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// `input_boolean` : un interrupteur virtuel (`on` / `off`).
    Boolean { initial: Option<bool> },
    /// `input_number` : un nombre.
    Number(Number),
    /// `input_select` : un choix dans une liste.
    Select {
        options: Vec<String>,
        initial: Option<String>,
    },
    /// `input_button` : un bouton ; son état est la date du dernier appui.
    Button,
    /// `timer` : un minuteur (`idle`, `active`, `paused`).
    Timer { duration: Duration },
    /// `counter` : un compteur.
    Counter(Counter),
    /// `input_text` : un texte de `min` à `max` caractères.
    Text {
        min: usize,
        max: usize,
        mode: TextMode,
        initial: Option<String>,
    },
    /// `input_datetime` : une date et/ou une heure (`07:30:00`).
    DateTime {
        moment: Moment,
        initial: Option<jiff::civil::DateTime>,
    },
}

/// Une entrée, vérifiée.
#[derive(Debug, Clone, PartialEq)]
pub struct Helper {
    pub entity_id: EntityId,
    pub name: String,
    pub kind: Kind,
}

/// Toutes les entrées du fichier.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Config {
    pub helpers: Vec<Helper>,
}

impl Config {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.helpers.is_empty()
    }
}

/// Lit le fichier. Un fichier absent ne contient aucune entrée.
pub fn load(path: &Path) -> Result<Config, InputError> {
    let Some(text) = ha_core::config_file::read(path, MAX_FILE_SIZE)
        .map_err(|error| error.into_error(InputError::TooLarge))?
    else {
        return Ok(Config::default());
    };
    parse(&text)
}

/// Lit et vérifie le contenu du fichier.
pub fn parse(yaml: &str) -> Result<Config, InputError> {
    if yaml.trim().is_empty() {
        return Ok(Config::default());
    }
    let file: FileConfig =
        serde_saphyr::from_str(yaml).map_err(|error| InputError::Yaml(error.to_string()))?;
    let mut helpers = Vec::new();
    for (id, config) in section("input_boolean", file.input_boolean)? {
        let config = config.unwrap_or_default();
        let kind = Kind::Boolean {
            initial: config.initial,
        };
        helpers.push(helper("input_boolean", &id, config.name, kind)?);
    }
    for (id, config) in section("input_number", file.input_number)? {
        let kind = check_number(&config).map_err(|reason| invalid("input_number", &id, &reason))?;
        helpers.push(helper("input_number", &id, config.name, kind)?);
    }
    for (id, config) in section("input_select", file.input_select)? {
        let kind = check_select(config.options, config.initial)
            .map_err(|reason| invalid("input_select", &id, &reason))?;
        helpers.push(helper("input_select", &id, config.name, kind)?);
    }
    for (id, config) in section("input_button", file.input_button)? {
        let name = config.unwrap_or_default().name;
        helpers.push(helper("input_button", &id, name, Kind::Button)?);
    }
    for (id, config) in section("timer", file.timer)? {
        let config = config.unwrap_or_default();
        let duration = match &config.duration {
            None => Duration::ZERO,
            Some(value) => {
                parse_duration(value).map_err(|reason| invalid("timer", &id, &reason))?
            }
        };
        helpers.push(helper("timer", &id, config.name, Kind::Timer { duration })?);
    }
    for (id, config) in section("counter", file.counter)? {
        let config = config.unwrap_or_default();
        let kind = check_counter(&config).map_err(|reason| invalid("counter", &id, &reason))?;
        helpers.push(helper("counter", &id, config.name, kind)?);
    }
    for (id, config) in section("input_text", file.input_text)? {
        let config = config.unwrap_or_default();
        let kind = check_text(&config).map_err(|reason| invalid("input_text", &id, &reason))?;
        helpers.push(helper("input_text", &id, config.name, kind)?);
    }
    for (id, config) in section("input_datetime", file.input_datetime)? {
        let config = config.unwrap_or_default();
        let kind =
            check_datetime(&config).map_err(|reason| invalid("input_datetime", &id, &reason))?;
        helpers.push(helper("input_datetime", &id, config.name, kind)?);
    }
    Ok(Config { helpers })
}

/// Plus grand texte d'un `input_text` (la longueur maximale d'un état).
pub const MAX_TEXT: usize = 255;
/// Les valeurs d'un compteur restent dans ± un milliard.
const COUNTER_LIMIT: i64 = 1_000_000_000;

fn check_counter(config: &CounterConfig) -> Result<Kind, String> {
    let step = config.step.unwrap_or(1);
    let initial = config.initial.unwrap_or(0);
    let inside = |v: i64| (-COUNTER_LIMIT..=COUNTER_LIMIT).contains(&v);
    if !(1..=COUNTER_LIMIT).contains(&step) {
        return Err("step : un entier positif".into());
    }
    if ![Some(initial), config.minimum, config.maximum]
        .into_iter()
        .flatten()
        .all(inside)
    {
        return Err(format!("valeurs entre -{COUNTER_LIMIT} et {COUNTER_LIMIT}"));
    }
    if let (Some(min), Some(max)) = (config.minimum, config.maximum)
        && min >= max
    {
        return Err("minimum doit être plus petit que maximum".into());
    }
    if config.minimum.is_some_and(|min| initial < min)
        || config.maximum.is_some_and(|max| initial > max)
    {
        return Err("initial doit être entre minimum et maximum".into());
    }
    Ok(Kind::Counter(Counter {
        initial,
        step,
        minimum: config.minimum,
        maximum: config.maximum,
        restore: config.restore.unwrap_or(true),
    }))
}

fn check_text(config: &TextConfig) -> Result<Kind, String> {
    let min = config.min.unwrap_or(0);
    let max = config.max.unwrap_or(100);
    if max == 0 || max > MAX_TEXT || min > max {
        return Err(format!("min et max : 0 ≤ min ≤ max ≤ {MAX_TEXT}"));
    }
    if let Some(initial) = &config.initial {
        let count = initial.chars().count();
        if count < min || count > max || initial.chars().any(char::is_control) {
            return Err("initial : longueur hors de min et max, ou caractère invalide".into());
        }
    }
    Ok(Kind::Text {
        min,
        max,
        mode: config.mode.unwrap_or_default(),
        initial: config.initial.clone(),
    })
}

fn check_datetime(config: &DateTimeConfig) -> Result<Kind, String> {
    let moment = match (
        config.has_date.unwrap_or(false),
        config.has_time.unwrap_or(false),
    ) {
        (true, true) => Moment::Both,
        (true, false) => Moment::Date,
        (false, true) => Moment::Time,
        (false, false) => return Err("has_date ou has_time (ou les deux)".into()),
    };
    let initial = config
        .initial
        .as_deref()
        .map(|text| parse_moment(moment, text).ok_or_else(|| format!("initial {text:?} invalide")))
        .transpose()?;
    Ok(Kind::DateTime { moment, initial })
}

/// Lit une date (`2026-10-15`), une heure (`07:30`, `07:30:00`) ou les deux
/// (`2026-10-15 07:30:00`), selon ce que l'entrée contient. Une heure seule
/// est rangée au 1er janvier 1970 (seule l'heure compte).
#[must_use]
pub fn parse_moment(moment: Moment, text: &str) -> Option<jiff::civil::DateTime> {
    use jiff::civil::{Date, DateTime, Time};
    let text = text.trim();
    let time = |t: &str| -> Option<Time> {
        let t = if t.len() == 5 {
            format!("{t}:00")
        } else {
            t.to_owned()
        };
        t.parse::<Time>()
            .ok()
            .map(|t| t.with().subsec_nanosecond(0).build().unwrap_or(t))
    };
    let in_range = |date: Date| (2000..=2100).contains(&date.year());
    match moment {
        Moment::Date => text
            .parse::<Date>()
            .ok()
            .filter(|d| in_range(*d))
            .map(|d| d.to_datetime(Time::midnight())),
        Moment::Time => time(text).map(|t| Date::constant(1970, 1, 1).to_datetime(t)),
        Moment::Both => {
            let (date, rest) = text.split_once([' ', 'T'])?;
            let date = date.parse::<Date>().ok().filter(|d| in_range(*d))?;
            Some(DateTime::from_parts(date, time(rest)?))
        }
    }
}

fn section<T>(
    domain: &str,
    entries: Option<BTreeMap<String, T>>,
) -> Result<BTreeMap<String, T>, InputError> {
    let entries = entries.unwrap_or_default();
    if entries.len() > MAX_PER_KIND {
        return Err(InputError::Invalid(format!(
            "{domain} : {MAX_PER_KIND} entrées au plus"
        )));
    }
    Ok(entries)
}

fn invalid(domain: &str, id: &str, reason: &str) -> InputError {
    InputError::Invalid(format!("{domain}.{id} : {reason}"))
}

fn helper(domain: &str, id: &str, name: Option<String>, kind: Kind) -> Result<Helper, InputError> {
    if !ha_core::is_valid_name(id) {
        return Err(invalid(
            domain,
            id,
            "identifiant : 1 à 64 caractères parmi a-z, 0-9 et « _ »",
        ));
    }
    let entity_id: EntityId = format!("{domain}.{id}")
        .parse()
        .map_err(|_| invalid(domain, id, "identifiant trop long"))?;
    let name = name.unwrap_or_else(|| id.replace('_', " "));
    if !is_valid_text(&name, MAX_NAME_CHARS) {
        return Err(invalid(
            domain,
            id,
            &format!("nom vide, trop long ({MAX_NAME_CHARS} caractères au plus) ou invalide"),
        ));
    }
    Ok(Helper {
        entity_id,
        name,
        kind,
    })
}

fn is_valid_text(text: &str, max_chars: usize) -> bool {
    !text.trim().is_empty()
        && text.chars().count() <= max_chars
        && !text.chars().any(char::is_control)
}

fn check_number(config: &NumberConfig) -> Result<Kind, String> {
    let step = config.step.unwrap_or(1.0);
    let values = [
        Some(config.min),
        Some(config.max),
        Some(step),
        config.initial,
    ];
    if values.into_iter().flatten().any(|value| !value.is_finite()) {
        return Err("nombre invalide".into());
    }
    if config.min >= config.max {
        return Err("min doit être plus petit que max".into());
    }
    if step <= 0.0 || step > config.max - config.min {
        return Err("step doit être positif, et pas plus grand que max - min".into());
    }
    if (config.max - config.min) / step > MAX_STEPS {
        return Err("step trop petit pour cet intervalle".into());
    }
    if config
        .initial
        .is_some_and(|initial| initial < config.min || initial > config.max)
    {
        return Err("initial doit être entre min et max".into());
    }
    if let Some(unit) = &config.unit_of_measurement
        && !is_valid_text(unit, MAX_UNIT_CHARS)
    {
        return Err(format!(
            "unit_of_measurement : {MAX_UNIT_CHARS} caractères au plus"
        ));
    }
    Ok(Kind::Number(Number {
        min: config.min,
        max: config.max,
        step,
        unit: config.unit_of_measurement.clone(),
        mode: config.mode.unwrap_or_default(),
        initial: config.initial,
    }))
}

fn check_select(options: Vec<String>, initial: Option<String>) -> Result<Kind, String> {
    if options.is_empty() || options.len() > MAX_OPTIONS {
        return Err(format!("options : 1 à {MAX_OPTIONS} choix"));
    }
    let mut seen = HashSet::new();
    for option in &options {
        if !is_valid_text(option, MAX_OPTION_CHARS) {
            return Err(format!(
                "option vide, trop longue ({MAX_OPTION_CHARS} caractères au plus) ou invalide"
            ));
        }
        if !seen.insert(option.as_str()) {
            return Err(format!("option {option:?} en double"));
        }
    }
    if let Some(initial) = &initial
        && !options.contains(initial)
    {
        return Err(format!("initial {initial:?} n'est pas dans les options"));
    }
    Ok(Kind::Select { options, initial })
}

/// Une durée de minuteur : `"01:30:00"`, `"90min"`, `"30s"`, `"2h"`, ou
/// un nombre de secondes. Entre 0 et 24 heures.
pub fn parse_duration(value: &Value) -> Result<Duration, String> {
    let invalid = || "durée invalide (exemples : 00:05:00, 30s, 5min, 2h, 300)".to_owned();
    let duration = match value {
        Value::Number(seconds) => Duration::from_secs(seconds.as_u64().ok_or_else(invalid)?),
        Value::String(text) => parse_duration_text(text.trim()).ok_or_else(invalid)?,
        _ => return Err(invalid()),
    };
    if duration > MAX_TIMER {
        return Err("durée trop longue (24 h au plus)".into());
    }
    Ok(duration)
}

fn parse_duration_text(text: &str) -> Option<Duration> {
    if text.contains(':') {
        let parts: Vec<&str> = text.split(':').collect();
        let [hours, minutes, seconds] = parts.as_slice() else {
            return None;
        };
        let number = |part: &str| -> Option<u64> {
            (!part.is_empty() && part.len() <= 2 && part.bytes().all(|b| b.is_ascii_digit()))
                .then(|| part.parse().ok())
                .flatten()
        };
        let (hours, minutes, seconds) = (number(hours)?, number(minutes)?, number(seconds)?);
        if minutes > 59 || seconds > 59 {
            return None;
        }
        return Some(Duration::from_secs(hours * 3600 + minutes * 60 + seconds));
    }
    let split = text.find(|c: char| !c.is_ascii_digit())?;
    let amount: u64 = text[..split].parse().ok()?;
    let seconds = match &text[split..] {
        "s" => amount,
        "min" => amount.checked_mul(60)?,
        "h" => amount.checked_mul(3600)?,
        _ => return None,
    };
    Some(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_kind_is_read() {
        let config = parse(
            "input_boolean:\n  mode_vacances:\n  invites: {name: Invités, initial: true}\n\
             input_number:\n  consigne: {min: 15, max: 23, step: 0.5, unit_of_measurement: °C, mode: box}\n\
             input_select:\n  mode_maison: {options: [Normal, Nuit], initial: Nuit}\n\
             input_button:\n  test_sirene:\n\
             timer:\n  lessive: {name: Lessive, duration: '01:30:00'}\n  oeufs: {duration: 180}\n",
        )
        .unwrap();
        let ids: Vec<&str> = config
            .helpers
            .iter()
            .map(|h| h.entity_id.as_str())
            .collect();
        assert_eq!(
            ids,
            [
                "input_boolean.invites",
                "input_boolean.mode_vacances",
                "input_number.consigne",
                "input_select.mode_maison",
                "input_button.test_sirene",
                "timer.lessive",
                "timer.oeufs",
            ]
        );
        assert_eq!(config.helpers[1].name, "mode vacances");
        assert_eq!(
            config.helpers[0].kind,
            Kind::Boolean {
                initial: Some(true)
            }
        );
        let Kind::Number(number) = &config.helpers[2].kind else {
            panic!("un nombre est attendu");
        };
        assert_eq!((number.step, number.mode), (0.5, NumberMode::Box));
        assert_eq!(
            config.helpers[5].kind,
            Kind::Timer {
                duration: Duration::from_mins(90)
            }
        );
        assert_eq!(
            config.helpers[6].kind,
            Kind::Timer {
                duration: Duration::from_mins(3)
            }
        );
        assert!(parse("").unwrap().is_empty());
        assert!(parse("input_boolean:\n").unwrap().is_empty());
    }

    #[test]
    fn mistakes_are_refused() {
        for bad in [
            "input_bolean:\n  a:",
            "input_boolean:\n  Mode:",
            "input_boolean:\n  a: {nom: A}",
            "input_boolean:\n  a: {name: ''}",
            "input_number:\n  a: {min: 5, max: 5}",
            "input_number:\n  a: {min: 0, max: 10, step: 0}",
            "input_number:\n  a: {min: 0, max: 10, step: 20}",
            "input_number:\n  a: {min: 0, max: 10, initial: 11}",
            "input_number:\n  a: {min: 0, max: 1000000000, step: 0.001}",
            "input_number:\n  a: {max: 10}",
            "input_select:\n  a: {options: []}",
            "input_select:\n  a: {options: [A, A]}",
            "input_select:\n  a: {options: [A], initial: B}",
            "timer:\n  a: {duration: '25:00:00'}",
            "timer:\n  a: {duration: 'demain'}",
            "timer:\n  a: {duration: -5}",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        let many: String = (0..=MAX_PER_KIND).fold("input_button:\n".to_owned(), |yaml, i| {
            yaml + &format!("  b{i}:\n")
        });
        assert!(parse(&many).is_err());
    }

    #[test]
    fn durations_are_read_strictly() {
        for (value, seconds) in [
            (json!("00:05:00"), 300),
            (json!("1:30:00"), 5400),
            (json!("90min"), 5400),
            (json!("30s"), 30),
            (json!("2h"), 7200),
            (json!(45), 45),
        ] {
            assert_eq!(
                parse_duration(&value),
                Ok(Duration::from_secs(seconds)),
                "{value}"
            );
        }
        for bad in [
            json!("5"),
            json!("00:60:00"),
            json!("1:2"),
            json!(1.5),
            json!(true),
            json!("25h"),
        ] {
            assert!(parse_duration(&bad).is_err(), "{bad}");
        }
    }
}
