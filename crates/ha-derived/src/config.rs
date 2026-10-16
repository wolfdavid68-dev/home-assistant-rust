//! Le fichier `derived.yaml` : lecture et vérification.
//!
//! ```yaml
//! group:
//!   lumieres_du_bas: {name: Lumières du bas, type: light, entities: [light.salon, light.cuisine]}
//! schedule:
//!   chauffage_matin:
//!     name: Chauffage du matin
//!     monday: [{from: "06:30", to: "08:00"}]
//! min_max:
//!   temperature_moyenne: {type: mean, entities: [sensor.salon, sensor.chambre], round_digits: 1}
//! threshold:
//!   trop_humide: {entity_id: sensor.humidite_sdb, upper: 70, hysteresis: 2}
//! utility_meter:
//!   electricite_du_jour: {source: sensor.compteur, cycle: daily}
//! history_stats:
//!   chauffage_du_jour: {entity_id: switch.radiateur, state: "on", type: time, cycle: daily}
//! ```
//!
//! Sécurité : le fichier est écrit par toi, mais on le vérifie comme s'il
//! venait d'ailleurs (tailles, noms, valeurs). Une erreur arrête le démarrage
//! avec une explication, plutôt qu'un assistant qui ferait autre chose.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use ha_core::EntityId;
use jiff::civil::{Time, Weekday};
use serde::Deserialize;

const MAX_FILE_SIZE: u64 = 256 * 1024;
const MAX_PER_KIND: usize = 50;
const MAX_MEMBERS: usize = 50;
const MAX_NAME_CHARS: usize = 60;
const MAX_BLOCKS_PER_DAY: usize = 10;

#[derive(Debug, thiserror::Error)]
pub enum DerivedError {
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

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    #[serde(default)]
    group: Option<BTreeMap<String, GroupConfig>>,
    #[serde(default)]
    schedule: Option<BTreeMap<String, ScheduleConfig>>,
    #[serde(default)]
    min_max: Option<BTreeMap<String, MinMaxConfig>>,
    #[serde(default)]
    threshold: Option<BTreeMap<String, ThresholdConfig>>,
    #[serde(default)]
    utility_meter: Option<BTreeMap<String, MeterConfig>>,
    #[serde(default)]
    history_stats: Option<BTreeMap<String, HistoryStatsConfig>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupConfig {
    #[serde(default)]
    name: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    entities: Vec<String>,
    /// Allumé seulement si **tous** le sont (sinon : au moins un).
    #[serde(default)]
    all: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockConfig {
    from: String,
    to: String,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ScheduleConfig {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    monday: Vec<BlockConfig>,
    #[serde(default)]
    tuesday: Vec<BlockConfig>,
    #[serde(default)]
    wednesday: Vec<BlockConfig>,
    #[serde(default)]
    thursday: Vec<BlockConfig>,
    #[serde(default)]
    friday: Vec<BlockConfig>,
    #[serde(default)]
    saturday: Vec<BlockConfig>,
    #[serde(default)]
    sunday: Vec<BlockConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MinMaxConfig {
    #[serde(default)]
    name: Option<String>,
    #[serde(rename = "type", default)]
    kind: Option<String>,
    entities: Vec<String>,
    #[serde(default)]
    round_digits: Option<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThresholdConfig {
    #[serde(default)]
    name: Option<String>,
    entity_id: String,
    #[serde(default)]
    lower: Option<f64>,
    #[serde(default)]
    upper: Option<f64>,
    #[serde(default)]
    hysteresis: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MeterConfig {
    #[serde(default)]
    name: Option<String>,
    source: String,
    cycle: String,
}

/// Un état, ou une liste d'états (`state: [heat, on]`).
#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryStatsConfig {
    #[serde(default)]
    name: Option<String>,
    entity_id: String,
    state: OneOrMany,
    #[serde(rename = "type")]
    measure: String,
    cycle: String,
}

// ─────────────────────────── Ce qu'on obtient ───────────────────────────

/// Les sortes de groupes (le domaine de l'entité créée).
pub const GROUP_TYPES: [&str; 5] = ["light", "switch", "fan", "cover", "binary_sensor"];

/// Ce qu'on calcule sur plusieurs capteurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Statistic {
    Min,
    Max,
    Mean,
    Median,
    Last,
    Sum,
    Range,
}

/// La période d'un compteur de consommation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cycle {
    Hourly,
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl Cycle {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hourly => "hourly",
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
            Self::Yearly => "yearly",
        }
    }
}

/// Une plage horaire d'un jour : de `from` (inclus) à `to` (exclu).
/// `to: "24:00"` veut dire jusqu'à minuit (`to = None`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    pub from: Time,
    pub to: Option<Time>,
}

/// Ce que calcule un assistant.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Group {
        members: Vec<EntityId>,
        all: bool,
    },
    /// Les plages de chaque jour, lundi d'abord.
    Schedule {
        week: [Vec<Block>; 7],
    },
    MinMax {
        statistic: Statistic,
        sources: Vec<EntityId>,
        round_digits: u8,
    },
    Threshold {
        source: EntityId,
        lower: Option<f64>,
        upper: Option<f64>,
        hysteresis: f64,
    },
    Meter {
        source: EntityId,
        cycle: Cycle,
    },
    /// Combien de temps (ou de fois) une entité a été dans un état, depuis
    /// le début de la période (étape 59).
    HistoryStats {
        source: EntityId,
        states: Vec<String>,
        measure: Measure,
        cycle: Cycle,
    },
}

