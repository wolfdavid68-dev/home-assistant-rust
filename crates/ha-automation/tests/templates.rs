//! Tests de l'étape 34 : déclencheurs sur un attribut, listes de valeurs,
//! `not_from` / `not_to`, et modèles (`{{ … }}`) dans les actions.

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
use serde_json::{Value, json};
use tokio::sync::broadcast::Receiver;
use tokio::time::Instant;

fn id(value: &str) -> EntityId {
    value.parse().unwrap()
}

fn attributes(value: Value) -> Attributes {
    let Value::Object(map) = value else { panic!() };
    map
}

/// Change un état (et ses attributs) et renvoie les automatisations déclenchées.
fn change(
    core: &Core,
    evaluator: &mut Evaluator,
    entity: &str,
    value: &str,
    attrs: Value,
) -> Vec<usize> {
    let old = core.states.get(&id(entity));
    core.states
        .set(id(entity), value, attributes(attrs))
        .unwrap();
    let new = core.states.get(&id(entity));
    evaluator.on_state_changed(&id(entity), old.as_deref(), new.as_deref(), Instant::now())
}

const NONE: [usize; 0] = [];

#[test]
fn lists_and_exclusions() {
    let yaml = r"
- id: vigilance
  triggers:
    - {trigger: state, entity_id: sensor.vigilance_meteo, to: [Orange, Rouge]}
  actions: [{delay: 1s}]
- id: sortie_de_panne
  triggers:
    - {trigger: state, entity_id: sensor.vigilance_meteo, not_from: [unavailable, unknown], not_to: Vert}
  actions: [{delay: 1s}]
";
    let core = Core::new();
    let mut evaluator = Evaluator::new(parse(yaml).unwrap());
    let mut set = |value: &str| {
        change(
            &core,
            &mut evaluator,
            "sensor.vigilance_meteo",
            value,
            json!({}),
        )
    };
    // Première valeur : l'ancienne est absente, `not_from` ne l'exclut pas.
    assert_eq!(set("unavailable"), [1]);
    // Depuis « unavailable » : exclu par not_from.
    assert_eq!(set("Jaune"), NONE);
    assert_eq!(set("Orange"), [0, 1]);
    assert_eq!(set("Rouge"), [0, 1]);
    // Vers « Vert » : exclu par not_to.
    assert_eq!(set("Vert"), NONE);
    assert_eq!(set("Jaune"), [1]);
}

#[test]
fn an_attribute_can_be_watched() {
    let yaml = r"
- id: vent
  triggers:
    - trigger: state
      entity_id: sensor.vigilance_meteo
      attribute: Vent violent
      to: [Orange, Rouge]
  conditions:
    - {condition: state, entity_id: sensor.vigilance_meteo, attribute: niveau_actuel, state: [Orange, Rouge]}
  actions: [{delay: 1s}]
- id: luminosite
  triggers:
    - {trigger: state, entity_id: light.salon, attribute: brightness, to: 255}
  actions: [{delay: 1s}]
";
    let core = Core::new();
    let mut evaluator = Evaluator::new(parse(yaml).unwrap());
    let vigilance = |core: &Core, evaluator: &mut Evaluator, state: &str, wind: &str| {
        change(
            core,
            evaluator,
            "sensor.vigilance_meteo",
            state,
            json!({"Vent violent": wind, "niveau_actuel": state}),
        )
    };
    assert_eq!(vigilance(&core, &mut evaluator, "Jaune", "Jaune"), NONE);
    // L'état ne change pas, l'attribut si : déclenché.
    assert_eq!(vigilance(&core, &mut evaluator, "Jaune", "Orange"), [0]);
    // L'état change, pas l'attribut : rien.
    assert_eq!(vigilance(&core, &mut evaluator, "Orange", "Orange"), NONE);
    assert_eq!(vigilance(&core, &mut evaluator, "Orange", "Jaune"), NONE);
    // La condition lit aussi l'attribut.
    let now = date(2026, 10, 15).at(12, 0, 0, 0);
    assert!(evaluator.conditions_pass(0, &core.states, now));
    vigilance(&core, &mut evaluator, "Jaune", "Jaune");
    assert!(!evaluator.conditions_pass(0, &core.states, now));

    // Un nombre s'écrit sans guillemets.
    let mut light = |level: i64| {
        change(
            &core,
            &mut evaluator,
            "light.salon",
            "on",
            json!({"brightness": level}),
        )
    };
    assert_eq!(light(128), NONE);
    assert_eq!(light(255), [1]);
}

