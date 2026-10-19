//! Tests de l'étape 64 : les modèles qui calculent, la condition `template`
//! et `trigger.event.data`, dans de vraies automatisations.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::sync::Arc;
use std::time::Duration;

use ha_automation::{Engine, parse};
use ha_core::{Attributes, Core, EntityId, Event};
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

/// Le prochain message envoyé, ou `None` si rien n'arrive.
async fn next_message(rx: &mut Receiver<Arc<Event>>, within: Duration) -> Option<String> {
    let deadline = Instant::now() + within;
    loop {
        let event = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .ok()?
            .ok()?;
        if let Event::CallService {
            domain,
            service,
            data,
            ..
        } = event.as_ref()
            && domain == "notify"
            && service == "send_message"
        {
            return data["message"].as_str().map(str::to_owned);
        }
    }
}

#[tokio::test]
async fn a_template_condition_reads_the_trigger_and_calculates() {
    let yaml = r"
- id: ecart
  triggers:
    - {trigger: state, entity_id: sensor.salon}
  conditions:
    - condition: template
      value_template: >-
        {{ trigger.to_state.state | float(0) - states('sensor.dehors') | float(0) > 10 }}
  actions:
    - action: notify.send_message
      target: {entity_id: notify.appli}
      data:
        message: >-
          Écart de {{ (states('sensor.salon') | float - states('sensor.dehors') | float) | round(1) }} °C,
          {{ 'aère' if states('sensor.dehors') | float > 5 else 'garde fermé' }}.
";
    let core = Arc::new(Core::new());
    core.states
        .set(id("sensor.dehors"), "8", Attributes::new())
        .unwrap();
    core.states
        .set(id("sensor.salon"), "17", Attributes::new())
        .unwrap();
    let mut rx = core.bus.subscribe();
    let engine = Engine::start(parse(yaml).unwrap(), Arc::clone(&core));
    // 18,5 − 8 = 10,5 : au-delà de 10, le message part.
    core.states
        .set(id("sensor.salon"), "18.5", Attributes::new())
        .unwrap();
    assert_eq!(
        next_message(&mut rx, Duration::from_secs(2))
            .await
            .as_deref(),
        Some("Écart de 10.5 °C, aère.")
    );
    // 17,9 − 8 = 9,9 : la condition est fausse, rien ne part.
    core.states
        .set(id("sensor.salon"), "17.9", Attributes::new())
        .unwrap();
    assert_eq!(
        next_message(&mut rx, Duration::from_millis(500)).await,
        None
    );
    engine.stop().await;
}

#[tokio::test]
async fn an_event_gives_its_details_to_the_templates() {
    let yaml = r"
- id: delestage_appli
  triggers:
    - {trigger: event, event_type: delestage, event_data: {action: coupe}}
  actions:
    - if:
        - condition: template
          value_template: '{{ trigger.event.data.mesure > trigger.event.data.limite }}'
      then:
        - action: notify.send_message
          target: {entity_id: notify.appli}
          data:
            message: >-
              {{ trigger.event.data.entity_id }} coupé
              ({{ trigger.event.data.mesure }} A sur {{ trigger.event.data.limite }}).
";
    let core = Arc::new(Core::new());
    let mut rx = core.bus.subscribe();
    let engine = Engine::start(parse(yaml).unwrap(), Arc::clone(&core));
    tokio::time::sleep(Duration::from_millis(50)).await;
    core.fire(
        "delestage",
        attributes(json!({"action": "coupe", "entity_id": "switch.chauffe_eau", "mesure": 33, "limite": 30})),
    )
    .unwrap();
    assert_eq!(
        next_message(&mut rx, Duration::from_secs(2))
            .await
            .as_deref(),
        Some("switch.chauffe_eau coupé (33 A sur 30).")
    );
    engine.stop().await;
}

#[test]
fn a_template_condition_is_checked_when_loading() {
    for (template, hint) in [
        ("il fait chaud", "un modèle {{ … }} attendu"),
        ("{{ states('sensor.x') | float >> 2 }}", "valeur attendue"),
        ("{{ 1 if true }}", "else"),
    ] {
        let yaml = format!(
            "- id: x\n  triggers: [{{trigger: state, entity_id: sensor.x}}]\n  conditions:\n    - condition: template\n      value_template: \"{template}\"\n  actions: [{{delay: 1s}}]\n"
        );
        let error = parse(&yaml).unwrap_err().to_string();
        assert!(error.contains("condition template"), "{template} : {error}");
        assert!(error.contains(hint), "{template} : {error}");
    }
    // Les entités lues par le modèle sont connues (pour l'éditeur).
    let yaml = r#"
- id: x
  triggers: [{trigger: state, entity_id: sensor.salon}]
  conditions:
    - {condition: template, value_template: "{{ is_state('binary_sensor.fenetre', 'off') }}"}
  actions: [{delay: 1s}]
"#;
    let automations = parse(yaml).unwrap();
    let ids: Vec<&str> = automations[0]
        .entity_ids()
        .iter()
        .map(|id| id.as_str())
        .collect();
    assert_eq!(ids, ["binary_sensor.fenetre", "sensor.salon"]);
}