/// Ce que mesure une statistique d'historique.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measure {
    /// Des heures.
    Time,
    /// Combien de fois l'entité est entrée dans l'état.
    Count,
    /// La part de la période, en %.
    Ratio,
}

/// Un assistant vérifié.
#[derive(Debug, Clone, PartialEq)]
pub struct Derived {
    pub entity_id: EntityId,
    pub name: String,
    pub kind: Kind,
}

impl Derived {
    /// Les entités dont il dépend.
    #[must_use]
    pub fn sources(&self) -> Vec<&EntityId> {
        match &self.kind {
            Kind::Group { members, .. } => members.iter().collect(),
            Kind::MinMax { sources, .. } => sources.iter().collect(),
            Kind::Threshold { source, .. }
            | Kind::Meter { source, .. }
            | Kind::HistoryStats { source, .. } => vec![source],
            Kind::Schedule { .. } => Vec::new(),
        }
    }
}

/// Lit le fichier. Un fichier absent ne contient aucun assistant.
pub fn load(path: &Path) -> Result<Vec<Derived>, DerivedError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    if metadata.len() > MAX_FILE_SIZE {
        return Err(DerivedError::TooLarge);
    }
    parse(&std::fs::read_to_string(path)?)
}

/// Lit et vérifie le contenu du fichier.
pub fn parse(yaml: &str) -> Result<Vec<Derived>, DerivedError> {
    if yaml.trim().is_empty() {
        return Ok(Vec::new());
    }
    let file: FileConfig =
        serde_saphyr::from_str(yaml).map_err(|error| DerivedError::Yaml(error.to_string()))?;
    let mut all = Vec::new();

    let groups = section("group", file.group)?;
    // Un groupe ne contient pas d'autre groupe : pas de boucle possible
    // (« A allume B qui allume A… »).
    let group_ids: HashSet<String> = groups
        .iter()
        .filter(|(_, config)| GROUP_TYPES.contains(&config.kind.as_str()))
        .map(|(id, config)| format!("{}.{id}", config.kind))
        .collect();
    for (id, config) in groups {
        let place = format!("group.{id}");
        if !GROUP_TYPES.contains(&config.kind.as_str()) {
            return Err(invalid(
                &place,
                &format!("type : {}", GROUP_TYPES.join(", ")),
            ));
        }
        let members = entity_list(&place, &config.entities)?;
        for member in &members {
            if member.domain() != config.kind {
                return Err(invalid(
                    &place,
                    &format!("{member} n'est pas un « {} »", config.kind),
                ));
            }
            if group_ids.contains(member.as_str()) {
                return Err(invalid(
                    &place,
                    &format!("{member} est un groupe : pas de groupe dans un groupe"),
                ));
            }
        }
        let kind = Kind::Group {
            members,
            all: config.all,
        };
        all.push(derived(&config.kind, &id, config.name, kind)?);
    }

    for (id, config) in section("schedule", file.schedule)? {
        let place = format!("schedule.{id}");
        let days = [
            config.monday,
            config.tuesday,
            config.wednesday,
            config.thursday,
            config.friday,
            config.saturday,
            config.sunday,
        ];
        let mut week: [Vec<Block>; 7] = Default::default();
        for (day, blocks) in week.iter_mut().zip(days) {
            *day = check_blocks(&place, &blocks)?;
        }
        all.push(derived(
            "schedule",
            &id,
            config.name,
            Kind::Schedule { week },
        )?);
    }

    for (id, config) in section("min_max", file.min_max)? {
        let place = format!("min_max.{id}");
        let statistic = match config.kind.as_deref().unwrap_or("max") {
            "min" => Statistic::Min,
            "max" => Statistic::Max,
            "mean" => Statistic::Mean,
            "median" => Statistic::Median,
            "last" => Statistic::Last,
            "sum" => Statistic::Sum,
            "range" => Statistic::Range,
            _ => {
                return Err(invalid(
                    &place,
                    "type : min, max, mean, median, last, sum ou range",
                ));
            }
        };
        let round_digits = config.round_digits.unwrap_or(2);
        if round_digits > 6 {
            return Err(invalid(&place, "round_digits : 6 au plus"));
        }
        let kind = Kind::MinMax {
            statistic,
            sources: entity_list(&place, &config.entities)?,
            round_digits,
        };
        all.push(derived("sensor", &id, config.name, kind)?);
    }

    for (id, config) in section("threshold", file.threshold)? {
        let place = format!("threshold.{id}");
        let hysteresis = config.hysteresis.unwrap_or(0.0);
        let finite = [config.lower, config.upper, Some(hysteresis)]
            .into_iter()
            .flatten()
            .all(f64::is_finite);
        if !finite || hysteresis < 0.0 {
            return Err(invalid(&place, "nombres invalides (hysteresis ≥ 0)"));
        }
        match (config.lower, config.upper) {
            (None, None) => return Err(invalid(&place, "il faut lower ou upper")),
            (Some(lower), Some(upper)) if lower >= upper => {
                return Err(invalid(&place, "lower doit être plus petit que upper"));
            }
            _ => {}
        }
        let kind = Kind::Threshold {
            source: entity(&place, &config.entity_id)?,
            lower: config.lower,
            upper: config.upper,
            hysteresis,
        };
        all.push(derived("binary_sensor", &id, config.name, kind)?);
    }

    for (id, config) in section("utility_meter", file.utility_meter)? {
        let place = format!("utility_meter.{id}");
        let kind = Kind::Meter {
            source: entity(&place, &config.source)?,
            cycle: cycle(&place, &config.cycle)?,
        };
        all.push(derived("sensor", &id, config.name, kind)?);
    }

    for (id, config) in section("history_stats", file.history_stats)? {
        let place = format!("history_stats.{id}");
        let measure = match config.measure.as_str() {
            "time" => Measure::Time,
            "count" => Measure::Count,
            "ratio" => Measure::Ratio,
            _ => return Err(invalid(&place, "type : time, count ou ratio")),
        };
        let states = match config.state {
            OneOrMany::One(state) => vec![state],
            OneOrMany::Many(states) => states,
        };
        if states.is_empty()
            || states.len() > 10
            || states
                .iter()
                .any(|s| s.is_empty() || s.len() > 64 || s.chars().any(char::is_control))
        {
            return Err(invalid(
                &place,
                "state : 1 à 10 états de 64 caractères au plus",
            ));
        }
        let kind = Kind::HistoryStats {
            source: entity(&place, &config.entity_id)?,
            states,
            measure,
            cycle: cycle(&place, &config.cycle)?,
        };
        all.push(derived("sensor", &id, config.name, kind)?);
    }

    // Deux assistants ne peuvent pas porter le même identifiant, ni se
    // calculer l'un à partir de l'autre (pas de boucle).
    let mut seen = HashSet::new();
    for item in &all {
        if !seen.insert(item.entity_id.clone()) {
            return Err(DerivedError::Invalid(format!(
                "{} : défini deux fois",
                item.entity_id
            )));
        }
    }
    for item in &all {
        if let Some(source) = item.sources().into_iter().find(|s| seen.contains(*s)) {
            return Err(DerivedError::Invalid(format!(
                "{} : {source} est lui-même un assistant calculé",
                item.entity_id
            )));
        }
    }
    Ok(all)
}

