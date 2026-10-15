//! **Statistiques long terme** : chaque heure, un résumé par capteur
//! (moyenne, minimum, maximum, et consommation pour un compteur), gardé des
//! années. L'historique détaillé, lui, n'est gardé que quelques jours.
//!
//! - Seuls les capteurs **avec une unité** (`°C`, `kWh`, `%`…) sont résumés.
//! - La **moyenne est pondérée par la durée** : 20 °C pendant 50 minutes puis
//!   26 °C pendant 10 minutes donnent 21 °C, pas 23 °C.
//! - Un **compteur** (`state_class: total_increasing`, ou énergie, gaz, eau)
//!   garde aussi sa consommation de l'heure, même s'il repart de zéro.

use std::time::{Duration, SystemTime};

use ha_core::{Background, CancellationToken, EntityId};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use tracing::{info, warn};

use crate::error::StorageError;
use crate::store::{Store, from_micros, to_micros};

const HOUR: i64 = 3_600_000_000;
/// Le calcul d'une heure attend un peu après sa fin, pour que l'historique
/// ait fini d'écrire ses dernières lignes.
const COMPILE_DELAY: Duration = Duration::from_secs(15);
/// Points renvoyés d'un coup, au plus (plus de 10 ans par jour).
pub const MAX_STATISTICS_POINTS: usize = 4000;

/// Le résumé d'une heure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HourSummary {
    pub mean: f64,
    pub min: f64,
    pub max: f64,
    pub last: f64,
    /// Compteurs seulement : ce qui a été consommé pendant l'heure.
    pub change: Option<f64>,
}

/// Une valeur numérique, ou `None` (`unavailable`, `unknown`, texte…).
fn number(state: Option<&str>) -> Option<f64> {
    state?.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Résume une heure, de `start` à `end` (microsecondes).
///
/// `initial` : la valeur juste avant `start` ; `samples` : les changements
/// pendant l'heure, dans l'ordre. Renvoie `None` si aucune valeur n'est
/// connue pendant l'heure.
#[must_use]
pub fn summarize(
    initial: Option<f64>,
    samples: &[(i64, Option<f64>)],
    start: i64,
    end: i64,
    counter: bool,
) -> Option<HourSummary> {
    let mut current = initial;
    let mut since = start;
    let (mut weighted, mut known) = (0.0, 0_i64);
    // Seules les valeurs qui ont **duré** pendant l'heure comptent : une
    // valeur remplacée pile au début de l'heure n'en fait pas partie.
    let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut last = None;
    // Compteur : la dernière valeur connue, et la consommation cumulée.
    let mut reading = initial;
    let mut change = 0.0;

    let mut close = |current: Option<f64>, until: i64| {
        let duration = until - since;
        if let Some(value) = current
            && duration > 0
        {
            // Concept : `as f64` sur une durée en microsecondes (moins d'une
            // heure) est exact : f64 représente exactement les entiers
            // jusqu'à 2⁵³.
            #[expect(
                clippy::cast_precision_loss,
                reason = "au plus 3,6 milliards : exact en f64"
            )]
            let seconds = duration as f64;
            weighted += value * seconds;
            known += duration;
            min = min.min(value);
            max = max.max(value);
            last = Some(value);
        }
        since = until;
    };
    for &(at, value) in samples {
        close(current, at.clamp(start, end));
        current = value;
        if counter && let Some(value) = value {
            match reading {
                // Le compteur a été remis à zéro : la nouvelle valeur est ce
                // qui a été consommé depuis.
                Some(previous) if value < previous => change += value,
                Some(previous) => change += value - previous,
                None => {}
            }
            reading = Some(value);
        }
    }
    close(current, end);

    let last = last?;
    #[expect(
        clippy::cast_precision_loss,
        reason = "au plus 3,6 milliards : exact en f64"
    )]
    let mean = weighted / known as f64;
    Some(HourSummary {
        mean,
        min,
        max,
        last,
        change: counter.then_some(change),
    })
}

