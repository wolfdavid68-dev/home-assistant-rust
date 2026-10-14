//! Tests des automatisations : logique de déclenchement avec une horloge
//! simulée, puis le moteur complet branché sur un vrai noyau.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::sync::Arc;
use std::time::Duration;

use ha_automation::evaluator::Evaluator;
use ha_automation::{Engine, parse};
use ha_core::{Attributes, Core, EntityId, Event};
use jiff::civil::date;
use tokio::sync::broadcast::Receiver;
use tokio::time::Instant;

fn id(value: &str) -> EntityId {
    value.parse().unwrap()
}

/// Change un état et renvoie les automatisations déclenchées.
fn change(core: &Core, evaluator: &mut Evaluator, entity: &str, value: &str) -> Vec<usize> {
    let old = core.states.get(&id(entity));
    core.states
        .set(id(entity), value, Attributes::new())
        .unwrap();
    let new = core.states.get(&id(entity));
    evaluator.on_state_changed(&id(entity), old.as_ref(), new.as_ref(), Instant::now())
}

#[test]
fn state_trigger_from_to() {
    let yaml = r#"
- id: porte
  triggers: [{trigger: state, entity_id: binary_sensor.porte, from: "off", to: "on"}]
  actions: [{delay: 1s}]
"#;
    let core = Core::new();
    let mut evaluator = Evaluator::new(parse(yaml).unwrap());

    assert!(
        change(&core, &mut evaluator, "binary_sensor.porte", "on").is_empty(),
        "pas de « from: off »"
    );
    change(&core, &mut evaluator, "binary_sensor.porte", "off");
    assert_eq!(
        change(&core, &mut evaluator, "binary_sensor.porte", "on"),
        [0]
    );
    // Une autre entité ne déclenche rien.
    assert!(change(&core, &mut evaluator, "binary_sensor.fenetre", "on").is_empty());
}

#[test]
fn numeric_state_fires_when_entering_the_range() {
    let yaml = r"
- id: froid
  triggers: [{trigger: numeric_state, entity_id: sensor.temperature, below: 18}]
  actions: [{delay: 1s}]
";
    let core = Core::new();
    let mut evaluator = Evaluator::new(parse(yaml).unwrap());

    assert!(change(&core, &mut evaluator, "sensor.temperature", "19.5").is_empty());
    assert_eq!(
        change(&core, &mut evaluator, "sensor.temperature", "17.9"),
        [0]
    );
    // On y reste : pas de nouveau déclenchement à chaque mesure.
    assert!(change(&core, &mut evaluator, "sensor.temperature", "17.5").is_empty());
    assert!(change(&core, &mut evaluator, "sensor.temperature", "indisponible").is_empty());
    assert_eq!(
        change(&core, &mut evaluator, "sensor.temperature", "16"),
        [0]
    );
}

#[test]
fn time_trigger_including_midnight() {
    let yaml = r#"
- id: matin
  triggers: [{trigger: time, at: ["07:00", "00:00"]}]
  actions: [{delay: 1s}]
"#;
    let core = Core::new();
    let mut evaluator = Evaluator::new(parse(yaml).unwrap());
    let now = Instant::now();
    let day = date(2026, 10, 14);

    let tick = |e: &mut Evaluator, a, b| e.on_tick(a, b, now, &core.states);
    assert!(tick(&mut evaluator, day.at(6, 59, 58, 0), day.at(6, 59, 59, 0)).is_empty());
    assert_eq!(
        tick(&mut evaluator, day.at(6, 59, 59, 0), day.at(7, 0, 0, 0)),
        [0]
    );
    assert!(
        tick(&mut evaluator, day.at(7, 0, 0, 0), day.at(7, 0, 1, 0)).is_empty(),
        "une seule fois"
    );
    // Passage de minuit, même si une seconde a été sautée.
    let next = date(2026, 10, 15);
    assert_eq!(
        tick(&mut evaluator, day.at(23, 59, 59, 0), next.at(0, 0, 1, 0)),
        [0]
    );
}

