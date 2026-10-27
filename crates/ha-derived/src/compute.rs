//! Les calculs des assistants, sans attente ni réseau : l'heure et les états
//! sont passés en paramètre, pour les tester facilement.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ha_core::State;
use jiff::ToSpan;
use jiff::civil::{Date, DateTime, Time};

use crate::config::{Block, Cycle, Statistic, day_index};

/// Un état qui ne compte pas (appareil absent ou muet).
fn silent(state: &State) -> bool {
    matches!(state.state(), "unavailable" | "unknown")
}

/// Les membres connus qui répondent.
fn answering(members: &[Option<Arc<State>>]) -> Vec<&State> {
    members
        .iter()
        .flatten()
        .map(Arc::as_ref)
        .filter(|s| !silent(s))
        .collect()
}

/// L'état d'un groupe de lumières, prises, ventilateurs ou détecteurs :
/// allumé si au moins un membre l'est (ou tous, avec `all`). `None` si aucun
/// membre ne répond.
#[must_use]
pub fn on_off_group(members: &[Option<Arc<State>>], all: bool) -> Option<bool> {
    let answering: Vec<&State> = answering(members);
    if answering.is_empty() {
        return None;
    }
    let on = answering.iter().filter(|s| s.state() == "on").count();
    Some(if all { on == answering.len() } else { on > 0 })
}

/// L'état d'un groupe de volets : en mouvement si l'un bouge, ouvert si l'un
/// est ouvert, fermé si tous le sont. Avec la position moyenne.
#[must_use]
pub fn cover_group(members: &[Option<Arc<State>>]) -> Option<(&'static str, Option<f64>)> {
    let answering: Vec<&State> = answering(members);
    if answering.is_empty() {
        return None;
    }
    let any = |value: &str| answering.iter().any(|s| s.state() == value);
    let state = if any("opening") {
        "opening"
    } else if any("closing") {
        "closing"
    } else if any("open") {
        "open"
    } else {
        "closed"
    };
    let positions: Vec<f64> = answering
        .iter()
        .filter_map(|s| s.attributes().get("current_position")?.as_f64())
        .collect();
    let position = (!positions.is_empty())
        .then(|| (positions.iter().sum::<f64>() / count(positions.len())).round());
    Some((state, position))
}

/// `usize` → `f64` sans perte pour une liste raisonnable (50 au plus ici).
fn count(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// Minimum, maximum, moyenne… des capteurs qui répondent par un nombre.
/// `last` prend le plus récemment changé.
#[must_use]
pub fn statistic(kind: Statistic, sources: &[Option<Arc<State>>]) -> Option<f64> {
    let mut values: Vec<(f64, &State)> = sources
        .iter()
        .flatten()
        .map(Arc::as_ref)
        .filter_map(|s| {
            Some((
                s.state()
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())?,
                s,
            ))
        })
        .collect();
    if values.is_empty() {
        return None;
    }
    let numbers = || values.iter().map(|(v, _)| *v);
    Some(match kind {
        Statistic::Min => numbers().fold(f64::INFINITY, f64::min),
        Statistic::Max => numbers().fold(f64::NEG_INFINITY, f64::max),
        Statistic::Sum => numbers().sum(),
        Statistic::Mean => numbers().sum::<f64>() / count(values.len()),
        Statistic::Range => {
            numbers().fold(f64::NEG_INFINITY, f64::max) - numbers().fold(f64::INFINITY, f64::min)
        }
        Statistic::Last => values
            .iter()
            .max_by_key(|(_, s)| s.last_updated())
            .map_or(0.0, |(v, _)| *v),
        Statistic::Median => {
            values.sort_by(|a, b| a.0.total_cmp(&b.0));
            let middle = values.len() / 2;
            if values.len().is_multiple_of(2) {
                f64::midpoint(values[middle - 1].0, values[middle].0)
            } else {
                values[middle].0
            }
        }
    })
}

