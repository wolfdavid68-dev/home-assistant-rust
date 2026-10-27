//! Le **bilan énergétique** : ce que la maison a consommé (et produit) par
//! jour, par mois et sur l'année, avec ce que ça coûte.
//!
//! Les chiffres viennent des **statistiques long terme** (étape 24) : chaque
//! heure, la consommation de chaque compteur est déjà calculée et gardée des
//! années. Rien de plus n'est enregistré ici.
//!
//! Le fichier `energy.yaml` dit quels compteurs compter, et leur prix :
//!
//! ```yaml
//! consumption:
//!   - entity: sensor.linky_heures_creuses
//!     price: 0.1696          # € par kWh (facultatif)
//!   - entity: sensor.linky_heures_pleines
//!     price: 0.2146
//! production:
//!   - entity: sensor.linky_production
//!     price: 0.1276          # prix de revente (facultatif)
//! subscription: 15.47        # abonnement, € par mois (facultatif)
//! ```
//!
//! Sans fichier, les index du compteur Linky sont pris automatiquement.

use std::path::Path;
use std::time::SystemTime;

use ha_core::{EntityId, StateMachine};
use ha_storage::{Period, Store};
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan, Zoned};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_FILE_SIZE: u64 = 64 * 1024;
const MAX_SOURCES: usize = 20;
/// Un prix au-delà de 10 € le kWh est sûrement une faute de frappe.
const MAX_PRICE: f64 = 10.0;
const MAX_SUBSCRIPTION: f64 = 1_000.0;
/// Jours de détail, au plus.
pub const MAX_DAYS: u32 = 366;

#[derive(Debug, thiserror::Error)]
pub enum EnergyError {
    #[error("lecture du fichier impossible : {0}")]
    Io(#[from] std::io::Error),
    #[error("fichier trop gros (64 Kio au plus)")]
    TooLarge,
    #[error("YAML invalide : {0}")]
    Yaml(String),
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Storage(#[from] ha_storage::StorageError),
    #[error("date hors limites")]
    Date,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    #[serde(default)]
    consumption: Vec<SourceConfig>,
    #[serde(default)]
    production: Vec<SourceConfig>,
    #[serde(default)]
    subscription: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceConfig {
    entity: String,
    #[serde(default)]
    price: Option<f64>,
}

/// Consommation ou production.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Consumption,
    Production,
}

/// Un compteur compté dans le bilan.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub entity_id: EntityId,
    pub kind: Kind,
    /// € par kWh.
    pub price: Option<f64>,
}

/// Le contenu de `energy.yaml`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EnergyConfig {
    /// Vide : les compteurs sont choisis automatiquement (index du Linky).
    pub sources: Vec<Source>,
    /// € par mois.
    pub subscription: Option<f64>,
}

fn check_price(place: &str, value: Option<f64>, max: f64) -> Result<Option<f64>, EnergyError> {
    match value {
        Some(price) if !price.is_finite() || !(0.0..=max).contains(&price) => Err(
            EnergyError::Invalid(format!("{place} : le prix doit être entre 0 et {max}")),
        ),
        other => Ok(other),
    }
}

/// Lit le fichier. Un fichier absent donne la configuration automatique.
pub fn load(path: &Path) -> Result<EnergyConfig, EnergyError> {
    let Some(text) = ha_core::config_file::read(path, MAX_FILE_SIZE)
        .map_err(|error| error.into_error(EnergyError::TooLarge))?
    else {
        return Ok(EnergyConfig::default());
    };
    parse(&text)
}

/// Lit et vérifie le contenu du fichier.
pub fn parse(yaml: &str) -> Result<EnergyConfig, EnergyError> {
    if yaml.trim().is_empty() {
        return Ok(EnergyConfig::default());
    }
    let file: FileConfig =
        serde_saphyr::from_str(yaml).map_err(|error| EnergyError::Yaml(error.to_string()))?;
    if file.consumption.len() + file.production.len() > MAX_SOURCES {
        return Err(EnergyError::Invalid(format!(
            "{MAX_SOURCES} compteurs au plus"
        )));
    }
    let mut sources: Vec<Source> = Vec::new();
    for (kind, list) in [
        (Kind::Consumption, file.consumption),
        (Kind::Production, file.production),
    ] {
        for source in list {
            let entity_id: EntityId = source.entity.parse().map_err(|_| {
                EnergyError::Invalid(format!("entité invalide : {}", source.entity))
            })?;
            if entity_id.domain() != "sensor" {
                return Err(EnergyError::Invalid(format!(
                    "{entity_id} : un compteur d'énergie est un capteur (sensor.…)"
                )));
            }
            if sources.iter().any(|s| s.entity_id == entity_id) {
                return Err(EnergyError::Invalid(format!("{entity_id} est en double")));
            }
            let price = check_price(entity_id.as_str(), source.price, MAX_PRICE)?;
            sources.push(Source {
                entity_id,
                kind,
                price,
            });
        }
    }
    let subscription = check_price("subscription", file.subscription, MAX_SUBSCRIPTION)?;
    Ok(EnergyConfig {
        sources,
        subscription,
    })
}