#[tokio::test(start_paused = true)]
async fn state_trigger_with_for_waits_and_cancels() {
    let yaml = r#"
- id: porte_ouverte
  triggers: [{trigger: state, entity_id: binary_sensor.porte, to: "on", for: 5min}]
  actions: [{delay: 1s}]
"#;
    let core = Core::new();
    let mut evaluator = Evaluator::new(parse(yaml).unwrap());
    let t = date(2026, 10, 14).at(12, 0, 0, 0);

    // Ouverte, puis refermée avant 5 minutes : rien.
    assert!(change(&core, &mut evaluator, "binary_sensor.porte", "on").is_empty());
    tokio::time::advance(Duration::from_mins(2)).await;
    change(&core, &mut evaluator, "binary_sensor.porte", "off");
    tokio::time::advance(Duration::from_mins(10)).await;
    assert!(
        evaluator
            .on_tick(t, t, Instant::now(), &core.states)
            .is_empty()
    );

    // Ouverte et restée ouverte 5 minutes : déclenché.
    change(&core, &mut evaluator, "binary_sensor.porte", "on");
    tokio::time::advance(Duration::from_secs(299)).await;
    assert!(
        evaluator
            .on_tick(t, t, Instant::now(), &core.states)
            .is_empty()
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(evaluator.on_tick(t, t, Instant::now(), &core.states), [0]);
}

#[test]
fn conditions_time_weekday_and_state() {
    let yaml = r#"
- id: nuit
  triggers: [{trigger: time, at: "12:00"}]
  conditions:
    - {condition: time, after: "22:00", before: "06:00", weekday: [ven, sam]}
    - {condition: state, entity_id: light.salon, state: ["off", "unknown"]}
  actions: [{delay: 1s}]
"#;
    let core = Core::new();
    let evaluator = Evaluator::new(parse(yaml).unwrap());
    // Le 16 octobre 2026 est un vendredi.
    let friday = date(2026, 10, 16);
    let thursday = date(2026, 10, 15);

    // light.salon n'existe pas encore : condition d'état fausse.
    assert!(!evaluator.conditions_pass(0, &core.states, friday.at(23, 0, 0, 0)));
    core.states
        .set(id("light.salon"), "off", Attributes::new())
        .unwrap();
    assert!(evaluator.conditions_pass(0, &core.states, friday.at(23, 0, 0, 0)));
    assert!(evaluator.conditions_pass(0, &core.states, friday.at(5, 59, 0, 0)));
    assert!(
        !evaluator.conditions_pass(0, &core.states, friday.at(12, 0, 0, 0)),
        "en journée"
    );
    assert!(
        !evaluator.conditions_pass(0, &core.states, thursday.at(23, 0, 0, 0)),
        "jeudi"
    );
    core.states
        .set(id("light.salon"), "on", Attributes::new())
        .unwrap();
    assert!(
        !evaluator.conditions_pass(0, &core.states, friday.at(23, 0, 0, 0)),
        "lampe allumée"
    );
}

/// Prochain appel de service publié sur le bus (2 s au plus).
async fn next_service_call(rx: &mut Receiver<Arc<Event>>) -> (String, String, String) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let event = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("aucun appel de service")
            .unwrap();
        if let Event::CallService {
            domain,
            service,
            entity_id,
            ..
        } = event.as_ref()
        {
            return (domain.clone(), service.clone(), entity_id.to_string());
        }
    }
}

#[tokio::test]
async fn engine_runs_actions_in_order() {
    let yaml = r#"
- id: arrivee
  triggers: [{trigger: state, entity_id: binary_sensor.porte, to: "on"}]
  actions:
    - action: light.turn_on
      target: {entity_id: [light.entree, light.salon]}
      data: {brightness: 200}
    - delay: 1s
    - action: light.turn_off
      target: {entity_id: light.entree}
"#;
    let core = Arc::new(Core::new());
    let mut rx = core.bus.subscribe();
    let engine = Engine::start(parse(yaml).unwrap(), Arc::clone(&core));

    core.states
        .set(id("binary_sensor.porte"), "on", Attributes::new())
        .unwrap();

    assert_eq!(
        next_service_call(&mut rx).await,
        ("light".into(), "turn_on".into(), "light.entree".into())
    );
    assert_eq!(
        next_service_call(&mut rx).await,
        ("light".into(), "turn_on".into(), "light.salon".into())
    );
    let before = Instant::now();
    assert_eq!(
        next_service_call(&mut rx).await,
        ("light".into(), "turn_off".into(), "light.entree".into())
    );
    assert!(
        before.elapsed() >= Duration::from_millis(900),
        "le délai a été respecté"
    );

    engine.stop().await;
}

#[tokio::test]
async fn runaway_loops_are_stopped() {
    // L'automatisation réagit à chaque changement de la lampe… et la modifie.
    let yaml = r"
- id: boucle
  triggers: [{trigger: state, entity_id: light.a}]
  actions: [{action: light.toggle, target: {entity_id: light.a}}]
";
    let core = Arc::new(Core::new());
    let mut rx = core.bus.subscribe();
    let engine = Engine::start(parse(yaml).unwrap(), Arc::clone(&core));

    // On simule un appareil qui obéit instantanément, 30 fois de suite.
    let mut runs = 0;
    for n in 0..30 {
        core.states
            .set(
                id("light.a"),
                if n % 2 == 0 { "on" } else { "off" },
                Attributes::new(),
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        while let Ok(event) = rx.try_recv() {
            if matches!(event.as_ref(), Event::CallService { .. }) {
                runs += 1;
            }
        }
    }
    assert_eq!(
        runs, 10,
        "la protection coupe après 10 déclenchements par minute"
    );

    engine.stop().await;
}