/// L'heure (UTC) qui contient `micros`.
fn hour_of(micros: i64) -> i64 {
    micros.div_euclid(HOUR) * HOUR
}

/// La période d'un graphique.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Hour,
    Day,
    Month,
}

/// Un point d'un graphique long terme.
#[derive(Debug, Clone, PartialEq)]
pub struct StatisticPoint {
    pub start: SystemTime,
    pub mean: f64,
    pub min: f64,
    pub max: f64,
    pub change: Option<f64>,
}

type StatisticRow = (i64, f64, f64, f64, Option<f64>);

impl Store {
    /// Calcule les statistiques de l'heure qui commence à `hour` (arrondi à
    /// l'heure) pour chaque capteur qui a une unité. Une heure déjà calculée
    /// n'est pas recalculée. Renvoie le nombre de capteurs résumés.
    pub async fn compile_statistics(&self, hour: SystemTime) -> Result<usize, StorageError> {
        let start = hour_of(to_micros(hour)?);
        let end = start + HOUR;
        let done: Option<(i64,)> =
            sqlx::query_as("SELECT start FROM statistics_runs WHERE start = ?1")
                .bind(start)
                .fetch_optional(self.pool())
                .await?;
        if done.is_some() {
            return Ok(0);
        }
        // 1. Les lectures, hors transaction : l'historique continue d'écrire
        //    pendant ce temps sans être bloqué (et sans nous bloquer).
        let sensors: Vec<(String, i64)> = sqlx::query_as(
            "SELECT entity_id,
                    COALESCE(json_extract(attributes, '$.state_class') IN ('total', 'total_increasing')
                             OR json_extract(attributes, '$.device_class') IN ('energy', 'gas', 'water'), 0)
             FROM states
             WHERE json_extract(attributes, '$.unit_of_measurement') IS NOT NULL",
        )
        .fetch_all(self.pool())
        .await?;
        let mut summaries = Vec::new();
        for (entity_id, counter) in sensors {
            let initial: Option<(Option<String>,)> = sqlx::query_as(
                "SELECT state FROM state_history
                 WHERE entity_id = ?1 AND recorded_at < ?2
                 ORDER BY recorded_at DESC, id DESC LIMIT 1",
            )
            .bind(&entity_id)
            .bind(start)
            .fetch_optional(self.pool())
            .await?;
            let rows: Vec<(i64, Option<String>)> = sqlx::query_as(
                "SELECT recorded_at, state FROM state_history
                 WHERE entity_id = ?1 AND recorded_at >= ?2 AND recorded_at < ?3
                 ORDER BY recorded_at, id",
            )
            .bind(&entity_id)
            .bind(start)
            .bind(end)
            .fetch_all(self.pool())
            .await?;
            let samples: Vec<(i64, Option<f64>)> = rows
                .iter()
                .map(|(at, state)| (*at, number(state.as_deref())))
                .collect();
            let initial = initial.and_then(|(state,)| number(state.as_deref()));
            if let Some(summary) = summarize(initial, &samples, start, end, counter != 0) {
                summaries.push((entity_id, summary));
            }
        }
        // 2. L'écriture : une transaction courte, qui commence par écrire
        //    (elle attend son tour si l'historique écrit en même temps).
        let mut tx = self.pool().begin().await?;
        let inserted =
            sqlx::query("INSERT INTO statistics_runs (start) VALUES (?1) ON CONFLICT DO NOTHING")
                .bind(start)
                .execute(&mut *tx)
                .await?
                .rows_affected();
        if inserted == 0 {
            return Ok(0);
        }
        for (entity_id, summary) in &summaries {
            sqlx::query(
                "INSERT INTO statistics (entity_id, start, mean, min, max, last, change)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (entity_id, start) DO NOTHING",
            )
            .bind(entity_id)
            .bind(start)
            .bind(summary.mean)
            .bind(summary.min)
            .bind(summary.max)
            .bind(summary.last)
            .bind(summary.change)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(summaries.len())
    }

    /// Calcule toutes les heures **terminées** qui manquent, depuis la
    /// dernière calculée (ou depuis `oldest`, la limite de l'historique).
    /// Renvoie le nombre d'heures calculées.
    pub async fn catch_up_statistics(
        &self,
        oldest: SystemTime,
        now: SystemTime,
    ) -> Result<u32, StorageError> {
        let (last,): (Option<i64>,) = sqlx::query_as("SELECT MAX(start) FROM statistics_runs")
            .fetch_one(self.pool())
            .await?;
        let oldest = hour_of(to_micros(oldest)?);
        let mut next = last.map_or(oldest, |last| (last + HOUR).max(oldest));
        let now = to_micros(now)?;
        let mut hours = 0;
        while next + HOUR <= now {
            self.compile_statistics(from_micros(next)?).await?;
            next += HOUR;
            hours += 1;
        }
        Ok(hours)
    }

    /// Les statistiques d'un capteur entre `since` et `until`, par heure,
    /// jour ou mois (jours et mois du fuseau `zone`).
    pub async fn statistics(
        &self,
        entity_id: &EntityId,
        since: SystemTime,
        until: SystemTime,
        period: Period,
        zone: &TimeZone,
    ) -> Result<Vec<StatisticPoint>, StorageError> {
        let rows: Vec<StatisticRow> = sqlx::query_as(
            "SELECT start, mean, min, max, change FROM statistics
             WHERE entity_id = ?1 AND start >= ?2 AND start < ?3
             ORDER BY start",
        )
        .bind(entity_id.as_str())
        .bind(hour_of(to_micros(since)?))
        .bind(to_micros(until)?)
        .fetch_all(self.pool())
        .await?;

        let mut points: Vec<(i64, Vec<StatisticRow>)> = Vec::new();
        for row in rows {
            let key = period_start(row.0, period, zone)?;
            match points.last_mut() {
                Some((start, group)) if *start == key => group.push(row),
                _ => points.push((key, vec![row])),
            }
        }
        let skip = points.len().saturating_sub(MAX_STATISTICS_POINTS);
        points
            .into_iter()
            .skip(skip)
            .map(|(start, group)| {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "au plus quelques centaines d'heures"
                )]
                let count = group.len() as f64;
                let changes: Vec<f64> = group.iter().filter_map(|row| row.4).collect();
                Ok(StatisticPoint {
                    start: from_micros(start)?,
                    // Chaque heure compte autant : la moyenne des moyennes.
                    mean: group.iter().map(|row| row.1).sum::<f64>() / count,
                    min: group.iter().map(|row| row.2).fold(f64::INFINITY, f64::min),
                    max: group
                        .iter()
                        .map(|row| row.3)
                        .fold(f64::NEG_INFINITY, f64::max),
                    change: (!changes.is_empty()).then(|| changes.iter().sum()),
                })
            })
            .collect()
    }
}

