//! Les déclencheurs `time` et `time_pattern` les jours de changement d'heure.
//! On fait tourner l'évaluateur seconde par seconde, comme le moteur, sur
//! toute une nuit, en heure de Paris.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use ha_automation::evaluator::Evaluator;
use ha_automation::parse;
use ha_core::{Attributes, Core};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use tokio::time::Instant;

const YAML: &str = r#"
- id: deux_heures_et_demie
  triggers: [{trigger: time, at: "02:30:00"}]
  actions: [{delay: 1s}]
- id: chaque_quart
  triggers: [{trigger: time_pattern, minutes: 15}]
  actions: [{delay: 1s}]
- id: trois_heures
  triggers: [{trigger: time, at: "03:00:00"}]
  actions: [{delay: 1s}]
- id: reveil
  triggers: [{trigger: time, at: input_datetime.reveil}]
  actions: [{delay: 1s}]
"#;

/// Paris, écrit comme une règle POSIX : pas besoin de la base des fuseaux
/// (absente de certaines images Docker), et le test ne dépend pas du fuseau
/// de la machine. Heure d'hiver UTC+1, heure d'été UTC+2 du dernier dimanche
/// de mars (2 h) au dernier dimanche d'octobre (3 h).
fn paris() -> TimeZone {
    TimeZone::posix("CET-1CEST,M3.5.0,M10.5.0/3").unwrap()
}

/// Fait tourner l'évaluateur chaque seconde de `from` à `to` (moments réels)
/// et rend, pour chaque déclenchement, l'automatisation et le moment.
fn night(from: &str, to: &str) -> Vec<(usize, Timestamp)> {
    let core = Core::new();
    core.states
        .set(
            "input_datetime.reveil".parse().unwrap(),
            "02:30:00",
            Attributes::new(),
        )
        .unwrap();
    let mut evaluator = Evaluator::new(parse(YAML).unwrap()).with_zone(paris());
    let end: Timestamp = to.parse().unwrap();
    let mut previous: Timestamp = from.parse().unwrap();
    let mut fired = Vec::new();
    while previous < end {
        let now = previous + SignedDuration::from_secs(1);
        for automation in evaluator.on_tick_at(previous, now, Instant::now(), &core.states) {
            fired.push((automation, now));
        }
        previous = now;
    }
    fired
}

fn times(fired: &[(usize, Timestamp)], automation: usize) -> Vec<String> {
    fired
        .iter()
        .filter(|(a, _)| *a == automation)
        .map(|(_, at)| at.to_zoned(paris()).strftime("%F %T%:z").to_string())
        .collect()
}

#[test]
fn an_ordinary_night() {
    // De minuit à 5 h, heure d'été.
    let fired = night("2026-10-14T22:00:00Z", "2026-10-15T03:00:00Z");
    assert_eq!(times(&fired, 0), ["2026-10-15 02:30:00+02:00"]);
    assert_eq!(times(&fired, 1).len(), 5, "00 h 15 … 04 h 15");
    assert_eq!(times(&fired, 2).len(), 1);
    assert_eq!(times(&fired, 3), times(&fired, 0));
}

#[test]
fn clocks_go_back_in_october() {
    // De minuit (heure d'été) à 5 h (heure d'hiver) : six heures réelles, et
    // l'horloge passe deux fois de 2 h à 3 h.
    let fired = night("2026-10-24T22:00:00Z", "2026-10-25T04:00:00Z");
    // 2 h 30 sonne une fois, la première (encore en heure d'été).
    assert_eq!(times(&fired, 0), ["2026-10-25 02:30:00+02:00"]);
    // Le motif aussi : 2 h 15 n'est pas rejoué.
    let quarters = times(&fired, 1);
    assert_eq!(quarters.len(), 5, "{quarters:?}");
    assert!(quarters[2].starts_with("2026-10-25 02:15:00+02:00"));
    assert!(quarters[3].starts_with("2026-10-25 03:15:00+01:00"));
    assert_eq!(times(&fired, 2), ["2026-10-25 03:00:00+01:00"]);
    // L'heure lue dans une entité suit la même règle.
    assert_eq!(times(&fired, 3), times(&fired, 0));
}

#[test]
fn clocks_go_forward_in_march() {
    // De minuit (heure d'hiver) à 5 h (heure d'été) : quatre heures réelles,
    // l'horloge saute de 2 h à 3 h.
    let fired = night("2026-03-28T23:00:00Z", "2026-03-29T03:00:00Z");
    let three = "2026-03-29 03:00:00+02:00";
    // 2 h 30 n'existe pas : une seule fois, à la fin du trou.
    assert_eq!(times(&fired, 0), [three]);
    assert_eq!(times(&fired, 2), [three]);
    assert_eq!(times(&fired, 3), [three]);
    // 2 h 15 sonne aussi à 3 h ; les autres quarts à leur heure.
    let quarters = times(&fired, 1);
    assert_eq!(quarters.len(), 5, "{quarters:?}");
    assert_eq!(quarters[2], three);
    assert!(quarters[3].starts_with("2026-03-29 03:15:00+02:00"));
}

#[test]
fn a_long_sleep_across_the_change_fires_once() {
    // La machine dort de 1 h 50 à 3 h 10 (heure d'hiver), en octobre : un
    // seul tic couvre les deux passages de 2 h 30.
    let core = Core::new();
    let mut evaluator = Evaluator::new(parse(YAML).unwrap()).with_zone(paris());
    let previous: Timestamp = "2026-10-24T23:50:00Z".parse().unwrap();
    let now: Timestamp = "2026-10-25T02:10:00Z".parse().unwrap();
    let fired = evaluator.on_tick_at(previous, now, Instant::now(), &core.states);
    assert_eq!(fired, [0, 1, 2]);
}