fn cycle(place: &str, text: &str) -> Result<Cycle, DerivedError> {
    Ok(match text {
        "hourly" => Cycle::Hourly,
        "daily" => Cycle::Daily,
        "weekly" => Cycle::Weekly,
        "monthly" => Cycle::Monthly,
        "yearly" => Cycle::Yearly,
        _ => {
            return Err(invalid(
                place,
                "cycle : hourly, daily, weekly, monthly ou yearly",
            ));
        }
    })
}

fn section<T>(
    name: &str,
    entries: Option<BTreeMap<String, T>>,
) -> Result<BTreeMap<String, T>, DerivedError> {
    let entries = entries.unwrap_or_default();
    if entries.len() > MAX_PER_KIND {
        return Err(DerivedError::Invalid(format!(
            "{name} : {MAX_PER_KIND} au plus"
        )));
    }
    Ok(entries)
}

fn invalid(place: &str, reason: &str) -> DerivedError {
    DerivedError::Invalid(format!("{place} : {reason}"))
}

fn entity(place: &str, text: &str) -> Result<EntityId, DerivedError> {
    EntityId::new(text).map_err(|error| invalid(place, &error.to_string()))
}

fn entity_list(place: &str, texts: &[String]) -> Result<Vec<EntityId>, DerivedError> {
    if texts.is_empty() || texts.len() > MAX_MEMBERS {
        return Err(invalid(place, &format!("1 à {MAX_MEMBERS} entités")));
    }
    let ids = texts
        .iter()
        .map(|text| entity(place, text))
        .collect::<Result<Vec<_>, _>>()?;
    let unique: HashSet<&EntityId> = ids.iter().collect();
    if unique.len() != ids.len() {
        return Err(invalid(place, "une entité en double"));
    }
    Ok(ids)
}

