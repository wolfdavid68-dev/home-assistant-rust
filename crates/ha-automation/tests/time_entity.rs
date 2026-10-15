//! Le déclencheur `time` qui suit une entrée `input_datetime` (l'heure du
//! réveil, réglable depuis le tableau de bord).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use ha_automation::evaluator::Evaluator;
use ha_automation::parse;
use ha_core::{Attributes, Core};
use jiff::civil::date;
use tokio::time::Instant;

fn set(core: &Core, entity: &str, value: &str) {
    core.states
        .set(entity.parse().unwrap(), value, Attributes::new())
        .unwrap();
}

#[test]
fn follows_the_wake_up_time() {
    let yaml = "- id: reveil\n  triggers: [{trigger: time, at: input_datetime.reveil}]\n  actions: [{delay: 1s}]\n";
    let core = Core::new();
    set(&core, "input_datetime.reveil", "06:45:00");
    let mut evaluator = Evaluator::new(parse(yaml).unwrap());
    let day = date(2026, 10, 15);
    let now = Instant::now();
    let mut tick = |a, b| evaluator.on_tick(a, b, now, &core.states);

    assert_eq!(tick(day.at(6, 44, 59, 0), day.at(6, 45, 0, 0)), [0]);
    // On règle le réveil plus tard : c'est la nouvelle heure qui compte.
    set(&core, "input_datetime.reveil", "07:30:00");
    assert_eq!(tick(day.at(7, 29, 59, 0), day.at(7, 30, 0, 0)), [0]);
    // Une date et une heure : une seule fois, ce jour-là.
    set(&core, "input_datetime.reveil", "2026-10-16 07:30:00");
    assert_eq!(
        tick(day.at(7, 29, 59, 0), day.at(7, 30, 0, 0)),
        Vec::<usize>::new()
    );
    let next = date(2026, 10, 16);
    assert_eq!(tick(next.at(7, 29, 59, 0), next.at(7, 30, 0, 0)), [0]);
    // Jamais réglée : rien.
    set(&core, "input_datetime.reveil", "unknown");
    assert_eq!(
        tick(day.at(7, 29, 59, 0), day.at(7, 30, 0, 0)),
        Vec::<usize>::new()
    );
}

#[test]
fn only_an_input_datetime() {
    let one = |at: &str| {
        parse(&format!(
            "- id: a\n  triggers: [{{trigger: time, at: {at}}}]\n  actions: [{{delay: 1s}}]\n"
        ))
    };
    assert!(one("'07:30'").is_ok());
    assert!(one("input_datetime.reveil").is_ok());
    assert!(one("sensor.heure").is_err());
}
