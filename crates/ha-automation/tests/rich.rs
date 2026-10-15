//! Automatisations plus riches : conditions « ou » / « non », identifiant
//! de déclencheur, actions `if` / `choose` / `condition`, modes d'exécution.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::sync::Arc;
use std::time::Duration;

use ha_automation::{AutomationConfig, Engine, RunMode, parse};
use ha_core::{Attributes, Core, Event};
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

const RICH: &str = r#"
- id: entree
  alias: Lumière de l'entrée
  mode: restart
  triggers:
    - trigger: state
      entity_id: binary_sensor.porte
      to: "on"
      id: ouverture
    - trigger: state
      entity_id: binary_sensor.porte
      to: "off"
      id: fermeture
  conditions:
    - condition: or
      conditions:
        - condition: state
          entity_id: input.mode
          state: nuit
        - condition: not
          conditions:
            - condition: state
              entity_id: input.mode
              state: absent
  actions:
    - choose:
        - conditions:
            - condition: trigger
              id: ouverture
          sequence:
            - action: light.turn_on
              target: {entity_id: light.entree}
      default:
        - action: light.turn_off
          target: {entity_id: light.entree}
    - if:
        - condition: state
          entity_id: input.mode
          state: nuit
      then:
        - action: light.turn_on
          target: {entity_id: light.couloir}
      else:
        - action: switch.turn_off
          target: {entity_id: switch.veilleuse}
    - condition: state
      entity_id: input.mode
      state: nuit
    - action: notify.send_message
      target: {entity_id: notify.telephone}
      data: {message: "Porte de l'entrée"}
"#;

#[test]
fn rich_automations_are_read_and_rewritten_identically() {
    let automations = parse(RICH).unwrap();
    let automation = &automations[0];
    assert_eq!(automation.mode, RunMode::Restart);
    assert_eq!(automation.trigger_ids, ["ouverture", "fermeture"]);
    let ids: Vec<&str> = automation
        .entity_ids()
        .iter()
        .map(|id| id.as_str())
        .collect();
    assert!(
        ids.contains(&"light.couloir") && ids.contains(&"input.mode"),
        "{ids:?}"
    );

    // Aller-retour par le YAML et le JSON (l'éditeur du tableau de bord).
    let config = AutomationConfig::from_yaml(RICH).unwrap();
    assert_eq!(
        AutomationConfig::from_yaml(&config.to_yaml()).unwrap(),
        config
    );
    assert_eq!(
        AutomationConfig::from_json(config.to_json()).unwrap(),
        config
    );
    assert_eq!(config.check().unwrap(), *automation);
}

