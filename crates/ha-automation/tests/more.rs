//! Encore plus riches : déclencheur `time_pattern`, actions `repeat` et
//! `wait_for_trigger`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::sync::Arc;
use std::time::Duration;

use ha_automation::evaluator::Evaluator;
use ha_automation::{Action, Engine, Pattern, RepeatUntil, Trigger, parse};
use ha_core::{Attributes, Core, Event};
use jiff::civil::date;
use tokio::sync::broadcast::Receiver;
use tokio::time::Instant;

fn set(core: &Core, entity: &str, value: &str) {
    core.states
        .set(entity.parse().unwrap(), value, Attributes::new())
        .unwrap();
}

/// Tous les appels de service publiés pendant `wait` : « service entité ».
async fn calls_during(rx: &mut Receiver<Arc<Event>>, wait: Duration) -> Vec<String> {
    let deadline = Instant::now() + wait;
    let mut calls = Vec::new();
    while let Ok(Ok(event)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        if let Event::CallService {
            domain,
            service,
            entity_id,
            ..
        } = event.as_ref()
        {
            calls.push(format!("{domain}.{service} {entity_id}"));
        }
    }
    calls
}

fn one(yaml_trigger: &str) -> Result<Trigger, String> {
    let yaml = format!("- id: a\n  triggers: [{yaml_trigger}]\n  actions: [{{delay: 1s}}]\n");
    parse(&yaml)
        .map(|mut automations| automations.remove(0).triggers.remove(0))
        .map_err(|error| error.to_string())
}