#[tokio::test(start_paused = true)]
async fn a_wait_is_not_cancelled_by_another_attribute() {
    let yaml = r#"
- id: porte_ouverte
  triggers: [{trigger: state, entity_id: binary_sensor.porte, to: "on", for: 5min}]
  actions: [{delay: 1s}]
"#;
    let core = Core::new();
    let mut evaluator = Evaluator::new(parse(yaml).unwrap());
    let t = date(2026, 10, 15).at(12, 0, 0, 0);
    change(
        &core,
        &mut evaluator,
        "binary_sensor.porte",
        "on",
        json!({"batterie": 90}),
    );
    tokio::time::advance(Duration::from_mins(2)).await;
    // Seule la batterie change : l'attente continue.
    change(
        &core,
        &mut evaluator,
        "binary_sensor.porte",
        "on",
        json!({"batterie": 89}),
    );
    tokio::time::advance(Duration::from_mins(3)).await;
    assert_eq!(evaluator.on_tick(t, t, Instant::now(), &core.states), [0]);
}

#[test]
fn mistakes_are_explained() {
    for (yaml, hint) in [
        (
            "{trigger: state, entity_id: light.a, to: on}",
            "entre guillemets",
        ),
        (
            "{trigger: state, entity_id: light.a, to: [\"on\", off]}",
            "entre guillemets",
        ),
        (
            "{trigger: state, entity_id: light.a, attribute: \"\"}",
            "attribute",
        ),
    ] {
        let file = format!("- id: x\n  triggers: [{yaml}]\n  actions: [{{delay: 1s}}]\n");
        let error = parse(&file).unwrap_err().to_string();
        assert!(error.contains(hint), "{yaml} : {error}");
    }
    let bad_template = r#"
- id: x
  triggers: [{trigger: state, entity_id: light.a}]
  actions:
    - action: notify.send_message
      target: {entity_id: notify.telephone}
      data: {message: "{{ states(light.a) }}"}
"#;
    let error = parse(bad_template).unwrap_err().to_string();
    assert!(error.contains("data.message"), "{error}");
}

/// Le prochain appel de service publié sur le bus, avec ses données.
async fn next_call(rx: &mut Receiver<Arc<Event>>) -> (String, Attributes) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let event = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("aucun appel de service")
            .unwrap();
        if let Event::CallService {
            domain,
            service,
            data,
            ..
        } = event.as_ref()
        {
            return (format!("{domain}.{service}"), data.clone());
        }
    }
}

#[tokio::test]
async fn templates_are_filled_in_when_the_action_runs() {
    let yaml = r#"
- id: vigilance_telephone
  triggers:
    - {trigger: state, entity_id: sensor.vigilance_meteo, to: [Orange, Rouge], id: grave}
  actions:
    - action: notify.send_message
      target: {entity_id: notify.telephone}
      data:
        title: "Vigilance {{ trigger.to_state.state | lower }}"
        message: >-
          {{ trigger.to_state.name }} : {{ trigger.from_state.state | lower }} →
          {{ trigger.to_state.state | lower }}. Dehors :
          {{ states('sensor.meteo_temperature') | round(1) }} °C.
        priority: 5
        tags: ["{{ trigger.id }}", fixe]
    - action: light.turn_on
      target: {entity_id: light.salon}
      data: {brightness: "{{ state_attr('light.salon', 'brightness') | int(0) }}"}
"#;
    let core = Arc::new(Core::new());
    core.states
        .set(id("sensor.meteo_temperature"), "18.46", Attributes::new())
        .unwrap();
    core.states
        .set(
            id("light.salon"),
            "on",
            attributes(json!({"brightness": 200})),
        )
        .unwrap();
    core.states
        .set(
            id("sensor.vigilance_meteo"),
            "Jaune",
            attributes(json!({"friendly_name": "Vigilance météo"})),
        )
        .unwrap();
    let mut rx = core.bus.subscribe();
    let engine = Engine::start(parse(yaml).unwrap(), Arc::clone(&core));
    core.states
        .set(
            id("sensor.vigilance_meteo"),
            "Orange",
            attributes(json!({"friendly_name": "Vigilance météo"})),
        )
        .unwrap();

    let (service, data) = next_call(&mut rx).await;
    assert_eq!(service, "notify.send_message");
    assert_eq!(data["title"], "Vigilance orange");
    assert_eq!(
        data["message"],
        "Vigilance météo : jaune → orange. Dehors : 18.5 °C."
    );
    assert_eq!(data["priority"], 5);
    assert_eq!(data["tags"], json!(["grave", "fixe"]));
    // Une seule expression qui donne un nombre : un nombre.
    let (service, data) = next_call(&mut rx).await;
    assert_eq!(service, "light.turn_on");
    assert_eq!(data["brightness"], 200);
    engine.stop().await;
}

#[test]
fn template_entities_are_listed() {
    let yaml = r#"
- id: x
  triggers: [{trigger: state, entity_id: light.a}]
  actions:
    - action: notify.send_message
      target: {entity_id: notify.telephone}
      data: {message: "{{ states('sensor.inconnu') }}"}
"#;
    let automations = parse(yaml).unwrap();
    let ids: Vec<&str> = automations[0]
        .entity_ids()
        .iter()
        .map(|id| id.as_str())
        .collect();
    assert_eq!(ids, ["light.a", "notify.telephone", "sensor.inconnu"]);
}