/// Arrondi à `digits` décimales, écrit simplement (`21.5`, `20`).
#[must_use]
pub fn rounded(value: f64, digits: u8) -> String {
    let text = format!("{value:.*}", usize::from(digits));
    if text.contains('.') {
        let trimmed = text.trim_end_matches('0').trim_end_matches('.');
        if trimmed == "-0" {
            "0".to_owned()
        } else {
            trimmed.to_owned()
        }
    } else {
        text
    }
}

/// Où est la valeur par rapport au seuil.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Above,
    Below,
    InRange,
}

impl Position {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Above => "above",
            Self::Below => "below",
            Self::InRange => "in_range",
        }
    }
}

/// Le seuil, comme dans Home Assistant, avec une **hystérésis** : pour ne
/// pas basculer sans arrêt autour de la limite, il faut la dépasser de
/// `hysteresis` dans un sens ou dans l'autre. Entre les deux, on garde
/// l'état précédent (`before`).
///
/// - seulement `upper` : allumé au-dessus ;
/// - seulement `lower` : allumé en dessous ;
/// - les deux : allumé entre les deux.
#[must_use]
pub fn threshold(
    before: Option<(bool, Position)>,
    value: f64,
    lower: Option<f64>,
    upper: Option<f64>,
    hysteresis: f64,
) -> (bool, Position) {
    let h = hysteresis;
    let keep = || before.unwrap_or((false, Position::Below));
    match (lower, upper) {
        (None, Some(upper)) => {
            if value > upper + h {
                (true, Position::Above)
            } else if value < upper - h {
                (false, Position::Below)
            } else {
                keep()
            }
        }
        (Some(lower), None) => {
            if value < lower - h {
                (true, Position::Below)
            } else if value > lower + h {
                (false, Position::Above)
            } else {
                keep()
            }
        }
        (Some(lower), Some(upper)) => {
            if value < lower - h {
                (false, Position::Below)
            } else if value > upper + h {
                (false, Position::Above)
            } else if value > lower + h && value < upper - h {
                (true, Position::InRange)
            } else {
                keep()
            }
        }
        (None, None) => keep(),
    }
}

/// La plage horaire est-elle active à `now`, et quand change-t-elle ?
#[must_use]
pub fn schedule(week: &[Vec<Block>; 7], now: DateTime) -> (bool, Option<DateTime>) {
    let blocks_of = |date: Date| &week[day_index(date.weekday())];
    let inside =
        |block: &Block, time: Time| block.from <= time && block.to.is_none_or(|to| time < to);
    let on = blocks_of(now.date()).iter().any(|b| inside(b, now.time()));
    // Le prochain changement : le premier début ou fin après maintenant, sur
    // huit jours au plus. Une plage qui finit à minuit et une qui commence à
    // minuit le lendemain ne font pas de changement.
    let mut next = None;
    'days: for offset in 0..8_i64 {
        let Ok(date) = now.date().checked_add(offset.days()) else {
            break;
        };
        let mut moments: Vec<DateTime> = Vec::new();
        for block in blocks_of(date) {
            moments.push(date.to_datetime(block.from));
            moments.push(match block.to {
                Some(to) => date.to_datetime(to),
                None => match date.tomorrow() {
                    Ok(tomorrow) => tomorrow.to_datetime(Time::midnight()),
                    Err(_) => continue,
                },
            });
        }
        moments.sort();
        for moment in moments {
            if moment > now && schedule_on(week, moment) != on {
                next = Some(moment);
                break 'days;
            }
        }
    }
    (on, next)
}

fn schedule_on(week: &[Vec<Block>; 7], at: DateTime) -> bool {
    week[day_index(at.date().weekday())]
        .iter()
        .any(|b| b.from <= at.time() && b.to.is_none_or(|to| at.time() < to))
}

