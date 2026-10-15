//! Le journal de bord avec un vrai noyau, une vraie base et le vrai moteur
//! d'automatisations : la porte s'ouvre, l'automatisation éteint la lumière,
//! et le journal dit pourquoi.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ha_automation::Engine;
use ha_core::{Attributes, Core, Event, Origin};
use ha_logbook::Logbook;
use ha_storage::Store;

const AUTOMATIONS: &str = "
- id: porte
  alias: Porte ouverte, lumière éteinte
  triggers:
    - trigger: state
      entity_id: binary_sensor.porte
      to: 'on'
  actions:
    - action: light.turn_off
      target: {entity_id: light.salon}
";

fn set(core: &Core, entity: &str, value: &str, name: &str, device_class: Option<&str>) {
    let mut attributes = Attributes::new();
    attributes.insert("friendly_name".into(), name.into());
    if let Some(class) = device_class {
        attributes.insert("device_class".into(), class.into());
    }
    core.states
        .set(entity.parse().unwrap(), value, attributes)
        .unwrap();
}

async fn settle() {
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test]
async fn the_logbook_tells_what_happened_and_why() {
    let store = Store::open_in_memory().await.unwrap();
    let core = Arc::new(Core::new());
    // Les appareils existent déjà : leur création n'est pas notée.
    set(
        &core,
        "binary_sensor.porte",
        "off",
        "Porte d'entrée",
        Some("door"),
    );
    set(&core, "light.salon", "on", "Lumière du salon", None);
    let logbook = Logbook::start(store.clone(), Arc::clone(&core));
    let engine = Engine::start(
        ha_automation::parse(AUTOMATIONS).unwrap(),
        Arc::clone(&core),
    );
    let mut events = core.bus.subscribe();

    // La porte s'ouvre ; l'automatisation commande la lumière…
    set(
        &core,
        "binary_sensor.porte",
        "on",
        "Porte d'entrée",
        Some("door"),
    );
    let command = loop {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .expect("aucune commande")
            .unwrap();
        if let Event::CallService { origin, .. } = event.as_ref() {
            break origin.clone();
        }
    };
    assert_eq!(
        command,
        Origin::Automation("Porte ouverte, lumière éteinte".into())
    );
    // … et la lumière obéit, comme le ferait le vrai appareil.
    set(&core, "light.salon", "off", "Lumière du salon", None);
    // Plus tard, quelqu'un la rallume depuis le tableau de bord.
    core.call_service_from(
        Origin::User("david".into()),
        "light",
        "turn_on",
        "light.salon".parse().unwrap(),
        Attributes::new(),
    )
    .unwrap();
    settle().await;
    set(&core, "light.salon", "on", "Lumière du salon", None);
    settle().await;

    let lines = store
        .logbook(
            SystemTime::UNIX_EPOCH,
            SystemTime::now() + Duration::from_secs(1),
            None,
            100,
        )
        .await
        .unwrap();
    let text: Vec<String> = lines
        .iter()
        .rev()
        .map(|line| {
            format!(
                "{} {}{}",
                line.name,
                line.message,
                line.cause
                    .as_deref()
                    .map(|cause| format!(" ({cause})"))
                    .unwrap_or_default()
            )
        })
        .collect();
    assert_eq!(
        text,
        [
            "Porte d'entrée s'est ouverte",
            "Porte ouverte, lumière éteinte automatisation lancée \
             (déclencheur : Porte d'entrée passe à « on »)",
            "Lumière du salon s'est éteinte (par l'automatisation « Porte ouverte, lumière éteinte »)",
            "Lumière du salon s'est allumée (par david)",
        ]
    );

    engine.stop().await;
    logbook.stop().await;
}
