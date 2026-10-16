//! Le thermostat complet, branché sur un vrai noyau, avec un radiateur
//! simulé qui obéit aux ordres.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::sync::Arc;
use std::time::Duration;

use ha_core::{Attributes, Core, Event};
use ha_thermostat::{Thermostats, parse};
use serde_json::{Value, json};

const CONFIG: &str = "
- id: salon
  name: Chauffage du salon
  heater: switch.radiateur
  sensor: sensor.temperature
  window: binary_sensor.fenetre
  min_cycle: 0s
  presets: {confort: 20, eco: 17}
";

fn set(core: &Core, entity: &str, value: &str) {
    core.states
        .set(entity.parse().unwrap(), value, Attributes::new())
        .unwrap();
}

fn climate(core: &Core) -> (String, Attributes) {
    let state = core.states.get(&"climate.salon".parse().unwrap()).unwrap();
    (state.state().to_owned(), state.attributes().clone())
}

fn call(core: &Core, service: &str, data: Value) {
    let Value::Object(data) = data else { panic!() };
    core.call_service("climate", service, "climate.salon".parse().unwrap(), data)
        .unwrap();
}

/// Un radiateur qui obéit : il passe à « on » ou « off » quand on le lui demande.
fn obedient_heater(core: &Arc<Core>) -> tokio::task::JoinHandle<()> {
    let core = Arc::clone(core);
    let mut rx = core.bus.subscribe();
    tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            if let Event::CallService {
                domain,
                service,
                entity_id,
                ..
            } = event.as_ref()
                && domain == "switch"
            {
                let state = if service == "turn_on" { "on" } else { "off" };
                core.states
                    .set(entity_id.clone(), state, Attributes::new())
                    .unwrap();
            }
        }
    })
}