impl EnergyConfig {
    /// Les compteurs à compter maintenant. Sans fichier : les index du
    /// Linky (`sensor.linky_…` en kWh), sinon sa consommation totale, et sa
    /// production.
    #[must_use]
    pub fn active_sources(&self, states: &StateMachine) -> Vec<Source> {
        if !self.sources.is_empty() {
            return self.sources.clone();
        }
        let mut linky: Vec<EntityId> = states
            .all()
            .into_iter()
            .filter(|state| {
                state.entity_id().as_str().starts_with("sensor.linky_")
                    && state
                        .attributes()
                        .get("state_class")
                        .and_then(Value::as_str)
                        == Some("total_increasing")
            })
            .map(|state| state.entity_id().clone())
            .collect();
        linky.sort();
        let production = |id: &EntityId| id.as_str() == "sensor.linky_production";
        let total = |id: &EntityId| id.as_str() == "sensor.linky_energie";
        let indexes: Vec<&EntityId> = linky
            .iter()
            .filter(|id| !production(id) && !total(id))
            .collect();
        let mut sources: Vec<Source> = if indexes.is_empty() {
            linky.iter().filter(|id| total(id)).collect::<Vec<_>>()
        } else {
            indexes
        }
        .into_iter()
        .map(|id| Source {
            entity_id: id.clone(),
            kind: Kind::Consumption,
            price: None,
        })
        .collect();
        sources.extend(linky.iter().filter(|id| production(id)).map(|id| Source {
            entity_id: id.clone(),
            kind: Kind::Production,
            price: None,
        }));
        sources
    }
}

/// Un compteur, tel que le tableau de bord le reçoit.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SourceReport {
    pub entity_id: String,
    pub name: String,
    pub kind: Kind,
    pub price: Option<f64>,
}

/// Un jour (ou un mois) : kWh par compteur, dans l'ordre des compteurs.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PeriodReport {
    /// `2026-10-14` (jour) ou `2026-10` (mois).
    pub date: String,
    pub kwh: Vec<Option<f64>>,
}

/// Un total : énergie consommée, produite, et ce que ça coûte.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Default)]
pub struct Total {
    pub consumed_kwh: f64,
    pub produced_kwh: f64,
    /// Le coût de ce qui a été consommé (sans l'abonnement), si les prix
    /// sont connus.
    pub cost: Option<f64>,
    /// Ce que rapporte la production revendue, si son prix est connu.
    pub revenue: Option<f64>,
}

/// Le bilan complet.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// `true` : les compteurs viennent de `energy.yaml`.
    pub configured: bool,
    pub sources: Vec<SourceReport>,
    pub days: Vec<PeriodReport>,
    pub months: Vec<PeriodReport>,
    pub today: Total,
    pub month: Total,
    pub year: Total,
    pub subscription: Option<f64>,
}

fn total(sources: &[Source], kwh: &[Option<f64>]) -> Total {
    let mut total = Total::default();
    let (mut cost, mut revenue) = (Some(0.0), Some(0.0));
    for (source, value) in sources.iter().zip(kwh) {
        let value = value.unwrap_or(0.0);
        match source.kind {
            Kind::Consumption => {
                total.consumed_kwh += value;
                cost = cost
                    .zip(source.price)
                    .map(|(sum, price)| sum + value * price);
            }
            Kind::Production => {
                total.produced_kwh += value;
                revenue = revenue
                    .zip(source.price)
                    .map(|(sum, price)| sum + value * price);
            }
        }
    }
    let has = |kind| sources.iter().any(|s| s.kind == kind);
    total.cost = cost.filter(|_| has(Kind::Consumption));
    total.revenue = revenue.filter(|_| has(Kind::Production));
    total
}

fn add(sum: &mut [Option<f64>], values: &[Option<f64>]) {
    for (sum, value) in sum.iter_mut().zip(values) {
        if let Some(value) = value {
            *sum = Some(sum.unwrap_or(0.0) + value);
        }
    }
}

fn start_of(date: Date, zone: &TimeZone) -> Result<SystemTime, EnergyError> {
    let zoned = date.to_zoned(zone.clone()).map_err(|_| EnergyError::Date)?;
    Ok(SystemTime::from(zoned.timestamp()))
}

/// Les valeurs de chaque compteur, par période, entre `since` et `until`.
async fn series(
    store: &Store,
    sources: &[Source],
    since: SystemTime,
    until: SystemTime,
    period: Period,
    zone: &TimeZone,
) -> Result<Vec<(Timestamp, Vec<Option<f64>>)>, EnergyError> {
    let mut rows: Vec<(Timestamp, Vec<Option<f64>>)> = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        for point in store
            .statistics(&source.entity_id, since, until, period, zone)
            .await?
        {
            let Some(change) = point.change else { continue };
            let at = Timestamp::try_from(point.start).map_err(|_| EnergyError::Date)?;
            let position = match rows.binary_search_by(|(start, _)| start.cmp(&at)) {
                Ok(position) => position,
                Err(position) => {
                    rows.insert(position, (at, vec![None; sources.len()]));
                    position
                }
            };
            if let Some(slot) = rows
                .get_mut(position)
                .and_then(|(_, values)| values.get_mut(index))
            {
                *slot = Some(change);
            }
        }
    }
    Ok(rows)
}

