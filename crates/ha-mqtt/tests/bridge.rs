//! Tests du pont MQTT ⇄ entités, sans broker : on lui donne des messages
//! comme le ferait Zigbee2MQTT ou Tasmota, et on regarde le résultat.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::UNIX_EPOCH;

use ha_core::{Attributes, Core, EntityId};
use ha_mqtt::Bridge;
use ha_mqtt::bridge::{Action, MAX_ENTITIES};
use serde_json::json;

fn id(value: &str) -> EntityId {
    value.parse().unwrap()
}

fn state(core: &Core, entity: &str) -> Option<String> {
    core.states.get(&id(entity)).map(|s| s.state().to_owned())
}

const Z2M_TEMP: &str = r#"{"name": "Temperature", "stat_t": "zigbee2mqtt/capteur",
    "val_tpl": "{{ value_json.temperature }}", "unit_of_meas": "°C",
    "obj_id": "capteur_temperature", "dev": {"name": "Capteur"}}"#;
const Z2M_HUMIDITY: &str = r#"{"name": "Humidity", "stat_t": "zigbee2mqtt/capteur",
    "val_tpl": "{{ value_json.humidity }}", "unit_of_meas": "%",
    "obj_id": "capteur_humidite", "dev": {"name": "Capteur"}}"#;
const TASMOTA_PLUG: &str = r#"{"name": "Prise café", "~": "tasmota/prise/",
    "stat_t": "~stat/POWER", "cmd_t": "~cmnd/POWER"}"#;
const Z2M_LIGHT: &str = r#"{"name": null, "schema": "json", "stat_t": "zigbee2mqtt/lampe",
    "cmd_t": "zigbee2mqtt/lampe/set", "brightness": true, "dev": {"name": "Lampe salon"}}"#;

#[test]
fn zigbee2mqtt_sensors_share_one_topic() {
    let core = Core::new();
    let mut bridge = Bridge::new("homeassistant");

    let a = bridge.handle_message(
        "homeassistant/sensor/0x01/temperature/config",
        Z2M_TEMP.as_bytes(),
        &core.states,
    );
    let b = bridge.handle_message(
        "homeassistant/sensor/0x01/humidity/config",
        Z2M_HUMIDITY.as_bytes(),
        &core.states,
    );
    // Un seul abonnement pour deux entités.
    assert_eq!(a, [Action::Subscribe("zigbee2mqtt/capteur".into())]);
    assert!(b.is_empty());
    assert_eq!(
        state(&core, "sensor.capteur_temperature").as_deref(),
        Some("unknown")
    );

    bridge.handle_message(
        "zigbee2mqtt/capteur",
        br#"{"temperature": 21.5, "humidity": 48, "linkquality": 120}"#,
        &core.states,
    );

    let temperature = core.states.get(&id("sensor.capteur_temperature")).unwrap();
    assert_eq!(temperature.state(), "21.5");
    assert_eq!(temperature.attributes()["unit_of_measurement"], "°C");
    assert_eq!(
        temperature.attributes()["friendly_name"],
        "Capteur Temperature"
    );
    assert_eq!(
        state(&core, "sensor.capteur_humidite").as_deref(),
        Some("48")
    );
}

#[test]
fn tasmota_switch_round_trip() {
    let core = Core::new();
    let mut bridge = Bridge::new("homeassistant");
    bridge.handle_message(
        "homeassistant/switch/prise/config",
        TASMOTA_PLUG.as_bytes(),
        &core.states,
    );
    let plug = id("switch.prise_cafe");

    // L'API demande d'allumer : le pont publie l'ordre…
    let actions =
        bridge.handle_service("switch", "turn_on", &plug, &Attributes::new(), &core.states);
    assert_eq!(
        actions,
        [Action::Publish {
            topic: "tasmota/prise/cmnd/POWER".into(),
            payload: "ON".into(),
        }]
    );
    // …mais l'état ne change que quand la prise confirme.
    assert_eq!(
        state(&core, "switch.prise_cafe").as_deref(),
        Some("unknown")
    );
    bridge.handle_message("tasmota/prise/stat/POWER", b"ON", &core.states);
    assert_eq!(state(&core, "switch.prise_cafe").as_deref(), Some("on"));

    // `toggle` sur une prise allumée l'éteint.
    let actions =
        bridge.handle_service("switch", "toggle", &plug, &Attributes::new(), &core.states);
    assert!(matches!(&actions[..], [Action::Publish { payload, .. }] if payload == "OFF"));

    // Valeur inattendue : ignorée.
    bridge.handle_message("tasmota/prise/stat/POWER", b"PEUT-ETRE", &core.states);
    assert_eq!(state(&core, "switch.prise_cafe").as_deref(), Some("on"));
}