/// Le début de la période en cours (heure locale).
#[must_use]
pub fn period_start(cycle: Cycle, now: DateTime) -> DateTime {
    let date = now.date();
    let midnight = |date: Date| date.to_datetime(Time::midnight());
    match cycle {
        Cycle::Hourly => date.to_datetime(Time::constant(now.hour(), 0, 0, 0)),
        Cycle::Daily => midnight(date),
        Cycle::Weekly => {
            let back = i64::from(date.weekday().to_monday_zero_offset());
            midnight(date.checked_sub(back.days()).unwrap_or(date))
        }
        Cycle::Monthly => midnight(date.first_of_month()),
        Cycle::Yearly => midnight(date.first_of_year()),
    }
}

/// Un compteur de consommation : ce qui a été consommé depuis le début de
/// la période, d'après un compteur qui ne fait que monter (kWh, m³…).
#[derive(Debug, Clone, PartialEq)]
pub struct Meter {
    pub value: f64,
    pub last_period: Option<f64>,
    pub period: DateTime,
    /// La dernière valeur lue sur le compteur source.
    pub last_source: Option<f64>,
}

impl Meter {
    /// Une nouvelle valeur du compteur source. S'il a **baissé**, il a été
    /// remis à zéro (changement de compteur…) : tout ce qu'il affiche est
    /// nouveau, comme dans Home Assistant.
    pub fn feed(&mut self, source: f64) {
        if !source.is_finite() || source < 0.0 {
            return;
        }
        if let Some(previous) = self.last_source {
            let delta = if source >= previous {
                source - previous
            } else {
                source
            };
            self.value += delta;
        }
        self.last_source = Some(source);
    }

    /// Une nouvelle période commence-t-elle ? On garde alors le total de la
    /// précédente et on repart de zéro. Renvoie `true` s'il a changé.
    pub fn roll(&mut self, cycle: Cycle, now: DateTime) -> bool {
        let start = period_start(cycle, now);
        if start == self.period {
            return false;
        }
        // Une période sautée (serveur arrêté plusieurs jours) n'a rien compté.
        self.last_period = Some(if self.period == period_before(cycle, start) {
            self.value
        } else {
            0.0
        });
        self.value = 0.0;
        self.period = start;
        true
    }
}

/// Le début de la période qui précède celle commençant à `start`.
fn period_before(cycle: Cycle, start: DateTime) -> DateTime {
    let earlier = match cycle {
        Cycle::Hourly => start.checked_sub(1.hour()),
        Cycle::Daily => start.checked_sub(1.day()),
        Cycle::Weekly => start.checked_sub(7.days()),
        Cycle::Monthly => start.checked_sub(1.month()),
        Cycle::Yearly => start.checked_sub(1.year()),
    };
    earlier.map_or(start, |moment| period_start(cycle, moment))
}

/// Comment l'intégrale compte entre deux mesures (étape 65).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// La valeur reste celle de la mesure précédente jusqu'à la suivante :
    /// exact pour nos capteurs, qui ne publient qu'au changement.
    Left,
    /// La moyenne des deux mesures (comme Home Assistant par défaut).
    Trapezoidal,
}

/// Une intégrale : ce qu'une puissance a fourni (W → Wh), au fil du temps.
#[derive(Debug, Clone, PartialEq)]
pub struct Integral {
    /// Le total, dans l'unité de sortie (kWh…).
    pub total: f64,
    /// La dernière mesure, et quand elle a été prise en compte.
    last: Option<(f64, Instant)>,
}

impl Integral {
    /// Reprend le total d'avant l'arrêt du serveur (l'arrêt lui-même n'est
    /// pas compté : on ne sait pas ce qui s'est passé).
    #[must_use]
    pub fn resume(total: f64) -> Self {
        Self {
            total: if total.is_finite() { total } else { 0.0 },
            last: None,
        }
    }

