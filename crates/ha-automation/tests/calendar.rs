//! Le déclencheur `calendar` : « 30 minutes avant un rendez-vous ».

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use ha_automation::evaluator::Evaluator;
use ha_automation::parse;
use ha_core::Core;
use jiff::civil::date;
use serde_json::{Value, json};
use tokio::time::Instant;

fn calendar(core: &Core, events: &Value) {
    let Value::Object(attributes) = json!({"friendly_name": "Agenda", "events": events}) else {
        panic!("un objet est attendu")
    };
    core.states
        .set("calendar.maison".parse().unwrap(), "off", attributes)
        .unwrap();
}

#[test]
fn half_an_hour_before_and_at_the_end() {
    let yaml = r#"
- id: rappel
  triggers: [{trigger: calendar, event: start, entity_id: calendar.maison, offset: "-00:30:00"}]
  actions: [{delay: 1s}]
- id: fin
  triggers: [{trigger: calendar, event: end, entity_id: calendar.maison}]
  actions: [{delay: 1s}]
"#;
    let core = Core::new();
    calendar(
        &core,
        &json!([{"uid": "1", "summary": "Dentiste",
                "start": "2026-10-15T18:30:00", "end": "2026-10-15T19:00:00"}]),
    );
    let mut evaluator = Evaluator::new(parse(yaml).unwrap());
    let day = date(2026, 10, 15);
    let now = Instant::now();
    let mut tick = |a, b| evaluator.on_tick(a, b, now, &core.states);

    assert_eq!(
        tick(day.at(17, 59, 58, 0), day.at(17, 59, 59, 0)),
        Vec::<usize>::new()
    );
    assert_eq!(tick(day.at(17, 59, 59, 0), day.at(18, 0, 0, 0)), [0]);
    assert_eq!(
        tick(day.at(18, 0, 0, 0), day.at(18, 0, 1, 0)),
        Vec::<usize>::new(),
        "une seule fois"
    );
    assert_eq!(tick(day.at(18, 59, 59, 0), day.at(19, 0, 0, 0)), [1]);

    // Un agenda sans rendez-vous, ou absent : rien.
    calendar(&core, &json!([]));
    assert_eq!(
        tick(day.at(17, 59, 59, 0), day.at(18, 0, 0, 0)),
        Vec::<usize>::new()
    );
    core.states.remove(&"calendar.maison".parse().unwrap());
    assert_eq!(
        tick(day.at(17, 59, 59, 0), day.at(18, 0, 0, 0)),
        Vec::<usize>::new()
    );
}

#[test]
fn calendar_triggers_are_checked() {
    let one = |trigger: &str| {
        parse(&format!(
            "- id: a\n  triggers: [{trigger}]\n  actions: [{{delay: 1s}}]\n"
        ))
        .map_err(|error| error.to_string())
    };
    assert!(one("{trigger: calendar, event: start, entity_id: calendar.maison}").is_ok());
    for bad in [
        // Pas un agenda.
        "{trigger: calendar, event: start, entity_id: light.salon}",
        "{trigger: calendar, event: debut, entity_id: calendar.maison}",
        "{trigger: calendar, entity_id: calendar.maison}",
        // Plus d'une semaine : l'agenda ne publie pas si loin.
        r#"{trigger: calendar, event: start, entity_id: calendar.maison, offset: "-200:00:00"}"#,
    ] {
        assert!(one(bad).is_err(), "{bad}");
    }
}