fn derived(
    domain: &str,
    id: &str,
    name: Option<String>,
    kind: Kind,
) -> Result<Derived, DerivedError> {
    let place = format!("{domain}.{id}");
    if !ha_core::is_valid_name(id) {
        return Err(invalid(
            &place,
            "identifiant : 1 à 64 caractères parmi a-z, 0-9 et « _ »",
        ));
    }
    let entity_id: EntityId = place
        .parse()
        .map_err(|_| invalid(&place, "identifiant trop long"))?;
    let name = name.unwrap_or_else(|| id.replace('_', " "));
    let valid = !name.trim().is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && !name.chars().any(char::is_control);
    if !valid {
        return Err(invalid(
            &place,
            &format!("nom vide ou trop long ({MAX_NAME_CHARS} caractères au plus)"),
        ));
    }
    Ok(Derived {
        entity_id,
        name,
        kind,
    })
}

/// Les plages d'un jour : triées, sans chevauchement, `from` avant `to`.
fn check_blocks(place: &str, blocks: &[BlockConfig]) -> Result<Vec<Block>, DerivedError> {
    if blocks.len() > MAX_BLOCKS_PER_DAY {
        return Err(invalid(
            place,
            &format!("{MAX_BLOCKS_PER_DAY} plages par jour au plus"),
        ));
    }
    let mut checked = blocks
        .iter()
        .map(|block| {
            let from = time(&block.from)
                .ok_or_else(|| invalid(place, &format!("heure {:?} invalide", block.from)))?;
            let to = match block.to.trim() {
                "24:00" | "24:00:00" => None,
                text => Some(
                    time(text)
                        .ok_or_else(|| invalid(place, &format!("heure {text:?} invalide")))?,
                ),
            };
            if to.is_some_and(|to| to <= from) {
                return Err(invalid(
                    place,
                    &format!(
                        "{} → {} : la fin doit être après le début",
                        block.from, block.to
                    ),
                ));
            }
            Ok(Block { from, to })
        })
        .collect::<Result<Vec<_>, _>>()?;
    checked.sort_by_key(|block| block.from);
    for pair in checked.windows(2) {
        if pair[0].to.is_none_or(|end| end > pair[1].from) {
            return Err(invalid(place, "deux plages se chevauchent"));
        }
    }
    Ok(checked)
}

