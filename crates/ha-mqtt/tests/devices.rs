//! Tests des nouveaux types d'appareils MQTT (verrou, sirène, vanne,
//! aspirateur, humidificateur, chauffe-eau), sans broker.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use ha_core::{Attributes, Core, EntityId};
use ha_mqtt::Bridge;
use ha_mqtt::bridge::Action;
use serde_json::{Value, json};

fn id(value: &str) -> EntityId {
    value.parse().unwrap()
}

fn data(value: Value) -> Attributes {
    let Value::Object(map) = value else {
        panic!("un objet est attendu")
    };
    map
}

fn publish(topic: &str, payload: &str) -> Vec<Action> {
    vec![Action::Publish {
        topic: topic.into(),
        payload: payload.into(),
    }]
}

/// Un pont avec une entité annoncée.
fn announce(component: &str, config: &str) -> (Core, Bridge, Vec<Action>) {
    let core = Core::new();
    let mut bridge = Bridge::new("homeassistant");
    let actions = bridge.handle_message(
        &format!("homeassistant/{component}/objet/config"),
        config.as_bytes(),
        &core.states,
    );
    (core, bridge, actions)
}

#[test]
fn a_lock_reports_its_states_and_obeys() {
    let (core, mut bridge, _) = announce(
        "lock",
        r#"{"name": "Porte d'entrée", "stat_t": "z2m/serrure", "cmd_t": "z2m/serrure/set",
            "val_tpl": "{{ value_json.lock_state }}", "stat_locked": "locked",
            "stat_unlocked": "unlocked", "pl_lock": "LOCK", "pl_unlk": "UNLOCK"}"#,
    );
    let lock = id("lock.porte_d_entree");
    bridge.handle_message("z2m/serrure", br#"{"lock_state": "locked"}"#, &core.states);
    let state = core.states.get(&lock).unwrap();
    assert_eq!(state.state(), "locked");
    // Pas de `payload_open` annoncé : la porte ne s'ouvre pas, elle se déverrouille.
    assert_eq!(state.attributes()["supports_open"], false);

    let empty = Attributes::new();
    assert_eq!(
        bridge.handle_service("lock", "unlock", &lock, &empty, &core.states),
        publish("z2m/serrure/set", "UNLOCK")
    );
    assert_eq!(
        bridge.handle_service("lock", "open", &lock, &empty, &core.states),
        Vec::<Action>::new()
    );
    // « Basculer » une serrure n'existe pas : trop facile de se tromper.
    assert_eq!(
        bridge.handle_service("lock", "toggle", &lock, &empty, &core.states),
        Vec::<Action>::new()
    );
    // Une valeur inconnue ne change rien.
    bridge.handle_message("z2m/serrure", br#"{"lock_state": "???"}"#, &core.states);
    assert_eq!(core.states.get(&lock).unwrap().state(), "locked");
}

#[test]
fn a_siren_is_switched_like_a_plug() {
    let (core, bridge, _) = announce(
        "siren",
        r#"{"name": "Sirène", "cmd_t": "maison/sirene/set"}"#,
    );
    let siren = id("siren.sirene");
    assert_eq!(
        bridge.handle_service("siren", "turn_on", &siren, &Attributes::new(), &core.states),
        publish("maison/sirene/set", "ON")
    );
    // Sans sujet d'état : on suppose qu'elle a obéi.
    assert_eq!(core.states.get(&siren).unwrap().state(), "on");
}

#[test]
fn a_valve_toggles_from_its_state() {
    let (core, mut bridge, _) = announce(
        "valve",
        r#"{"name": "Arrosage", "stat_t": "jardin/vanne", "cmd_t": "jardin/vanne/set"}"#,
    );
    let valve = id("valve.arrosage");
    bridge.handle_message("jardin/vanne", b"closed", &core.states);
    assert_eq!(core.states.get(&valve).unwrap().state(), "closed");
    let empty = Attributes::new();
    assert_eq!(
        bridge.handle_service("valve", "toggle", &valve, &empty, &core.states),
        publish("jardin/vanne/set", "OPEN")
    );
    bridge.handle_message("jardin/vanne", b"open", &core.states);
    assert_eq!(
        bridge.handle_service("valve", "toggle", &valve, &empty, &core.states),
        publish("jardin/vanne/set", "CLOSE")
    );
}

#[test]
fn a_vacuum_reads_state_and_battery_from_one_message() {
    let (core, mut bridge, actions) = announce(
        "vacuum",
        r#"{"name": "Robot", "stat_t": "robot/etat", "cmd_t": "robot/commande"}"#,
    );
    // Un seul abonnement, même si l'état et la batterie y arrivent tous les deux.
    assert_eq!(actions, [Action::Subscribe("robot/etat".into())]);
    let robot = id("vacuum.robot");
    bridge.handle_message(
        "robot/etat",
        br#"{"state": "cleaning", "battery_level": 61, "fan_speed": "max"}"#,
        &core.states,
    );
    let state = core.states.get(&robot).unwrap();
    assert_eq!(state.state(), "cleaning");
    assert_eq!(state.attributes()["battery_level"], 61.0);
    assert_eq!(state.attributes()["fan_speed"], "max");
    assert_eq!(
        bridge.handle_service(
            "vacuum",
            "return_to_base",
            &robot,
            &Attributes::new(),
            &core.states
        ),
        publish("robot/commande", "return_to_base")
    );
}

#[test]
fn a_humidifier_keeps_its_target_between_messages() {
    let (core, mut bridge, actions) = announce(
        "humidifier",
        r#"{"name": "Humidificateur", "stat_t": "h/etat", "cmd_t": "h/set",
            "hum_cmd_t": "h/consigne/set", "hum_stat_t": "h/consigne",
            "curr_hum_t": "h/mesure", "min_hum": 30, "max_hum": 70}"#,
    );
    assert_eq!(actions.len(), 3, "{actions:?}");
    let humidifier = id("humidifier.humidificateur");
    bridge.handle_message("h/consigne", b"45", &core.states);
    bridge.handle_message("h/mesure", b"38", &core.states);
    bridge.handle_message("h/etat", b"ON", &core.states);
    let state = core.states.get(&humidifier).unwrap();
    assert_eq!(state.state(), "on");
    // Le message « ON » n'a pas effacé la consigne ni la mesure.
    assert_eq!(state.attributes()["humidity"], 45.0);
    assert_eq!(state.attributes()["current_humidity"], 38.0);
    assert_eq!(state.attributes()["max_humidity"], 70.0);

    assert_eq!(
        bridge.handle_service(
            "humidifier",
            "set_humidity",
            &humidifier,
            &data(json!({"humidity": 50})),
            &core.states
        ),
        publish("h/consigne/set", "50")
    );
    // Hors des bornes annoncées : rien n'est envoyé.
    assert_eq!(
        bridge.handle_service(
            "humidifier",
            "set_humidity",
            &humidifier,
            &data(json!({"humidity": 95})),
            &core.states
        ),
        Vec::<Action>::new()
    );
    assert_eq!(
        bridge.handle_service(
            "humidifier",
            "turn_off",
            &humidifier,
            &Attributes::new(),
            &core.states
        ),
        publish("h/set", "OFF")
    );
}