/// Le début du jour ou du mois (heure locale) qui contient l'heure `start`.
fn period_start(start: i64, period: Period, zone: &TimeZone) -> Result<i64, StorageError> {
    if period == Period::Hour {
        return Ok(start);
    }
    let invalid = |_| StorageError::Timestamp("date hors limites");
    let local = Timestamp::from_microsecond(start)
        .map_err(invalid)?
        .to_zoned(zone.clone());
    let day = match period {
        Period::Month => local.date().first_of_month(),
        _ => local.date(),
    };
    let begin = day.to_zoned(zone.clone()).map_err(invalid)?;
    Ok(begin.timestamp().as_microsecond())
}

/// La tâche qui calcule les statistiques chaque heure.
#[derive(Debug)]
pub struct Statistics(Background);

impl Statistics {
    /// Rattrape les heures manquantes (jusqu'à `history_days` en arrière :
    /// au-delà, l'historique détaillé n'existe plus), puis calcule chaque
    /// heure dès qu'elle est terminée.
    #[must_use]
    pub fn start(store: Store, history_days: u32) -> Self {
        Self(Background::spawn("les statistiques", move |token| {
            run(store, history_days, token)
        }))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

async fn run(store: Store, history_days: u32, token: CancellationToken) {
    let oldest = SystemTime::now()
        .checked_sub(Duration::from_hours(24 * u64::from(history_days)))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    match store.catch_up_statistics(oldest, SystemTime::now()).await {
        Ok(0) => {}
        Ok(hours) => info!("statistiques : {hours} heure(s) rattrapée(s)"),
        Err(error) => warn!("statistiques : rattrapage impossible ({error})"),
    }
    loop {
        tokio::select! {
            () = token.cancelled() => break,
            () = tokio::time::sleep(until_next_hour()) => {}
        }
        // Rattrape aussi une heure manquée (machine en veille…).
        if let Err(error) = store.catch_up_statistics(oldest, SystemTime::now()).await {
            warn!("statistiques : calcul impossible ({error})");
        }
    }
}

/// Le temps jusqu'à la prochaine heure pile, plus une petite marge.
fn until_next_hour() -> Duration {
    let now = to_micros(SystemTime::now()).unwrap_or(0);
    let next = hour_of(now) + HOUR;
    Duration::from_micros(u64::try_from(next - now).unwrap_or(0)) + COMPILE_DELAY
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000_000;

    #[test]
    fn the_mean_is_weighted_by_time() {
        // 20 °C depuis avant l'heure, puis 26 °C les 10 dernières minutes.
        let summary = summarize(Some(20.0), &[(50 * MIN, Some(26.0))], 0, HOUR, false).unwrap();
        assert!((summary.mean - 21.0).abs() < 1e-9, "{}", summary.mean);
        assert_eq!((summary.min, summary.max, summary.last), (20.0, 26.0, 26.0));
        assert_eq!(summary.change, None);
    }

    #[test]
    fn unavailable_periods_are_left_out() {
        // 10 pendant 30 min, indisponible 20 min, puis 40 pendant 10 min.
        let samples = [(30 * MIN, None), (50 * MIN, Some(40.0))];
        let summary = summarize(Some(10.0), &samples, 0, HOUR, false).unwrap();
        assert!((summary.mean - 17.5).abs() < 1e-9, "{}", summary.mean);
        // Rien de connu : pas de résumé.
        assert_eq!(summarize(None, &[], 0, HOUR, false), None);
        assert_eq!(summarize(None, &[(0, None)], 0, HOUR, false), None);
        // Une valeur apparue en cours d'heure : la moyenne porte sur sa durée.
        let late = summarize(None, &[(45 * MIN, Some(8.0))], 0, HOUR, false).unwrap();
        assert!((late.mean - 8.0).abs() < 1e-9);
    }

    #[test]
    fn a_counter_survives_a_reset() {
        // 100 → 103 → (remise à zéro) 1 → 2 : 3 + 1 + 1 = 5 consommés.
        let samples = [
            (10 * MIN, Some(103.0)),
            (20 * MIN, Some(1.0)),
            (30 * MIN, Some(2.0)),
        ];
        let summary = summarize(Some(100.0), &samples, 0, HOUR, true).unwrap();
        assert_eq!(summary.change, Some(5.0));
        assert!((summary.last - 2.0).abs() < 1e-9);
        // Sans changement : 0 consommé (et pas « inconnu »).
        let quiet = summarize(Some(100.0), &[], 0, HOUR, true).unwrap();
        assert_eq!(quiet.change, Some(0.0));
    }

    #[test]
    fn values_are_read_strictly() {
        assert_eq!(number(Some(" 21.5 ")), Some(21.5));
        for bad in [
            None,
            Some("on"),
            Some("unavailable"),
            Some("NaN"),
            Some("inf"),
        ] {
            assert_eq!(number(bad), None, "{bad:?}");
        }
        assert_eq!(hour_of(HOUR + 5), HOUR);
        assert_eq!(hour_of(-5), -HOUR);
    }
}