async fn wait_heater(core: &Core, expected: &str) {
    for _ in 0..200 {
        if core
            .states
            .get(&"switch.radiateur".parse().unwrap())
            .is_some_and(|s| s.state() == expected)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("le radiateur n'est jamais passé à {expected}");
}

async fn settle() {
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn the_thermostat_heats_only_when_needed() {
    let core = Arc::new(Core::new());
    let heater = obedient_heater(&core);
    set(&core, "switch.radiateur", "off");
    set(&core, "sensor.temperature", "17.5");
    set(&core, "binary_sensor.fenetre", "off");
    let thermostats = Thermostats::start_with_tick(
        parse(CONFIG).unwrap(),
        Arc::clone(&core),
        Duration::from_millis(50),
    );
    settle().await;

    // Premier démarrage : arrêté, rien n'est allumé.
    let (mode, attributes) = climate(&core);
    assert_eq!(mode, "off");
    assert_eq!(attributes["temperature"], 19.0);
    assert_eq!(attributes["current_temperature"], 17.5);
    assert_eq!(attributes["preset_modes"], json!(["confort", "eco"]));

    // En marche : il fait 17,5 °C pour 19 demandés → on chauffe.
    call(&core, "set_hvac_mode", json!({"hvac_mode": "heat"}));
    wait_heater(&core, "on").await;
    settle().await;
    assert_eq!(climate(&core).1["hvac_action"], "heating");

    // 19,1 °C : dans la marge, on continue. 19,3 °C : on arrête.
    set(&core, "sensor.temperature", "19.1");
    settle().await;
    wait_heater(&core, "on").await;
    set(&core, "sensor.temperature", "19.3");
    wait_heater(&core, "off").await;
    settle().await;
    assert_eq!(climate(&core).1["hvac_action"], "idle");

    // Préréglage « confort » (20 °C) : on rechauffe.
    call(&core, "set_preset_mode", json!({"preset_mode": "confort"}));
    wait_heater(&core, "on").await;
    assert_eq!(climate(&core).1["preset_mode"], "confort");

    // Fenêtre ouverte : coupé tout de suite, et c'est affiché.
    set(&core, "binary_sensor.fenetre", "on");
    wait_heater(&core, "off").await;
    settle().await;
    assert!(
        climate(&core).1["info"]
            .as_str()
            .unwrap()
            .contains("fenêtre")
    );
    set(&core, "binary_sensor.fenetre", "off");
    wait_heater(&core, "on").await;

    // Capteur en panne : coupé par sécurité.
    set(&core, "sensor.temperature", "unavailable");
    wait_heater(&core, "off").await;
    set(&core, "sensor.temperature", "18.0");
    wait_heater(&core, "on").await;

    // Une consigne absurde est refusée avant l'envoi (étape 53) : rien ne
    // change.
    let (_, before) = climate(&core);
    let Value::Object(data) = json!({"temperature": 40}) else {
        panic!()
    };
    let refused = core
        .call_service(
            "climate",
            "set_temperature",
            "climate.salon".parse().unwrap(),
            data,
        )
        .unwrap_err();
    assert_eq!(
        refused.to_string(),
        "Chauffage du salon : consigne entre 7 et 25 (reçu 40)"
    );
    settle().await;
    let (_, attributes) = climate(&core);
    assert_eq!(attributes["temperature"], before["temperature"]);
    assert_eq!(attributes["preset_mode"], before["preset_mode"]);

    // Arrêt : coupé.
    call(&core, "turn_off", json!({}));
    wait_heater(&core, "off").await;
    settle().await;
    assert_eq!(climate(&core).1["hvac_action"], "off");

    thermostats.stop().await;
    heater.abort();
}

#[tokio::test]
async fn settings_survive_a_restart() {
    let core = Arc::new(Core::new());
    // L'état d'avant l'arrêt, tel que la base le restaure.
    let Value::Object(previous) = json!({"temperature": 21.5, "preset_mode": "eco"}) else {
        panic!()
    };
    core.states
        .set("climate.salon".parse().unwrap(), "heat", previous)
        .unwrap();
    set(&core, "sensor.temperature", "20.0");
    set(&core, "switch.radiateur", "off");
    let thermostats = Thermostats::start(parse(CONFIG).unwrap(), Arc::clone(&core));
    settle().await;
    let (mode, attributes) = climate(&core);
    assert_eq!(mode, "heat");
    assert_eq!(attributes["temperature"], 21.5);
    assert_eq!(attributes["preset_mode"], "eco");
    thermostats.stop().await;
}

#[tokio::test]
async fn the_minimum_cycle_protects_the_relay() {
    let core = Arc::new(Core::new());
    let heater = obedient_heater(&core);
    set(&core, "switch.radiateur", "off");
    set(&core, "sensor.temperature", "17.0");
    let config = CONFIG.replace("min_cycle: 0s", "min_cycle: 5min");
    let thermostats = Thermostats::start_with_tick(
        parse(&config).unwrap(),
        Arc::clone(&core),
        Duration::from_millis(50),
    );
    call(&core, "turn_on", json!({}));
    wait_heater(&core, "on").await;
    // Il fait chaud tout de suite : on ne coupe pas avant 5 minutes…
    set(&core, "sensor.temperature", "22.0");
    tokio::time::sleep(Duration::from_millis(200)).await;
    wait_heater(&core, "on").await;
    // … sauf pour la sécurité : la fenêtre s'ouvre, on coupe.
    set(&core, "binary_sensor.fenetre", "on");
    wait_heater(&core, "off").await;
    thermostats.stop().await;
    heater.abort();
}

#[tokio::test]
async fn a_deaf_heater_is_not_spammed() {
    // Un radiateur injoignable n'obéit pas : l'ordre est renvoyé au plus
    // une fois par minute, même si la température oscille.
    let core = Arc::new(Core::new());
    let mut rx = core.bus.subscribe();
    set(&core, "switch.radiateur", "off");
    set(&core, "sensor.temperature", "17.0");
    let thermostats = Thermostats::start_with_tick(
        parse(CONFIG).unwrap(),
        Arc::clone(&core),
        Duration::from_millis(20),
    );
    call(&core, "turn_on", json!({}));
    for value in ["18.9", "18.5", "18.95", "18.6", "18.9", "18.4"] {
        set(&core, "sensor.temperature", value);
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    let mut orders = 0;
    while let Ok(event) = rx.try_recv() {
        if matches!(event.as_ref(), Event::CallService { domain, .. } if domain == "switch") {
            orders += 1;
        }
    }
    assert_eq!(orders, 1, "un seul ordre envoyé");
    thermostats.stop().await;
}