#[test]
fn a_water_heater_has_modes_and_a_setpoint() {
    let (core, mut bridge, _) = announce(
        "water_heater",
        r#"{"name": "Chauffe-eau", "mode_stat_t": "ce/mode", "mode_cmd_t": "ce/mode/set",
            "temp_cmd_t": "ce/consigne/set", "temp_stat_t": "ce/consigne",
            "curr_temp_t": "ce/mesure", "modes": ["off", "eco", "electric"],
            "min_temp": 40, "max_temp": 65}"#,
    );
    let heater = id("water_heater.chauffe_eau");
    bridge.handle_message("ce/mode", b"eco", &core.states);
    bridge.handle_message("ce/mesure", b"52.5", &core.states);
    let state = core.states.get(&heater).unwrap();
    assert_eq!(state.state(), "eco");
    assert_eq!(state.attributes()["current_temperature"], 52.5);
    assert_eq!(
        state.attributes()["operation_list"],
        json!(["off", "eco", "electric"])
    );
    // Un mode qu'il n'a pas annoncé est ignoré.
    bridge.handle_message("ce/mode", b"turbo", &core.states);
    assert_eq!(core.states.get(&heater).unwrap().state(), "eco");

    assert_eq!(
        bridge.handle_service(
            "water_heater",
            "set_operation_mode",
            &heater,
            &data(json!({"operation_mode": "electric"})),
            &core.states
        ),
        publish("ce/mode/set", "electric")
    );
    assert_eq!(
        bridge.handle_service(
            "water_heater",
            "set_operation_mode",
            &heater,
            &data(json!({"operation_mode": "turbo"})),
            &core.states
        ),
        Vec::<Action>::new()
    );
    assert_eq!(
        bridge.handle_service(
            "water_heater",
            "set_temperature",
            &heater,
            &data(json!({"temperature": 55})),
            &core.states
        ),
        publish("ce/consigne/set", "55")
    );
    // Pas de sujet « marche / arrêt » annoncé : rien à publier.
    assert_eq!(
        bridge.handle_service(
            "water_heater",
            "turn_off",
            &heater,
            &Attributes::new(),
            &core.states
        ),
        Vec::<Action>::new()
    );
}

#[test]
fn removing_an_entity_releases_all_its_topics() {
    let (core, mut bridge, _) = announce(
        "humidifier",
        r#"{"name": "H", "stat_t": "h/etat", "cmd_t": "h/set", "hum_cmd_t": "h/c/set",
            "hum_stat_t": "h/c"}"#,
    );
    let mut actions =
        bridge.handle_message("homeassistant/humidifier/objet/config", b"", &core.states);
    actions.sort_by_key(|a| format!("{a:?}"));
    assert_eq!(
        actions,
        [
            Action::Unsubscribe("h/c".into()),
            Action::Unsubscribe("h/etat".into())
        ]
    );
    assert!(core.states.get(&id("humidifier.h")).is_none());
}

#[test]
fn invalid_device_descriptions_are_refused() {
    for (component, config) in [
        // Bornes impossibles.
        (
            "humidifier",
            r#"{"cmd_t": "h", "min_hum": 80, "max_hum": 20}"#,
        ),
        ("water_heater", r#"{"mode_cmd_t": "c", "max_temp": 1000}"#),
        // Modes : liste vide, pas des textes, texte avec retour à la ligne.
        ("water_heater", r#"{"mode_cmd_t": "c", "modes": []}"#),
        ("water_heater", r#"{"mode_cmd_t": "c", "modes": [1, 2]}"#),
        ("water_heater", r#"{"mode_cmd_t": "c", "modes": ["a\nb"]}"#),
        // Modèle non compris.
        (
            "vacuum",
            r#"{"stat_t": "r", "val_tpl": "{{ value_json.state | upper }}"}"#,
        ),
    ] {
        let (core, _, actions) = announce(component, config);
        assert_eq!(actions, Vec::<Action>::new(), "{component} {config}");
        assert_eq!(core.states.all().len(), 0, "{component} {config}");
    }
}