/// `07:30` ou `07:30:00`.
fn time(text: &str) -> Option<Time> {
    let text = text.trim();
    // `jiff` lirait aussi l'heure d'une date complète : on n'accepte que l'heure.
    if text.contains('-') || !(5..=8).contains(&text.len()) {
        return None;
    }
    let text = if text.len() == 5 {
        format!("{text}:00")
    } else {
        text.to_owned()
    };
    text.parse().ok()
}

/// Le jour de la semaine, lundi = 0.
#[must_use]
pub fn day_index(day: Weekday) -> usize {
    usize::try_from(day.to_monday_zero_offset()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_is_read() {
        let all = parse(
            "group:\n  bas: {name: Lumières du bas, type: light, entities: [light.salon, light.cuisine]}\n\
             schedule:\n  matin: {monday: [{from: '06:30', to: '08:00'}, {from: '18:00', to: '24:00'}]}\n\
             min_max:\n  moyenne: {type: mean, entities: [sensor.a, sensor.b], round_digits: 1}\n\
             threshold:\n  humide: {entity_id: sensor.humidite, upper: 70, hysteresis: 2}\n\
             utility_meter:\n  jour: {source: sensor.compteur, cycle: daily}\n",
        )
        .unwrap();
        let ids: Vec<&str> = all.iter().map(|d| d.entity_id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "light.bas",
                "schedule.matin",
                "sensor.moyenne",
                "binary_sensor.humide",
                "sensor.jour"
            ]
        );
        let Kind::Schedule { week } = &all[1].kind else {
            panic!("plages attendues")
        };
        assert_eq!(week[0].len(), 2);
        assert_eq!(week[0][1].to, None, "24:00 = jusqu'à minuit");
        assert_eq!(week[1], Vec::<Block>::new());
    }

    #[test]
    fn mistakes_are_refused() {
        for bad in [
            "group:\n  a: {type: lock, entities: [lock.porte]}",
            "group:\n  a: {type: light, entities: [switch.prise]}",
            "group:\n  a: {type: light, entities: []}",
            "group:\n  a: {type: light, entities: [light.x, light.x]}",
            // Un groupe dans un groupe.
            "group:\n  a: {type: light, entities: [light.b]}\n  b: {type: light, entities: [light.c]}",
            "schedule:\n  a: {monday: [{from: '08:00', to: '07:00'}]}",
            "schedule:\n  a: {monday: [{from: '07:00', to: '09:00'}, {from: '08:00', to: '10:00'}]}",
            "schedule:\n  a: {monday: [{from: '2026-10-15 07:00', to: '09:00'}]}",
            "schedule:\n  a: {lundi: []}",
            "min_max:\n  a: {type: moyenne, entities: [sensor.a]}",
            "threshold:\n  a: {entity_id: sensor.a}",
            "threshold:\n  a: {entity_id: sensor.a, lower: 5, upper: 3}",
            "threshold:\n  a: {entity_id: sensor.a, upper: 3, hysteresis: -1}",
            "utility_meter:\n  a: {source: sensor.a, cycle: daily}\n  b: {source: sensor.a, cycle: chaque_jour}",
            // Un assistant calculé à partir d'un autre.
            "min_max:\n  a: {entities: [sensor.b]}\nutility_meter:\n  b: {source: sensor.c, cycle: daily}",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn history_stats_are_checked() {
        let all = parse(
            "history_stats:\n  chauffage: {entity_id: climate.salon, state: [heat], type: time, cycle: daily}\n  \
             portes: {name: Portes, entity_id: binary_sensor.porte, state: \"on\", type: count, cycle: weekly}\n",
        )
        .unwrap();
        assert_eq!(all[0].entity_id.as_str(), "sensor.chauffage");
        let Kind::HistoryStats {
            states,
            measure,
            cycle,
            ..
        } = &all[1].kind
        else {
            panic!()
        };
        assert_eq!(
            (states.as_slice(), *measure, *cycle),
            (&["on".to_owned()][..], Measure::Count, Cycle::Weekly)
        );
        for bad in [
            "history_stats:\n  a: {entity_id: switch.a, state: \"on\", type: duree, cycle: daily}",
            "history_stats:\n  a: {entity_id: switch.a, state: [], type: time, cycle: daily}",
            "history_stats:\n  a: {entity_id: switch.a, state: \"on\", type: time, cycle: toujours}",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
