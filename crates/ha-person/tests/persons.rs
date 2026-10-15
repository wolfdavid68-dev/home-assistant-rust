//! Personnes et zone Maison avec un vrai noyau, et l'automatisation
//! « quand le dernier part ».

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::sync::Arc;
use std::time::Duration;

use ha_core::{Attributes, Core, Event};
use ha_person::{Persons, parse};

const PERSONS: &str = "
- id: david
  name: David
  trackers: [device_tracker.telephone_david, device_tracker.pc_david]
- id: alice
  name: Alice
  trackers: [device_tracker.telephone_alice]
";

fn set(core: &Core, entity: &str, value: &str) {
    core.states
        .set(entity.parse().unwrap(), value, Attributes::new())
        .unwrap();
}

fn state(core: &Core, entity: &str) -> String {
    core.states
        .get(&entity.parse().unwrap())
        .map(|s| s.state().to_owned())
        .unwrap_or_default()
}

async fn settle() {
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn persons_and_the_home_zone_follow_the_devices() {
    let core = Arc::new(Core::new());
    set(&core, "device_tracker.telephone_david", "home");
    set(&core, "device_tracker.pc_david", "not_home");
    set(&core, "device_tracker.telephone_alice", "home");
    let persons = Persons::start(parse(PERSONS).unwrap(), Arc::clone(&core));

    // Dès le démarrage.
    assert_eq!(state(&core, "person.david"), "home");
    assert_eq!(state(&core, "zone.home"), "2");
    let zone = core.states.get(&"zone.home".parse().unwrap()).unwrap();
    assert_eq!(
        zone.attributes()["persons"],
        serde_json::json!(["David", "Alice"])
    );

    // Le téléphone de David part, et son PC est déjà absent : David est parti.
    set(&core, "device_tracker.telephone_david", "not_home");
    settle().await;
    assert_eq!(state(&core, "person.david"), "not_home");
    assert_eq!(state(&core, "zone.home"), "1");
    // Il revient avec seulement son PC : présent.
    set(&core, "device_tracker.pc_david", "home");
    settle().await;
    assert_eq!(state(&core, "person.david"), "home");
    let david = core.states.get(&"person.david".parse().unwrap()).unwrap();
    assert_eq!(david.attributes()["source"], "device_tracker.pc_david");

    persons.stop().await;
}

#[tokio::test]
async fn when_the_last_one_leaves() {
    let automation = ha_automation::parse(
        r#"
- id: dernier_parti
  triggers:
    - trigger: state
      entity_id: zone.home
      to: "0"
  actions:
    - action: light.turn_off
      target: {entity_id: light.salon}
"#,
    )
    .unwrap();
    let core = Arc::new(Core::new());
    let mut rx = core.bus.subscribe();
    set(&core, "device_tracker.telephone_david", "home");
    set(&core, "device_tracker.pc_david", "not_home");
    set(&core, "device_tracker.telephone_alice", "home");
    let engine = ha_automation::Engine::start(automation, Arc::clone(&core));
    let persons = Persons::start(parse(PERSONS).unwrap(), Arc::clone(&core));
    settle().await;
    while rx.try_recv().is_ok() {}

    set(&core, "device_tracker.telephone_alice", "not_home");
    settle().await;
    let calls = |rx: &mut tokio::sync::broadcast::Receiver<Arc<Event>>| {
        let mut calls = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let Event::CallService {
                service, entity_id, ..
            } = event.as_ref()
            {
                calls.push(format!("{service} {entity_id}"));
            }
        }
        calls
    };
    assert_eq!(calls(&mut rx), Vec::<String>::new(), "David est encore là");
    set(&core, "device_tracker.telephone_david", "not_home");
    settle().await;
    assert_eq!(
        calls(&mut rx),
        ["turn_off light.salon"],
        "le dernier est parti"
    );

    persons.stop().await;
    engine.stop().await;
}