    /// Avance jusqu'à `now`, puis retient `value` comme nouvelle mesure
    /// (`None` : la source est indisponible, rien n'est compté jusqu'à son
    /// retour). `scale` convertit valeur × secondes en unité de sortie
    /// (W·s → kWh : 1 / 3 600 000).
    pub fn feed(&mut self, value: Option<f64>, now: Instant, method: Method, scale: f64) {
        if let Some((previous, at)) = self.last {
            let seconds = now.saturating_duration_since(at).as_secs_f64();
            let height = match (method, value) {
                (Method::Trapezoidal, Some(value)) => f64::midpoint(previous, value),
                _ => previous,
            };
            let area = height * seconds * scale;
            if area.is_finite() {
                self.total += area;
            }
        }
        self.last = value.filter(|v| v.is_finite()).map(|value| (value, now));
    }

    /// A-t-elle une mesure en cours ?
    #[must_use]
    pub fn running(&self) -> bool {
        self.last.is_some()
    }

    /// Avance jusqu'à `now` sans nouvelle mesure (la valeur n'a pas changé).
    pub fn advance(&mut self, now: Instant, method: Method, scale: f64) {
        if let Some((value, _)) = self.last {
            self.feed(Some(value), now, method, scale);
        }
    }
}

/// Une dérivée : à quelle vitesse une valeur change, sur une fenêtre de
/// temps (étape 65).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Slope {
    /// Les mesures de la fenêtre (et la dernière d'avant, qui donne la
    /// valeur au début de la fenêtre).
    samples: VecDeque<(Instant, f64)>,
}

/// Mesures gardées au plus dans la fenêtre d'une dérivée.
const MAX_SAMPLES: usize = 500;

impl Slope {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Une nouvelle mesure (`None` : indisponible, la fenêtre repart de zéro).
    pub fn feed(&mut self, value: Option<f64>, now: Instant) {
        match value.filter(|v| v.is_finite()) {
            Some(value) => {
                if self.samples.len() == MAX_SAMPLES {
                    self.samples.pop_front();
                }
                self.samples.push_back((now, value));
            }
            None => self.samples.clear(),
        }
    }

    /// La pente à `now`, en unité par seconde : de la valeur au début de la
    /// fenêtre à la valeur actuelle (une valeur reste la même jusqu'à la
    /// mesure suivante). `None` tant qu'il n'y a pas deux mesures.
    pub fn per_second(&mut self, now: Instant, window: Duration) -> Option<f64> {
        let start = now.checked_sub(window)?;
        // On garde une seule mesure d'avant la fenêtre : la valeur au début.
        while self.samples.len() > 1 && self.samples.get(1).is_some_and(|(at, _)| *at <= start) {
            self.samples.pop_front();
        }
        let (first_at, first) = *self.samples.front()?;
        if self.samples.len() < 2 {
            // Une seule mesure, d'avant la fenêtre : rien n'a bougé.
            return (first_at <= start).then_some(0.0);
        }
        let (_, last) = *self.samples.back()?;
        let from = first_at.max(start);
        let seconds = now.saturating_duration_since(from).as_secs_f64();
        if seconds < 1.0 {
            return None;
        }
        Some((last - first) / seconds)
    }
}

/// Où en est un appareil à cycle (lave-linge, lave-vaisselle…), étape 67.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Off,
    Running,
    Finished,
}

impl Phase {
    /// L'état publié.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "Arrêt",
            Self::Running => "En marche",
            Self::Finished => "Terminé",
        }
    }
}

/// Un cycle qui vient de finir : sa durée, et son énergie si on la connaît.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Done {
    pub duration: Duration,
    pub energy: Option<f64>,
}

/// Le suivi d'un appareil d'après sa puissance : en marche au-dessus de
/// `start_above`, terminé quand elle reste sous `stop_below` pendant
/// `finished_after` (une pause de trempage ne compte pas pour une fin).
#[derive(Debug, Clone, PartialEq)]
pub struct Appliance {
    pub phase: Phase,
    /// Début du cycle en cours (ou du dernier).
    started: Option<Instant>,
    /// L'énergie du compteur au début du cycle.
    energy_at_start: Option<f64>,
    /// Depuis quand la puissance est basse.
    low_since: Option<Instant>,
}

