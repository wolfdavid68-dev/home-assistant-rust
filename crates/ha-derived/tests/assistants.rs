//! Les assistants calculés pour de vrai, avec le bus d'événements.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::sync::Arc;
use std::time::Duration;

use ha_core::{Attributes, Core, Event, Origin};
use ha_derived::{Assistants, parse};
use serde_json::{Value, json};

fn set(core: &Core, entity: &str, value: &str, attributes: &Value) {
    let Value::Object(attributes) = attributes.clone() else {
        panic!("un objet est attendu")
    };
    core.states
        .set(entity.parse().unwrap(), value, attributes)
        .unwrap();
}

fn state(core: &Core, entity: &str) -> Option<String> {
    core.states
        .get(&entity.parse().unwrap())
        .map(|s| s.state().to_owned())
}

/// Attend (2 s au plus) que l'entité ait cette valeur.
async fn wait_for(core: &Core, entity: &str, value: &str) {
    for _ in 0..200 {
        if state(core, entity).as_deref() == Some(value) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "{entity} : attendu {value:?}, obtenu {:?}",
        state(core, entity)
    );
}

const YAML: &str = "group:\n  bas: {name: Lumières du bas, type: light, entities: [light.salon, light.cuisine]}\n\
    min_max:\n  moyenne: {type: mean, entities: [sensor.salon, sensor.chambre], round_digits: 1}\n\
    threshold:\n  humide: {entity_id: sensor.humidite, upper: 70, hysteresis: 2}\n\
    utility_meter:\n  jour: {source: sensor.compteur, cycle: daily}\n";

#[tokio::test]
async fn groups_averages_thresholds_and_meters() {
    let core = Arc::new(Core::new());
    set(&core, "light.salon", "off", &json!({}));
    set(&core, "light.cuisine", "off", &json!({}));
    set(
        &core,
        "sensor.salon",
        "20",
        &json!({"unit_of_measurement": "°C"}),
    );
    set(
        &core,
        "sensor.chambre",
        "18.5",
        &json!({"unit_of_measurement": "°C"}),
    );
    let assistants = Assistants::start(parse(YAML).unwrap(), Arc::clone(&core));

    // Le groupe suit ses membres.
    assert_eq!(state(&core, "light.bas").as_deref(), Some("off"));
    set(&core, "light.cuisine", "on", &json!({}));
    wait_for(&core, "light.bas", "on").await;

    // Une commande sur le groupe va à chaque membre, au nom de la même personne.
    let mut events = core.bus.subscribe();
    core.call_service_from(
        Origin::User("david".into()),
        "light",
        "turn_off",
        "light.bas".parse().unwrap(),
        Attributes::new(),
    )
    .unwrap();
    let mut forwarded = Vec::new();
    while forwarded.len() < 2 {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let Event::CallService {
            entity_id,
            service,
            origin,
            ..
        } = event.as_ref()
            && entity_id.as_str() != "light.bas"
        {
            assert_eq!(service, "turn_off");
            assert_eq!(origin, &Origin::User("david".into()));
            forwarded.push(entity_id.to_string());
        }
    }
    forwarded.sort();
    assert_eq!(forwarded, ["light.cuisine", "light.salon"]);

    // La moyenne, avec l'unité des capteurs.
    assert_eq!(state(&core, "sensor.moyenne").as_deref(), Some("19.2"));
    set(
        &core,
        "sensor.chambre",
        "21",
        &json!({"unit_of_measurement": "°C"}),
    );
    wait_for(&core, "sensor.moyenne", "20.5").await;
    let average = core.states.get(&"sensor.moyenne".parse().unwrap()).unwrap();
    assert_eq!(average.attributes()["unit_of_measurement"], "°C");

    // Le seuil, avec sa marge.
    set(&core, "sensor.humidite", "71", &json!({}));
    wait_for(&core, "binary_sensor.humide", "off").await;
    set(&core, "sensor.humidite", "73", &json!({}));
    wait_for(&core, "binary_sensor.humide", "on").await;
    set(&core, "sensor.humidite", "69", &json!({}));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        state(&core, "binary_sensor.humide").as_deref(),
        Some("on"),
        "dans la marge : on garde"
    );

    // Le compteur du jour : la première lecture sert de référence.
    set(
        &core,
        "sensor.compteur",
        "1000",
        &json!({"unit_of_measurement": "kWh", "device_class": "energy"}),
    );
    set(
        &core,
        "sensor.compteur",
        "1001.25",
        &json!({"unit_of_measurement": "kWh", "device_class": "energy"}),
    );
    wait_for(&core, "sensor.jour", "1.25").await;
    assistants.stop().await;

    // Après un redémarrage, il reprend où il en était.
    let assistants = Assistants::start(parse(YAML).unwrap(), Arc::clone(&core));
    set(
        &core,
        "sensor.compteur",
        "1002",
        &json!({"unit_of_measurement": "kWh"}),
    );
    wait_for(&core, "sensor.jour", "2").await;
    let meter = core.states.get(&"sensor.jour".parse().unwrap()).unwrap();
    assert_eq!(meter.attributes()["unit_of_measurement"], "kWh");
    assert_eq!(meter.attributes()["source"], "sensor.compteur");
    assistants.stop().await;
}

#[test]
fn the_example_file_is_understood() {
    let derived = parse(include_str!("../../../derived.example.yaml")).unwrap();
    assert_eq!(derived.len(), 13);
}

#[tokio::test]
async fn an_integral_and_a_derivative() {
    let core = Arc::new(Core::new());
    // 3 600 000 W pendant une seconde font 1 kWh : le test reste court.
    set(
        &core,
        "sensor.puissance",
        "3600000",
        &json!({"unit_of_measurement": "W", "device_class": "power"}),
    );
    set(
        &core,
        "sensor.chambre",
        "20",
        &json!({"unit_of_measurement": "°C"}),
    );
    let yaml = "integration:\n  energie: {source: sensor.puissance, unit_prefix: k, round: 0}\n\
        derivative:\n  variation: {source: sensor.chambre, unit_time: s, time_window: \"00:01:00\", round: 0}\n";
    let assistants = Assistants::start(parse(yaml).unwrap(), Arc::clone(&core));
    wait_for(&core, "sensor.energie", "0").await;
    let energy = core.states.get(&"sensor.energie".parse().unwrap()).unwrap();
    assert_eq!(energy.attributes()["unit_of_measurement"], "kWh");
    assert_eq!(energy.attributes()["device_class"], "energy");
    tokio::time::sleep(Duration::from_millis(1100)).await;
    // La puissance tombe : la seconde passée à 3,6 MW est comptée.
    set(
        &core,
        "sensor.puissance",
        "0",
        &json!({"unit_of_measurement": "W", "device_class": "power"}),
    );
    wait_for(&core, "sensor.energie", "1").await;
    // +1 °C en un peu plus d'une seconde : environ 1 °C/s.
    set(
        &core,
        "sensor.chambre",
        "21",
        &json!({"unit_of_measurement": "°C"}),
    );
    wait_for(&core, "sensor.variation", "1").await;
    let slope = core
        .states
        .get(&"sensor.variation".parse().unwrap())
        .unwrap();
    assert_eq!(slope.attributes()["unit_of_measurement"], "°C/s");
    assistants.stop().await;
}