#[test]
fn time_patterns_are_read_like_home_assistant() {
    assert_eq!(
        one(r#"{trigger: time_pattern, minutes: "/15"}"#),
        Ok(Trigger::TimePattern {
            hours: Pattern::Any,
            minutes: Pattern::Every(15),
            second: 0
        })
    );
    // Sans guillemets, un nombre est accepté ; les minutes valent alors 0.
    assert_eq!(
        one("{trigger: time_pattern, hours: 7}"),
        Ok(Trigger::TimePattern {
            hours: Pattern::At(7),
            minutes: Pattern::At(0),
            second: 0
        })
    );
    assert_eq!(
        one(r#"{trigger: time_pattern, hours: "*", minutes: 30, seconds: 10}"#),
        Ok(Trigger::TimePattern {
            hours: Pattern::Any,
            minutes: Pattern::At(30),
            second: 10
        })
    );
    for bad in [
        "{trigger: time_pattern}",
        r#"{trigger: time_pattern, minutes: "/0"}"#,
        "{trigger: time_pattern, minutes: 60}",
        "{trigger: time_pattern, hours: 24}",
        r#"{trigger: time_pattern, minutes: "toutes"}"#,
        r#"{trigger: time_pattern, seconds: "/5"}"#,
        r#"{trigger: time_pattern, seconds: "*"}"#,
        "{trigger: time_pattern, minutes: 5, jours: 1}",
    ] {
        assert!(one(bad).is_err(), "{bad}");
    }
}

#[test]
fn every_fifteen_minutes() {
    let yaml = r#"
- id: quart
  triggers: [{trigger: time_pattern, minutes: "/15"}]
  actions: [{delay: 1s}]
- id: deux_heures
  triggers: [{trigger: time_pattern, hours: "/2"}]
  actions: [{delay: 1s}]
"#;
    let core = Core::new();
    let mut evaluator = Evaluator::new(parse(yaml).unwrap());
    let day = date(2026, 10, 15);
    let now = Instant::now();
    let mut tick = |a, b| evaluator.on_tick(a, b, now, &core.states);

    assert_eq!(
        tick(day.at(10, 14, 58, 0), day.at(10, 14, 59, 0)),
        Vec::<usize>::new()
    );
    assert_eq!(tick(day.at(10, 14, 59, 0), day.at(10, 15, 0, 0)), [0]);
    assert_eq!(
        tick(day.at(10, 15, 0, 0), day.at(10, 15, 1, 0)),
        Vec::<usize>::new(),
        "une seule fois"
    );
    // Une seconde sautée (machine chargée) : rattrapé, une fois.
    assert_eq!(
        tick(day.at(10, 29, 59, 500_000_000), day.at(10, 30, 1, 0)),
        [0]
    );
    // 13 h : impair, rien ; 14 h : les deux.
    assert_eq!(tick(day.at(12, 59, 59, 0), day.at(13, 0, 0, 0)), [0]);
    assert_eq!(tick(day.at(13, 59, 59, 0), day.at(14, 0, 0, 0)), [0, 1]);
    // Passage de minuit.
    let next = date(2026, 10, 16);
    assert_eq!(tick(day.at(23, 59, 59, 0), next.at(0, 0, 0, 0)), [0, 1]);
}

#[test]
fn repeat_and_wait_are_checked() {
    let actions = |yaml: &str| {
        parse(&format!(
            "- id: a\n  triggers: [{{trigger: time, at: '07:00'}}]\n  actions:\n{yaml}"
        ))
        .map(|mut automations| automations.remove(0).actions)
        .map_err(|error| error.to_string())
    };
    let repeat = actions(
        "    - repeat:\n        count: 3\n        sequence: [{action: light.toggle, target: {entity_id: light.a}}]\n",
    )
    .unwrap();
    assert!(matches!(
        &repeat[0],
        Action::Repeat { until: RepeatUntil::Count(3), actions } if actions.len() == 1
    ));
    let wait = actions(
        "    - wait_for_trigger: [{trigger: state, entity_id: binary_sensor.porte, to: 'off'}]\n      timeout: 5min\n",
    )
    .unwrap();
    assert!(matches!(
        &wait[0],
        Action::WaitForTrigger { triggers, timeout: Some(t), continue_on_timeout: true }
            if triggers.len() == 1 && *t == Duration::from_mins(5)
    ));

    for bad in [
        // Ni count, ni while, ni until ; ou deux à la fois.
        "    - repeat: {sequence: [{delay: 1s}]}\n",
        "    - repeat: {count: 2, until: [{condition: state, entity_id: light.a, state: 'on'}], sequence: [{delay: 1s}]}\n",
        "    - repeat: {count: 0, sequence: [{delay: 1s}]}\n",
        "    - repeat: {count: 101, sequence: [{delay: 1s}]}\n",
        "    - repeat: {count: 2, sequence: []}\n",
        "    - repeat: {while: [], sequence: [{delay: 1s}]}\n",
        "    - wait_for_trigger: []\n",
        "    - wait_for_trigger: [{trigger: sun, event: sunset}]\n",
        "    - wait_for_trigger: [{trigger: time, at: '07:00'}]\n      timeout: 2 jours\n",
        "    - wait_for_trigger: [{trigger: time, at: '07:00'}]\n      delai: 5min\n",
    ] {
        assert!(actions(bad).is_err(), "{bad}");
    }
}

#[tokio::test(start_paused = true)]
async fn repeat_count_while_and_until() {
    let yaml = r#"
- id: clignote
  triggers: [{trigger: state, entity_id: input_button.test}]
  actions:
    - repeat:
        count: 3
        sequence:
          - action: light.toggle
            target: {entity_id: light.entree}
          - delay: 1s
- id: tant_que
  triggers: [{trigger: state, entity_id: input_button.tant_que}]
  actions:
    - repeat:
        while:
          - condition: state
            entity_id: input_boolean.alarme
            state: "on"
        sequence:
          - action: notify.send_message
            target: {entity_id: notify.telephone}
          - delay: 10s
- id: sans_fin
  triggers: [{trigger: state, entity_id: input_button.sans_fin}]
  actions:
    - repeat:
        until:
          - condition: state
            entity_id: input_boolean.jamais
            state: "on"
        sequence:
          - action: switch.toggle
            target: {entity_id: switch.prise}
"#;
    let core = Arc::new(Core::new());
    set(&core, "input_boolean.alarme", "on");
    for button in ["test", "tant_que", "sans_fin"] {
        set(&core, &format!("input_button.{button}"), "unknown");
    }
    let engine = Engine::start(parse(yaml).unwrap(), Arc::clone(&core));
    let mut rx = core.bus.subscribe();

    // count : 3 fois.
    set(&core, "input_button.test", "1");
    let calls = calls_during(&mut rx, Duration::from_secs(10)).await;
    assert_eq!(calls, vec!["light.toggle light.entree"; 3]);

    // while : tant que l'alarme sonne (vérifié avant chaque tour).
    set(&core, "input_button.tant_que", "1");
    let calls = calls_during(&mut rx, Duration::from_secs(25)).await;
    assert_eq!(calls.len(), 3, "à 0, 10 et 20 s");
    set(&core, "input_boolean.alarme", "off");
    let calls = calls_during(&mut rx, Duration::from_mins(1)).await;
    assert_eq!(
        calls,
        Vec::<String>::new(),
        "l'alarme est arrêtée : plus de message"
    );

    // until, jamais vrai : arrêté au bout de 100 tours.
    set(&core, "input_button.sans_fin", "1");
    let calls = calls_during(&mut rx, Duration::from_secs(5)).await;
    assert_eq!(calls.len(), 100);

    engine.stop().await;
}

#[tokio::test(start_paused = true)]
async fn wait_for_the_door_to_close() {
    let yaml = r#"
- id: porte
  alias: Porte du garage
  mode: restart
  triggers: [{trigger: state, entity_id: binary_sensor.garage, to: "on"}]
  actions:
    - action: light.turn_on
      target: {entity_id: light.garage}
    - wait_for_trigger:
        - trigger: state
          entity_id: binary_sensor.garage
          to: "off"
      timeout: 5min
      continue_on_timeout: false
    - action: light.turn_off
      target: {entity_id: light.garage}
"#;
    let core = Arc::new(Core::new());
    set(&core, "binary_sensor.garage", "off");
    let engine = Engine::start(parse(yaml).unwrap(), Arc::clone(&core));
    let mut rx = core.bus.subscribe();

    // La porte s'ouvre puis se referme au bout d'une minute : lumière éteinte.
    set(&core, "binary_sensor.garage", "on");
    assert_eq!(
        calls_during(&mut rx, Duration::from_mins(1)).await,
        ["light.turn_on light.garage"]
    );
    set(&core, "binary_sensor.garage", "off");
    assert_eq!(
        calls_during(&mut rx, Duration::from_secs(1)).await,
        ["light.turn_off light.garage"]
    );

    // Elle reste ouverte plus de 5 minutes : on s'arrête, sans éteindre.
    set(&core, "binary_sensor.garage", "on");
    assert_eq!(
        calls_during(&mut rx, Duration::from_mins(10)).await,
        ["light.turn_on light.garage"]
    );

    engine.stop().await;
}
