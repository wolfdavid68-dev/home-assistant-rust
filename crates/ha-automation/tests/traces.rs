//! L'historique des exécutions : raison, conditions, actions, résultat.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::sync::Arc;
use std::time::Duration;

use ha_automation::{Engine, Outcome, Setup, StepStatus, Traces, parse};
use ha_core::{Attributes, Core};

fn set(core: &Core, entity: &str, value: &str) {
    let mut attributes = Attributes::new();
    if entity == "binary_sensor.porte" {
        attributes.insert("friendly_name".into(), "Porte d'entrée".into());
    }
    core.states
        .set(entity.parse().unwrap(), value, attributes)
        .unwrap();
}

const AUTOMATIONS: &str = r#"
- id: entree
  alias: Lumière de l'entrée
  mode: restart
  triggers:
    - trigger: state
      entity_id: binary_sensor.porte
      to: "on"
  conditions:
    - condition: state
      entity_id: input_select.mode
      state: nuit
  actions:
    - if:
        - condition: state
          entity_id: light.entree
          state: "off"
      then:
        - action: light.turn_on
          target: {entity_id: light.entree}
    - condition: state
      entity_id: input_boolean.vacances
      state: "off"
    - delay: "00:00:01"
    - action: light.turn_off
      target: {entity_id: light.entree}
"#;

#[tokio::test]
async fn each_run_is_recorded_step_by_step() {
    let core = Arc::new(Core::new());
    set(&core, "input_select.mode", "jour");
    set(&core, "light.entree", "off");
    set(&core, "input_boolean.vacances", "on");
    set(&core, "binary_sensor.porte", "off");
    let traces = Traces::new();
    let engine = Engine::start_with(
        Setup {
            automations: parse(AUTOMATIONS).unwrap(),
            traces: traces.clone(),
            ..Setup::default()
        },
        Arc::clone(&core),
    );

    // 1. Le jour : la condition est fausse, rien n'est fait.
    set(&core, "binary_sensor.porte", "on");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let list = traces.of("entree");
    assert_eq!(list.len(), 1);
    let trace = &list[0];
    assert_eq!(trace.outcome, Outcome::ConditionsFailed);
    assert_eq!(
        trace.trigger.as_deref(),
        Some("Porte d'entrée passe à « on »")
    );
    assert_eq!(trace.conditions.len(), 1);
    assert_eq!(
        trace.conditions[0].description,
        "input_select.mode est « nuit »"
    );
    assert!(!trace.conditions[0].passed);
    assert_eq!(trace.steps, Vec::new());
    assert!(trace.finished.is_some());

    // 2. La nuit, en vacances : on allume, puis la condition arrête tout.
    set(&core, "input_select.mode", "nuit");
    set(&core, "binary_sensor.porte", "off");
    set(&core, "binary_sensor.porte", "on");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let trace = &traces.of("entree")[0];
    assert_eq!(trace.outcome, Outcome::Stopped);
    let steps: Vec<(&str, StepStatus, Option<&str>)> = trace
        .steps
        .iter()
        .map(|s| (s.path.as_str(), s.status, s.detail.as_deref()))
        .collect();
    assert_eq!(
        steps,
        [
            ("1", StepStatus::Done, Some("alors")),
            ("1 › alors › 1", StepStatus::Done, None),
            ("2", StepStatus::Stopped, Some("fausse : on s'arrête là")),
        ]
    );
    assert_eq!(trace.steps[1].description, "light.turn_on sur light.entree");

    // 3. Hors vacances, et relancée pendant l'attente : la première exécution
    // est interrompue (mode `restart`).
    set(&core, "input_boolean.vacances", "off");
    set(&core, "binary_sensor.porte", "off");
    set(&core, "binary_sensor.porte", "on");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(traces.of("entree")[0].outcome, Outcome::Running);
    set(&core, "binary_sensor.porte", "off");
    set(&core, "binary_sensor.porte", "on");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let list = traces.of("entree");
    assert_eq!(list[1].outcome, Outcome::Aborted);
    assert_eq!(list[0].outcome, Outcome::Running);

    // La seconde va jusqu'au bout.
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    let trace = &traces.of("entree")[0];
    assert_eq!(trace.outcome, Outcome::Finished);
    let paths: Vec<&str> = trace.steps.iter().map(|s| s.path.as_str()).collect();
    assert_eq!(paths, ["1", "1 › alors › 1", "2", "3", "4"]);
    assert_eq!(trace.steps[3].description, "attendre 1 s");
    assert!(trace.run > list[1].run);

    engine.stop().await;
}

#[tokio::test]
async fn a_loop_is_visible_in_the_history() {
    let core = Arc::new(Core::new());
    set(&core, "input_boolean.bouton", "off");
    let traces = Traces::new();
    let yaml = r"
- id: boucle
  alias: Boucle
  mode: parallel
  max: 20
  triggers:
    - trigger: state
      entity_id: input_boolean.bouton
  actions:
    - delay: 00:00:05
";
    let engine = Engine::start_with(
        Setup {
            automations: parse(yaml).unwrap(),
            traces: traces.clone(),
            ..Setup::default()
        },
        Arc::clone(&core),
    );
    for round in 0..11 {
        let value = if round % 2 == 0 { "on" } else { "off" };
        set(&core, "input_boolean.bouton", value);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let list = traces.of("boucle");
    assert_eq!(list[0].outcome, Outcome::Loop);
    // Et le tableau de bord prévient.
    let notice = core
        .states
        .get(
            &"persistent_notification.automation_loop_boucle"
                .parse()
                .unwrap(),
        )
        .expect("une notification est attendue");
    assert_eq!(notice.attributes()["title"], "Automatisation en boucle");
    engine.stop().await;
    // Le serveur s'arrête : les exécutions en cours sont « interrompues ».
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        traces
            .of("boucle")
            .iter()
            .skip(1)
            .all(|t| t.outcome == Outcome::Aborted)
    );
}

#[tokio::test(start_paused = true)]
async fn unknown_entities_raise_a_repair_until_they_appear() {
    let core = Arc::new(Core::new());
    set(&core, "binary_sensor.porte", "off");
    set(&core, "input_select.mode", "jour");
    let engine = Engine::start(parse(AUTOMATIONS).unwrap(), Arc::clone(&core));
    let repair: ha_core::EntityId = "persistent_notification.repair_automation_entree"
        .parse()
        .unwrap();

    // Pas tout de suite : les appareils ont le temps de se connecter.
    tokio::time::sleep(Duration::from_mins(1)).await;
    assert!(core.states.get(&repair).is_none());
    tokio::time::sleep(Duration::from_secs(65)).await;
    let state = core.states.get(&repair).expect("une alerte est attendue");
    let message = state.attributes()["message"].as_str().unwrap();
    assert!(
        message.starts_with("Ces entités n'existent pas : input_boolean.vacances, light.entree."),
        "{message}"
    );
    assert_eq!(state.attributes()["severity"], "error");

    // Les appareils arrivent : l'alerte disparaît à la vérification suivante.
    set(&core, "light.entree", "off");
    set(&core, "input_boolean.vacances", "off");
    tokio::time::sleep(Duration::from_secs(5 * 60 + 2)).await;
    assert!(core.states.get(&repair).is_none());
    engine.stop().await;
}