#[test]
fn json_light_with_brightness() {
    let core = Core::new();
    let mut bridge = Bridge::new("homeassistant");
    bridge.handle_message(
        "homeassistant/light/0x02/light/config",
        Z2M_LIGHT.as_bytes(),
        &core.states,
    );
    let lamp = id("light.lampe_salon");

    let data = json!({"brightness": 128}).as_object().unwrap().clone();
    let actions = bridge.handle_service("light", "turn_on", &lamp, &data, &core.states);
    let [Action::Publish { topic, payload }] = &actions[..] else {
        unreachable!("une publication attendue : {actions:?}");
    };
    assert_eq!(topic, "zigbee2mqtt/lampe/set");
    let sent: serde_json::Value = serde_json::from_str(payload).unwrap();
    assert_eq!(sent, json!({"state": "ON", "brightness": 128}));

    bridge.handle_message(
        "zigbee2mqtt/lampe",
        br#"{"state": "ON", "brightness": 200}"#,
        &core.states,
    );
    let light = core.states.get(&lamp).unwrap();
    assert_eq!(light.state(), "on");
    assert_eq!(light.attributes()["brightness"], 200);

    // Luminosité hors limites : refusée.
    let data = json!({"brightness": 9999}).as_object().unwrap().clone();
    assert!(
        bridge
            .handle_service("light", "turn_on", &lamp, &data, &core.states)
            .is_empty()
    );
}

#[test]
fn services_for_other_entities_are_ignored() {
    let core = Core::new();
    let mut bridge = Bridge::new("homeassistant");
    bridge.handle_message(
        "homeassistant/switch/prise/config",
        TASMOTA_PLUG.as_bytes(),
        &core.states,
    );
    let none = Attributes::new();
    // Entité inconnue du pont, domaine incohérent, service inconnu.
    assert!(
        bridge
            .handle_service(
                "switch",
                "turn_on",
                &id("switch.autre"),
                &none,
                &core.states
            )
            .is_empty()
    );
    assert!(
        bridge
            .handle_service(
                "light",
                "turn_on",
                &id("switch.prise_cafe"),
                &none,
                &core.states
            )
            .is_empty()
    );
    assert!(
        bridge
            .handle_service(
                "switch",
                "explode",
                &id("switch.prise_cafe"),
                &none,
                &core.states
            )
            .is_empty()
    );
}

#[test]
fn removal_unsubscribes_and_deletes_the_entity() {
    let core = Core::new();
    let mut bridge = Bridge::new("homeassistant");
    let topic = "homeassistant/switch/prise/config";
    bridge.handle_message(topic, TASMOTA_PLUG.as_bytes(), &core.states);

    let actions = bridge.handle_message(topic, b"", &core.states);

    assert_eq!(
        actions,
        [Action::Unsubscribe("tasmota/prise/stat/POWER".into())]
    );
    assert!(state(&core, "switch.prise_cafe").is_none());
    assert!(bridge.is_empty());
}

#[test]
fn same_name_gets_a_suffix() {
    let core = Core::new();
    let mut bridge = Bridge::new("homeassistant");
    let plug = |n: u8| format!(r#"{{"name": "Prise", "stat_t": "tasmota/p{n}/POWER"}}"#);
    bridge.handle_message(
        "homeassistant/switch/p1/config",
        plug(1).as_bytes(),
        &core.states,
    );
    bridge.handle_message(
        "homeassistant/switch/p2/config",
        plug(2).as_bytes(),
        &core.states,
    );

    assert!(state(&core, "switch.prise").is_some());
    assert!(state(&core, "switch.prise_2").is_some());
}

#[test]
fn restored_state_is_kept_on_rediscovery() {
    let core = Core::new();
    core.states
        .restore(
            id("switch.prise_cafe"),
            "on",
            Attributes::new(),
            UNIX_EPOCH,
            UNIX_EPOCH,
        )
        .unwrap();
    let mut bridge = Bridge::new("homeassistant");

    bridge.handle_message(
        "homeassistant/switch/prise/config",
        TASMOTA_PLUG.as_bytes(),
        &core.states,
    );

    assert_eq!(state(&core, "switch.prise_cafe").as_deref(), Some("on"));
}

#[test]
fn entity_count_is_limited() {
    let core = Core::new();
    let mut bridge = Bridge::new("homeassistant");
    for n in 0..MAX_ENTITIES + 10 {
        let config = format!(r#"{{"stat_t": "spam/{n}"}}"#);
        bridge.handle_message(
            &format!("homeassistant/sensor/s{n}/config"),
            config.as_bytes(),
            &core.states,
        );
    }
    assert_eq!(bridge.len(), MAX_ENTITIES);
}

#[test]
fn optimistic_switch_without_state_topic() {
    let core = Core::new();
    let mut bridge = Bridge::new("homeassistant");
    bridge.handle_message(
        "homeassistant/switch/relais/config",
        br#"{"name": "Relais", "cmd_t": "maison/relais/set"}"#,
        &core.states,
    );

    bridge.handle_service(
        "switch",
        "turn_on",
        &id("switch.relais"),
        &Attributes::new(),
        &core.states,
    );

    assert_eq!(state(&core, "switch.relais").as_deref(), Some("on"));
}

#[test]
fn changing_the_state_topic_moves_the_subscription() {
    let core = Core::new();
    let mut bridge = Bridge::new("homeassistant");
    let topic = "homeassistant/sensor/t/config";
    bridge.handle_message(topic, br#"{"name": "T", "stat_t": "ancien"}"#, &core.states);

    let actions = bridge.handle_message(
        topic,
        br#"{"name": "T", "stat_t": "nouveau"}"#,
        &core.states,
    );

    assert_eq!(
        actions,
        [
            Action::Unsubscribe("ancien".into()),
            Action::Subscribe("nouveau".into()),
        ]
    );
    // Même entité, pas de doublon.
    assert_eq!(bridge.len(), 1);
    assert!(state(&core, "sensor.t").is_some());
}