#[test]
fn mistakes_in_rich_automations_are_explained() {
    let base = "- id: a\n  triggers: [{trigger: time, at: '12:00'}]\n";
    let cases = [
        // Action inconnue : le message dit quoi écrire.
        ("  actions: [{sauter: 3}]", "action inconnue"),
        // Faute de frappe dans un `if`.
        (
            "  actions: [{if: [{condition: state, entity_id: a.b, state: x}], than: []}]",
            "than",
        ),
        (
            "  actions: [{if: [], then: [{delay: 1s}]}]",
            "au moins une condition",
        ),
        ("  actions: [{choose: []}]", "au moins une option"),
        (
            "  conditions: [{condition: or, conditions: []}]\n  actions: [{delay: 1s}]",
            "au moins une condition",
        ),
        ("  max: 5\n  actions: [{delay: 1s}]", "queued ou parallel"),
        (
            "  mode: queued\n  max: 50\n  actions: [{delay: 1s}]",
            "entre 1 et 20",
        ),
        ("  mode: toujours\n  actions: [{delay: 1s}]", "toujours"),
    ];
    for (rest, expected) in cases {
        let error = parse(&format!("{base}{rest}")).unwrap_err().to_string();
        assert!(error.contains(expected), "{rest}\n→ {error}");
    }
    // Trop d'imbrication : refusé.
    let mut nested = "{delay: 1s}".to_owned();
    for _ in 0..7 {
        nested = format!("{{if: [{{condition: trigger, id: x}}], then: [{nested}]}}");
    }
    let error = parse(&format!("{base}  actions: [{nested}]"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("imbriqués"), "{error}");
}

#[tokio::test]
async fn choose_if_and_condition_follow_the_situation() {
    let core = Arc::new(Core::new());
    let mut rx = core.bus.subscribe();
    set(&core, "input.mode", "nuit");
    set(&core, "binary_sensor.porte", "off");
    let engine = Engine::start(parse(RICH).unwrap(), Arc::clone(&core));

    // Ouverture, la nuit : allumer l'entrée et le couloir, puis notifier.
    set(&core, "binary_sensor.porte", "on");
    assert_eq!(
        calls_during(&mut rx, Duration::from_millis(300)).await,
        [
            "light.turn_on light.entree",
            "light.turn_on light.couloir",
            "notify.send_message notify.telephone"
        ]
    );
    // Fermeture, en journée : éteindre l'entrée, couper la veilleuse, et la
    // condition « nuit » arrête là : pas de notification.
    set(&core, "input.mode", "jour");
    set(&core, "binary_sensor.porte", "off");
    assert_eq!(
        calls_during(&mut rx, Duration::from_millis(300)).await,
        [
            "light.turn_off light.entree",
            "switch.turn_off switch.veilleuse"
        ]
    );
    // Absent : ni « nuit », ni « pas absent » → conditions fausses, rien.
    set(&core, "input.mode", "absent");
    set(&core, "binary_sensor.porte", "on");
    assert_eq!(
        calls_during(&mut rx, Duration::from_millis(300)).await,
        Vec::<String>::new()
    );
    engine.stop().await;
}

/// Une automatisation qui attend 1 s puis allume, déclenchée deux fois à
/// 400 ms d'écart : que se passe-t-il selon son mode ?
async fn run_twice(mode: &str) -> Vec<(String, u128)> {
    let yaml = format!(
        "- id: a\n  mode: {mode}\n  triggers: [{{trigger: state, entity_id: input.bouton}}]\n  \
         actions: [{{delay: 1s}}, {{action: light.turn_on, target: {{entity_id: light.a}}}}]"
    );
    let core = Arc::new(Core::new());
    let mut rx = core.bus.subscribe();
    let engine = Engine::start(parse(&yaml).unwrap(), Arc::clone(&core));
    let start = Instant::now();
    set(&core, "input.bouton", "1");
    tokio::time::sleep(Duration::from_millis(400)).await;
    set(&core, "input.bouton", "2");
    let mut calls = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(2_800);
    while let Ok(Ok(event)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        if let Event::CallService { service, .. } = event.as_ref() {
            calls.push((service.clone(), start.elapsed().as_millis()));
        }
    }
    engine.stop().await;
    calls
}

/// Concept : `start_paused` met l'horloge de tokio en pause. Quand toutes
/// les tâches attendent, tokio avance directement au prochain réveil : les
/// durées sont exactes, et le test ne dépend pas de la vitesse de la machine.
#[tokio::test(start_paused = true)]
async fn run_modes_like_home_assistant() {
    let (single, restart, queued, parallel) = tokio::join!(
        run_twice("single"),
        run_twice("restart"),
        run_twice("queued"),
        run_twice("parallel"),
    );
    let times = |calls: &[(String, u128)]| calls.iter().map(|(_, t)| *t).collect::<Vec<_>>();

    // single : le 2e déclenchement est ignoré → un seul appel, à 1 s.
    assert_eq!(times(&single), [1_000], "{single:?}");
    // restart : la 1re exécution est arrêtée → un seul appel, à 1,4 s.
    assert_eq!(times(&restart), [1_400], "{restart:?}");
    // queued : la 2e attend la fin de la 1re → à 1 s puis à 2 s.
    assert_eq!(times(&queued), [1_000, 2_000], "{queued:?}");
    // parallel : les deux en même temps → à 1 s et à 1,4 s.
    assert_eq!(times(&parallel), [1_000, 1_400], "{parallel:?}");
}

#[tokio::test]
async fn scripts_can_use_conditions_too() {
    let scripts = ha_automation::parse_scripts(
        r"
- id: bonsoir
  sequence:
    - if:
        - condition: state
          entity_id: input.mode
          state: nuit
      then:
        - action: light.turn_on
          target: {entity_id: light.salon}
      else:
        - action: light.turn_off
          target: {entity_id: light.salon}
",
    )
    .unwrap();
    let core = Arc::new(Core::new());
    let mut rx = core.bus.subscribe();
    set(&core, "input.mode", "nuit");
    let engine = Engine::start_with(
        ha_automation::Setup {
            scripts,
            ..ha_automation::Setup::default()
        },
        Arc::clone(&core),
    );
    core.call_service(
        "script",
        "turn_on",
        "script.bonsoir".parse().unwrap(),
        Attributes::new(),
    )
    .unwrap();
    let calls = calls_during(&mut rx, Duration::from_millis(300)).await;
    assert!(
        calls.contains(&"light.turn_on light.salon".to_owned()),
        "{calls:?}"
    );
    engine.stop().await;
}