impl Appliance {
    #[must_use]
    pub fn new(phase: Phase) -> Self {
        Self {
            phase,
            started: None,
            energy_at_start: None,
            low_since: None,
        }
    }

    /// Reprend après un redémarrage : un cycle en cours est compté depuis
    /// maintenant (son début est perdu).
    #[must_use]
    pub fn resumed(phase: Phase, now: Instant) -> Self {
        Self {
            started: (phase == Phase::Running).then_some(now),
            ..Self::new(phase)
        }
    }

    /// Une mesure, ou le temps qui passe (`power` : la dernière connue).
    /// Renvoie le cycle qui vient de se terminer.
    pub fn step(
        &mut self,
        now: Instant,
        power: Option<f64>,
        energy: Option<f64>,
        start_above: f64,
        stop_below: f64,
        finished_after: Duration,
    ) -> Option<Done> {
        let power = power?;
        if power > start_above {
            self.low_since = None;
            if self.phase != Phase::Running {
                self.phase = Phase::Running;
                self.started = Some(now);
                self.energy_at_start = energy;
            }
            return None;
        }
        if self.phase != Phase::Running {
            return None;
        }
        if power >= stop_below {
            // Entre les deux seuils : on ne décide rien.
            self.low_since = None;
            return None;
        }
        let since = *self.low_since.get_or_insert(now);
        if now.saturating_duration_since(since) < finished_after {
            return None;
        }
        self.phase = Phase::Finished;
        self.low_since = None;
        let duration = self.started.map_or(Duration::ZERO, |start| {
            since.saturating_duration_since(start)
        });
        let used = match (energy, self.energy_at_start) {
            (Some(end), Some(start)) if end >= start => Some(end - start),
            _ => None,
        };
        Some(Done {
            duration,
            energy: used,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ha_core::{Attributes, Core};
    use jiff::civil::date;
    use serde_json::{Value, json};

    fn state(value: &str, attributes: Value) -> Option<Arc<State>> {
        let Value::Object(attributes) = attributes else {
            panic!("un objet est attendu")
        };
        let core = Core::new();
        let id: ha_core::EntityId = "sensor.x".parse().unwrap();
        core.states.set(id.clone(), value, attributes).unwrap();
        core.states.get(&id)
    }

    #[test]
    fn groups() {
        let lights = [
            state("on", json!({})),
            state("off", json!({})),
            state("unavailable", json!({})),
        ];
        assert_eq!(on_off_group(&lights, false), Some(true));
        assert_eq!(on_off_group(&lights, true), Some(false));
        assert_eq!(
            on_off_group(&[state("unavailable", json!({})), None], false),
            None
        );

        let covers = [
            state("open", json!({"current_position": 100})),
            state("closed", json!({"current_position": 0})),
        ];
        assert_eq!(cover_group(&covers), Some(("open", Some(50.0))));
        let closed = [state("closed", json!({})), state("closed", json!({}))];
        assert_eq!(cover_group(&closed), Some(("closed", None)));
    }

    #[test]
    fn statistics() {
        let sensors = [
            state("19.5", json!({})),
            state("21", json!({})),
            state("unavailable", json!({})),
            state("22.5", json!({})),
        ];
        assert_eq!(statistic(Statistic::Mean, &sensors), Some(21.0));
        assert_eq!(statistic(Statistic::Min, &sensors), Some(19.5));
        assert_eq!(statistic(Statistic::Range, &sensors), Some(3.0));
        assert_eq!(statistic(Statistic::Median, &sensors[..2]), Some(20.25));
        assert_eq!(statistic(Statistic::Sum, &[state("x", json!({}))]), None);
        assert_eq!(rounded(20.25, 1), "20.2");
        assert_eq!(rounded(21.0, 2), "21");
        assert_eq!(rounded(-0.001, 1), "0");
        let _ = Attributes::new();
    }

    #[test]
    fn threshold_with_hysteresis() {
        let upper = |before, value| threshold(before, value, None, Some(70.0), 2.0);
        assert_eq!(
            upper(None, 71.0),
            (false, Position::Below),
            "pas assez au-dessus"
        );
        let on = upper(None, 72.5);
        assert_eq!(on, (true, Position::Above));
        // Redescend un peu : reste allumé.
        assert_eq!(upper(Some(on), 69.0), on);
        assert_eq!(upper(Some(on), 67.5), (false, Position::Below));

        let range = |value| threshold(None, value, Some(18.0), Some(22.0), 0.0).0;
        assert!(range(20.0));
        assert!(!range(17.0));
        assert!(!range(23.0));
    }

    #[test]
    fn schedules() {
        let week: [Vec<Block>; 7] = std::array::from_fn(|day| {
            if day < 5 {
                vec![Block {
                    from: Time::constant(6, 30, 0, 0),
                    to: Some(Time::constant(8, 0, 0, 0)),
                }]
            } else {
                Vec::new()
            }
        });
        // Jeudi 15 octobre 2026.
        let thursday = date(2026, 10, 15);
        assert_eq!(
            schedule(&week, thursday.at(7, 0, 0, 0)),
            (true, Some(thursday.at(8, 0, 0, 0)))
        );
        assert_eq!(
            schedule(&week, thursday.at(9, 0, 0, 0)),
            (false, Some(date(2026, 10, 16).at(6, 30, 0, 0)))
        );
        // Vendredi soir : prochain lundi.
        assert_eq!(
            schedule(&week, date(2026, 10, 16).at(20, 0, 0, 0)),
            (false, Some(date(2026, 10, 19).at(6, 30, 0, 0)))
        );
    }

    #[test]
    fn a_daily_meter() {
        let day = date(2026, 10, 15);
        let mut meter = Meter {
            value: 0.0,
            last_period: None,
            period: period_start(Cycle::Daily, day.at(8, 0, 0, 0)),
            last_source: None,
        };
        meter.feed(1000.0); // première lecture : la référence
        meter.feed(1002.5);
        meter.feed(1003.0);
        assert!((meter.value - 3.0).abs() < 1e-9);
        // Compteur remplacé : il repart de 0.
        meter.feed(0.5);
        assert!((meter.value - 3.5).abs() < 1e-9);
        assert!(!meter.roll(Cycle::Daily, day.at(23, 59, 0, 0)));
        assert!(meter.roll(Cycle::Daily, date(2026, 10, 16).at(0, 0, 1, 0)));
        assert_eq!(meter.last_period, Some(3.5));
        assert!(meter.value.abs() < 1e-9);
        // Trois jours sans serveur : la période d'avant n'a rien compté.
        assert!(meter.roll(Cycle::Daily, date(2026, 10, 19).at(9, 0, 0, 0)));
        assert_eq!(meter.last_period, Some(0.0));
        assert_eq!(
            period_start(Cycle::Weekly, day.at(9, 0, 0, 0)),
            date(2026, 10, 12).at(0, 0, 0, 0)
        );
        assert_eq!(
            period_start(Cycle::Monthly, day.at(9, 0, 0, 0)),
            date(2026, 10, 1).at(0, 0, 0, 0)
        );
    }

    #[test]
    fn an_integral_turns_power_into_energy() {
        // W·s → kWh.
        let scale = 1.0 / 3_600_000.0;
        let start = Instant::now();
        let at = |minutes: u64| start + Duration::from_mins(minutes);
        let mut energy = Integral::resume(10.0);
        // 1 000 W pendant 30 min, puis 2 000 W pendant 15 min : 0,5 + 0,5 kWh.
        energy.feed(Some(1000.0), at(0), Method::Left, scale);
        energy.advance(at(10), Method::Left, scale);
        assert!(
            (energy.total - (10.0 + 1.0 / 6.0)).abs() < 1e-9,
            "{}",
            energy.total
        );
        energy.feed(Some(2000.0), at(30), Method::Left, scale);
        energy.advance(at(45), Method::Left, scale);
        assert!((energy.total - 11.0).abs() < 1e-9, "{}", energy.total);
        // Source indisponible : rien n'est compté jusqu'à son retour.
        energy.feed(None, at(45), Method::Left, scale);
        energy.advance(at(120), Method::Left, scale);
        energy.feed(Some(500.0), at(120), Method::Left, scale);
        assert!((energy.total - 11.0).abs() < 1e-9, "{}", energy.total);
        // Trapèzes : la moyenne des deux mesures.
        let mut trapezes = Integral::resume(0.0);
        trapezes.feed(Some(0.0), at(0), Method::Trapezoidal, scale);
        trapezes.feed(Some(2000.0), at(60), Method::Trapezoidal, scale);
        assert!((trapezes.total - 1.0).abs() < 1e-9, "{}", trapezes.total);
        assert_eq!(Integral::resume(f64::NAN).total, 0.0);
    }

    #[test]
    fn a_slope_over_a_window() {
        let start = Instant::now();
        let at = |minutes: u64| start + Duration::from_mins(minutes);
        let window = Duration::from_mins(10);
        let mut slope = Slope::default();
        slope.feed(Some(20.0), at(0));
        assert_eq!(slope.per_second(at(5), window), None, "une seule mesure");
        // La fenêtre est ouverte : 20 → 19,5 en 5 min, soit -6 °C/h.
        slope.feed(Some(19.5), at(5));
        let per_hour = slope.per_second(at(5), window).unwrap() * 3600.0;
        assert!((per_hour + 6.0).abs() < 1e-9, "{per_hour}");
        // 20 min plus tard, plus rien ne bouge : la pente revient à zéro.
        assert_eq!(slope.per_second(at(25), window), Some(0.0));
        // Indisponible : on repart de zéro.
        slope.feed(None, at(26));
        assert_eq!(slope.per_second(at(27), window), None);
    }

    #[test]
    fn a_washing_machine_cycle() {
        let start = Instant::now();
        let at = |minutes: u64| start + Duration::from_mins(minutes);
        let finished_after = Duration::from_mins(5);
        let mut machine = Appliance::new(Phase::Off);
        let step = |m: &mut Appliance, minutes, power: f64, energy: f64| {
            m.step(
                at(minutes),
                Some(power),
                Some(energy),
                10.0,
                5.0,
                finished_after,
            )
        };
        // En veille : rien.
        assert_eq!(step(&mut machine, 0, 0.8, 100.0), None);
        assert_eq!(machine.phase, Phase::Off);
        // Le cycle démarre.
        assert_eq!(step(&mut machine, 1, 2000.0, 100.0), None);
        assert_eq!(machine.phase, Phase::Running);
        // Trempage : 3 minutes à 2 W, ce n'est pas une fin.
        assert_eq!(step(&mut machine, 20, 2.0, 100.6), None);
        assert_eq!(step(&mut machine, 23, 150.0, 100.6), None);
        // Fin : sous 5 W pendant 5 minutes.
        assert_eq!(step(&mut machine, 80, 1.0, 101.2), None);
        let done = step(&mut machine, 85, 1.0, 101.2).unwrap();
        assert_eq!(machine.phase, Phase::Finished);
        // La durée va jusqu'au moment où la puissance est retombée.
        assert_eq!(done.duration, Duration::from_mins(79));
        assert!((done.energy.unwrap() - 1.2).abs() < 1e-9);
        // Il reste « Terminé » en veille ; un nouveau cycle repart.
        assert_eq!(step(&mut machine, 120, 0.5, 101.2), None);
        assert_eq!(machine.phase, Phase::Finished);
        assert_eq!(step(&mut machine, 300, 1800.0, 101.2), None);
        assert_eq!(machine.phase, Phase::Running);
        // Sans mesure, rien ne change.
        assert_eq!(
            machine.step(at(400), None, None, 10.0, 5.0, finished_after),
            None
        );
        assert_eq!(machine.phase, Phase::Running);
    }
}
