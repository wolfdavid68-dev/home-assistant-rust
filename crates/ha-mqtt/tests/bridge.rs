//! Tests du pont MQTT ⇄ entités, sans broker : on lui donne des messages
//! comme le ferait Zigbee2MQTT ou Tasmota, et on regarde le résultat.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

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
    assert_eq!(b, Vec::<Action>::new());
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
    assert_eq!(
        bridge.handle_service("light", "turn_on", &lamp, &data, &core.states),
        Vec::<Action>::new()
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
    assert_eq!(
        bridge.handle_service(
            "switch",
            "turn_on",
            &id("switch.autre"),
            &none,
            &core.states
        ),
        Vec::<Action>::new()
    );
    assert_eq!(
        bridge.handle_service(
            "light",
            "turn_on",
            &id("switch.prise_cafe"),
            &none,
            &core.states
        ),
        Vec::<Action>::new()
    );
    assert_eq!(
        bridge.handle_service(
            "switch",
            "explode",
            &id("switch.prise_cafe"),
            &none,
            &core.states
        ),
        Vec::<Action>::new()
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

    // D'abord le nouveau sujet, puis l'ancien : aucun message perdu entre les deux.
    assert_eq!(
        actions,
        [
            Action::Subscribe("nouveau".into()),
            Action::Unsubscribe("ancien".into()),
        ]
    );
    // Même entité, pas de doublon.
    assert_eq!(bridge.len(), 1);
    assert!(state(&core, "sensor.t").is_some());
}

/// La sonnette Ring, comme ring-mqtt l'annonce (étape 82) : « Ding » et la
/// photo, envoyée telle quelle (JPEG) sur son sujet.
#[test]
fn a_ring_doorbell_and_its_snapshot() {
    let core = Core::new();
    let mut bridge = Bridge::new("homeassistant").with_images(std::sync::Arc::clone(&core.images));
    let device = r#""device": {"name": "Porte d'entrée", "ids": ["abc"], "mf": "Ring"}"#;
    bridge.handle_message(
        "homeassistant/binary_sensor/loc1/abc_ding/config",
        format!(
            r#"{{"name": "Ding", "unique_id": "abc_ding", "device_class": "occupancy",
               "state_topic": "ring/loc1/camera/abc/ding/state",
               "json_attributes_topic": "ring/loc1/camera/abc/ding/attributes",
               "availability_topic": "ring/loc1/camera/abc/status", {device}}}"#
        )
        .as_bytes(),
        &core.states,
    );
    let actions = bridge.handle_message(
        "homeassistant/camera/loc1/abc_snapshot/config",
        format!(
            r#"{{"name": "Snapshot", "unique_id": "abc_snapshot",
               "topic": "ring/loc1/camera/abc/snapshot/image",
               "json_attributes_topic": "ring/loc1/camera/abc/snapshot/attributes",
               "availability_topic": "ring/loc1/camera/abc/status", {device}}}"#
        )
        .as_bytes(),
        &core.states,
    );
    assert_eq!(
        actions,
        [Action::Subscribe(
            "ring/loc1/camera/abc/snapshot/image".into()
        )]
    );

    bridge.handle_message("ring/loc1/camera/abc/ding/state", b"ON", &core.states);
    let ding = core
        .states
        .get(&id("binary_sensor.porte_d_entree_sonnette"))
        .unwrap();
    assert_eq!(ding.state(), "on");
    assert_eq!(ding.attributes()["device_class"], "occupancy");
    assert_eq!(
        ding.attributes()["friendly_name"],
        "Porte d'entrée Sonnette",
        "en français"
    );

    // Une photo de 100 Kio, plus que la limite d'un état (64 Kio).
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
    jpeg.resize(100 * 1024, 0x42);
    bridge.handle_message("ring/loc1/camera/abc/snapshot/image", &jpeg, &core.states);
    let camera = id("camera.porte_d_entree_photo");
    let state = core.states.get(&camera).unwrap();
    assert_eq!(state.state(), "idle");
    assert_eq!(state.attributes()["friendly_name"], "Porte d'entrée Photo");
    assert_eq!(state.attributes()["image_size"], 100 * 1024);
    let image = core.images.get(&camera).unwrap();
    assert_eq!(image.content_type, "image/jpeg");
    assert_eq!(image.bytes.len(), 100 * 1024);

    // Pas une image : refusée, la précédente reste.
    bridge.handle_message(
        "ring/loc1/camera/abc/snapshot/image",
        b"<script>",
        &core.states,
    );
    assert_eq!(core.images.get(&camera).unwrap().bytes.len(), 100 * 1024);

    // La caméra retirée : son image aussi.
    bridge.handle_message(
        "homeassistant/camera/loc1/abc_snapshot/config",
        b"",
        &core.states,
    );
    assert!(core.images.get(&camera).is_none());
}

/// Une image en base64 (`image_encoding: b64`), comme Frigate.
#[test]
fn a_base64_camera() {
    let core = Core::new();
    let mut bridge = Bridge::new("homeassistant").with_images(std::sync::Arc::clone(&core.images));
    bridge.handle_message(
        "homeassistant/camera/porte/config",
        br#"{"name": "Porte", "topic": "cam/porte", "image_encoding": "b64"}"#,
        &core.states,
    );
    // « /9j/4A== » : les octets FF D8 FF E0.
    bridge.handle_message("cam/porte", b"/9j/4A==\n", &core.states);
    assert_eq!(
        core.images.get(&id("camera.porte")).unwrap().content_type,
        "image/jpeg"
    );
    // Un autre encodage est refusé.
    bridge.handle_message(
        "homeassistant/camera/autre/config",
        br#"{"name": "Autre", "topic": "cam/autre", "image_encoding": "hex"}"#,
        &core.states,
    );
    assert!(core.states.get(&id("camera.autre")).is_none());
}