/// Calcule le bilan des `days` derniers jours, du mois et de l'année.
///
/// Les statistiques sont calculées à la fin de chaque heure : l'heure en
/// cours n'est pas encore comptée.
pub async fn report(
    store: &Store,
    states: &StateMachine,
    config: &EnergyConfig,
    days: u32,
    now: &Zoned,
) -> Result<Report, EnergyError> {
    let zone = now.time_zone().clone();
    let sources = config.active_sources(states);
    let today = now.date();
    let first_day = today
        .checked_sub(i64::from(days.clamp(1, MAX_DAYS) - 1).days())
        .map_err(|_| EnergyError::Date)?;
    let first_of_year = today.first_of_year();
    let until = SystemTime::from(now.timestamp());

    let daily = series(
        store,
        &sources,
        start_of(first_day, &zone)?,
        until,
        Period::Day,
        &zone,
    )
    .await?;
    let monthly = series(
        store,
        &sources,
        start_of(first_of_year, &zone)?,
        until,
        Period::Month,
        &zone,
    )
    .await?;

    let local = |at: &Timestamp| at.to_zoned(zone.clone()).date();
    let days: Vec<PeriodReport> = daily
        .iter()
        .map(|(at, kwh)| PeriodReport {
            date: local(at).to_string(),
            kwh: kwh.clone(),
        })
        .collect();
    let months: Vec<PeriodReport> = monthly
        .iter()
        .map(|(at, kwh)| PeriodReport {
            date: local(at).strftime("%Y-%m").to_string(),
            kwh: kwh.clone(),
        })
        .collect();

    let empty = vec![None; sources.len()];
    let today_kwh = daily
        .iter()
        .find(|(at, _)| local(at) == today)
        .map_or(&empty, |(_, kwh)| kwh);
    let month_kwh = monthly
        .iter()
        .find(|(at, _)| local(at) == today.first_of_month())
        .map_or(&empty, |(_, kwh)| kwh);
    let mut year_kwh = empty.clone();
    for (_, kwh) in &monthly {
        add(&mut year_kwh, kwh);
    }

    let name = |id: &EntityId| {
        states
            .get(id)
            .and_then(|state| {
                state
                    .attributes()
                    .get("friendly_name")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| id.as_str().to_owned())
    };
    Ok(Report {
        configured: !config.sources.is_empty(),
        sources: sources
            .iter()
            .map(|source| SourceReport {
                entity_id: source.entity_id.to_string(),
                name: name(&source.entity_id),
                kind: source.kind,
                price: source.price,
            })
            .collect(),
        today: total(&sources, today_kwh),
        month: total(&sources, month_kwh),
        year: total(&sources, &year_kwh),
        days,
        months,
        subscription: config.subscription,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::float_cmp, reason = "tests")]

    use super::*;

    #[test]
    fn the_file_is_checked() {
        let config = parse(
            "consumption:\n  - {entity: sensor.hc, price: 0.17}\n  - {entity: sensor.hp}\n\
             production:\n  - {entity: sensor.solaire, price: 0.13}\nsubscription: 15.47\n",
        )
        .unwrap();
        assert_eq!(config.sources.len(), 3);
        assert_eq!(config.sources[0].price, Some(0.17));
        assert_eq!(config.sources[2].kind, Kind::Production);
        assert_eq!(config.subscription, Some(15.47));
        assert_eq!(parse("").unwrap(), EnergyConfig::default());

        for bad in [
            "consumption: [{entity: light.salon}]",
            "consumption: [{entity: sensor.a}, {entity: sensor.a}]",
            "consumption: [{entity: sensor.a, price: -1}]",
            "consumption: [{entity: sensor.a, price: 50}]",
            "consumption: [{entity: sensor.a, prix: 0.2}]",
            "subscription: .nan",
            "autre: 1",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn totals_and_costs() {
        let id = |s: &str| s.parse::<EntityId>().unwrap();
        let sources = [
            Source {
                entity_id: id("sensor.hc"),
                kind: Kind::Consumption,
                price: Some(0.2),
            },
            Source {
                entity_id: id("sensor.hp"),
                kind: Kind::Consumption,
                price: Some(0.3),
            },
            Source {
                entity_id: id("sensor.pv"),
                kind: Kind::Production,
                price: None,
            },
        ];
        let result = total(&sources, &[Some(10.0), None, Some(4.0)]);
        assert_eq!(result.consumed_kwh, 10.0);
        assert_eq!(result.produced_kwh, 4.0);
        assert_eq!(result.cost.map(|c| (c * 100.0).round()), Some(200.0));
        // Pas de prix de revente : on ne devine pas.
        assert_eq!(result.revenue, None);
    }
}
