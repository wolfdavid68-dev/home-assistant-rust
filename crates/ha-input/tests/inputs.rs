//! Les entrées avec un vrai noyau et le vrai moteur d'automatisations :
//! appels de service, minuteur qui sonne, redémarrage.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::sync::Arc;
use std::time::Duration;

use ha_automation::Engine;
use ha_core::{Attributes, Core, Event};
use ha_input::{Inputs, parse};
use serde_json::{Value, json};

const INPUTS: &str = "
input_boolean:
  mode_vacances:
    name: Mode vacances
  lessive_finie:
input_select:
  mode_maison:
    options: [Normal, Nuit]
input_button:
  bonne_nuit:
timer:
  lessive:
    duration: '00:10:00'
";

/// Quand la lessive est finie, on le note ; « Bonne nuit » passe la maison
/// en mode Nuit, sauf en vacances.
const AUTOMATIONS: &str = "
- id: lessive
  triggers:
    - trigger: event
      event_type: timer.finished
      event_data: {entity_id: timer.lessive}
  actions:
    - action: input_boolean.turn_on
      target: {entity_id: input_boolean.lessive_finie}
- id: bonne_nuit
  triggers:
    - trigger: state
      entity_id: input_button.bonne_nuit
  conditions:
    - condition: state
      entity_id: input_boolean.mode_vacances
      state: 'off'
  actions:
    - action: input_select.select_option
      target: {entity_id: input_select.mode_maison}
      data: {option: Nuit}
";

fn state(core: &Core, entity: &str) -> String {
    core.states
        .get(&entity.parse().unwrap())
        .map(|s| s.state().to_owned())
        .unwrap_or_default()
}

fn call(core: &Core, entity: &str, service: &str, data: Value) {
    let Value::Object(data) = data else {
        panic!("un objet est attendu")
    };
    let entity_id: ha_core::EntityId = entity.parse().unwrap();
    let domain = entity_id.domain().to_owned();
    core.call_service(&domain, service, entity_id, data)
        .unwrap();
}

async fn settle() {
    tokio::time::sleep(Duration::from_millis(50)).await;
}

/// Comme le serveur : les entrées sont créées avant les automatisations, et
/// démarrées après.
fn start(core: &Arc<Core>) -> (Engine, Inputs) {
    let prepared = Inputs::prepare(parse(INPUTS).unwrap(), core);
    let engine = Engine::start(ha_automation::parse(AUTOMATIONS).unwrap(), Arc::clone(core));
    (engine, prepared.start(Arc::clone(core)))
}

#[tokio::test(start_paused = true)]
async fn a_finished_timer_triggers_an_automation() {
    let core = Arc::new(Core::new());
    let mut events = core.bus.subscribe();
    let (engine, inputs) = start(&core);
    assert_eq!(state(&core, "timer.lessive"), "idle");

    call(&core, "timer.lessive", "start", json!({}));
    settle().await;
    assert_eq!(state(&core, "timer.lessive"), "active");

    // Une minute avant la fin : rien.
    tokio::time::sleep(Duration::from_mins(9)).await;
    assert_eq!(state(&core, "input_boolean.lessive_finie"), "off");
    // À la fin : le minuteur revient au repos, et l'automatisation note la lessive.
    tokio::time::sleep(Duration::from_secs(61)).await;
    assert_eq!(state(&core, "timer.lessive"), "idle");
    assert_eq!(state(&core, "input_boolean.lessive_finie"), "on");

    let mut fired = Vec::new();
    while let Ok(event) = events.try_recv() {
        // Les événements des minuteurs (pas `automation_triggered`).
        if let Event::Fired { event_type, data } = event.as_ref()
            && event_type.starts_with("timer.")
        {
            fired.push((event_type.clone(), data["entity_id"].clone()));
        }
    }
    assert_eq!(
        fired,
        [
            ("timer.started".to_owned(), json!("timer.lessive")),
            ("timer.finished".to_owned(), json!("timer.lessive")),
        ]
    );

    inputs.stop().await;
    engine.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_cancelled_timer_does_not() {
    let core = Arc::new(Core::new());
    let (engine, inputs) = start(&core);
    call(&core, "timer.lessive", "start", json!({}));
    settle().await;
    call(&core, "timer.lessive", "cancel", json!({}));
    tokio::time::sleep(Duration::from_mins(20)).await;
    assert_eq!(state(&core, "timer.lessive"), "idle");
    assert_eq!(state(&core, "input_boolean.lessive_finie"), "off");
    inputs.stop().await;
    engine.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_button_and_a_condition_on_a_boolean() {
    let core = Arc::new(Core::new());
    let (engine, inputs) = start(&core);

    // En vacances : « Bonne nuit » ne fait rien.
    call(&core, "input_boolean.mode_vacances", "toggle", json!({}));
    call(&core, "input_button.bonne_nuit", "press", json!({}));
    settle().await;
    assert_eq!(state(&core, "input_boolean.mode_vacances"), "on");
    assert_eq!(state(&core, "input_select.mode_maison"), "Normal");

    // De retour : la maison passe en mode Nuit.
    call(&core, "input_boolean.mode_vacances", "turn_off", json!({}));
    tokio::time::sleep(Duration::from_secs(1)).await;
    call(&core, "input_button.bonne_nuit", "press", json!({}));
    settle().await;
    assert_eq!(state(&core, "input_select.mode_maison"), "Nuit");

    // Un appel à une entrée qui n'existe pas, ou au mauvais domaine, ne casse rien.
    call(&core, "input_boolean.inconnue", "toggle", json!({}));
    core.call_service(
        "light",
        "toggle",
        "input_boolean.mode_vacances".parse().unwrap(),
        Attributes::new(),
    )
    .unwrap();
    settle().await;
    assert_eq!(state(&core, "input_boolean.mode_vacances"), "off");

    inputs.stop().await;
    engine.stop().await;
}

#[tokio::test(start_paused = true)]
async fn after_a_restart_values_come_back_and_late_timers_ring() {
    let core = Arc::new(Core::new());
    // L'état d'avant l'arrêt, comme la base le restaure au démarrage : le
    // minuteur devait sonner il y a une minute.
    let mut attributes = Attributes::new();
    let past = jiff::Timestamp::now() - jiff::SignedDuration::from_mins(1);
    attributes.insert("finishes_at".into(), past.to_string().into());
    core.states
        .set("timer.lessive".parse().unwrap(), "active", attributes)
        .unwrap();
    core.states
        .set(
            "input_boolean.mode_vacances".parse().unwrap(),
            "on",
            Attributes::new(),
        )
        .unwrap();

    let (engine, inputs) = start(&core);
    settle().await;
    assert_eq!(state(&core, "input_boolean.mode_vacances"), "on");
    assert_eq!(state(&core, "timer.lessive"), "idle");
    assert_eq!(state(&core, "input_boolean.lessive_finie"), "on");

    inputs.stop().await;
    engine.stop().await;
}

#[test]
fn the_example_files_are_understood() {
    let config = parse(include_str!("../../../inputs.example.yaml")).unwrap();
    assert_eq!(config.helpers.len(), 10);
    let automations =
        ha_automation::parse(include_str!("../../../automations.example.yaml")).unwrap();
    assert!(automations.iter().any(|a| a.id == "lessive_finie"));
}
