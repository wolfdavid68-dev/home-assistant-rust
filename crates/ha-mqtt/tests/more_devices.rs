//! Tests des types MQTT de l'étape 29 (volet, ventilateur, thermostat,
//! bouton, nombre, liste, texte, événement, alarme), sans broker. Les
//! descriptions ressemblent à celles de Zigbee2MQTT.

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

fn data(value: &Value) -> Attributes {
    let Value::Object(map) = value.clone() else {
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

fn call(bridge: &Bridge, core: &Core, entity: &str, service: &str, value: &Value) -> Vec<Action> {
    let entity = id(entity);
    bridge.handle_service(
        entity.domain(),
        service,
        &entity,
        &data(value),
        &core.states,
    )
}

#[test]
fn a_cover_with_a_position_from_0_to_255() {
    let (core, mut bridge, _) = announce(
        "cover",
        r#"{"name": "Volet", "dev_cla": "shutter", "stat_t": "v/etat", "cmd_t": "v/set",
            "pos_t": "v/pos", "set_pos_t": "v/pos/set", "pos_open": 255, "pos_clsd": 0}"#,
    );
    bridge.handle_message("v/etat", b"open", &core.states);
    bridge.handle_message("v/pos", b"191", &core.states);
    let cover = core.states.get(&id("cover.volet")).unwrap();
    assert_eq!(cover.state(), "open");
    assert_eq!(cover.attributes()["current_position"], 75.0);
    assert_eq!(cover.attributes()["device_class"], "shutter");
    // Un volet arrêté à mi-course reste « ouvert ».
    bridge.handle_message("v/etat", b"stopped", &core.states);
    assert_eq!(core.states.get(&id("cover.volet")).unwrap().state(), "open");

    assert_eq!(
        call(&bridge, &core, "cover.volet", "close_cover", &json!({})),
        publish("v/set", "CLOSE")
    );
    assert_eq!(
        call(&bridge, &core, "cover.volet", "stop_cover", &json!({})),
        publish("v/set", "STOP")
    );
    assert_eq!(
        call(
            &bridge,
            &core,
            "cover.volet",
            "set_cover_position",
            &json!({"position": 50})
        ),
        publish("v/pos/set", "128")
    );
    assert_eq!(
        call(
            &bridge,
            &core,
            "cover.volet",
            "set_cover_position",
            &json!({"position": 150})
        ),
        Vec::<Action>::new()
    );
}

#[test]
fn a_cover_without_state_topic_follows_its_position() {
    let (core, mut bridge, _) = announce(
        "cover",
        r#"{"name": "Store", "cmd_t": "s/set", "pos_t": "s/pos"}"#,
    );
    bridge.handle_message("s/pos", b"0", &core.states);
    assert_eq!(
        core.states.get(&id("cover.store")).unwrap().state(),
        "closed"
    );
    bridge.handle_message("s/pos", b"40", &core.states);
    assert_eq!(core.states.get(&id("cover.store")).unwrap().state(), "open");
}

#[test]
fn a_fan_with_speed_presets_and_oscillation() {
    let (core, mut bridge, _) = announce(
        "fan",
        r#"{"name": "Ventilateur", "stat_t": "f/etat", "cmd_t": "f/set",
            "pct_cmd_t": "f/vitesse/set", "pct_stat_t": "f/vitesse", "spd_rng_min": 1, "spd_rng_max": 6,
            "osc_cmd_t": "f/osc/set", "osc_stat_t": "f/osc",
            "pr_mode_cmd_t": "f/mode/set", "pr_mode_stat_t": "f/mode", "pr_modes": ["nuit", "turbo"]}"#,
    );
    bridge.handle_message("f/etat", b"ON", &core.states);
    bridge.handle_message("f/vitesse", b"3", &core.states);
    bridge.handle_message("f/osc", b"oscillate_on", &core.states);
    bridge.handle_message("f/mode", b"nuit", &core.states);
    let fan = core.states.get(&id("fan.ventilateur")).unwrap();
    assert_eq!(fan.state(), "on");
    assert_eq!(fan.attributes()["percentage"], 50.0);
    assert_eq!(fan.attributes()["oscillating"], true);
    assert_eq!(fan.attributes()["preset_mode"], "nuit");
    assert_eq!(fan.attributes()["preset_modes"], json!(["nuit", "turbo"]));

    assert_eq!(
        call(&bridge, &core, "fan.ventilateur", "toggle", &json!({})),
        publish("f/set", "OFF")
    );
    assert_eq!(
        call(
            &bridge,
            &core,
            "fan.ventilateur",
            "set_percentage",
            &json!({"percentage": 100})
        ),
        publish("f/vitesse/set", "6")
    );
    assert_eq!(
        call(
            &bridge,
            &core,
            "fan.ventilateur",
            "oscillate",
            &json!({"oscillating": false})
        ),
        publish("f/osc/set", "oscillate_off")
    );
    assert_eq!(
        call(
            &bridge,
            &core,
            "fan.ventilateur",
            "set_preset_mode",
            &json!({"preset_mode": "rm -rf"})
        ),
        Vec::<Action>::new()
    );
}

#[test]
fn a_climate_like_a_zigbee_valve() {
    let (core, mut bridge, _) = announce(
        "climate",
        r#"{"name": "Radiateur", "mode_stat_t": "c/mode", "mode_cmd_t": "c/mode/set",
            "modes": ["off", "heat"], "temp_cmd_t": "c/consigne/set", "temp_stat_t": "c/consigne",
            "curr_temp_t": "c/mesure", "act_t": "c/action", "min_temp": 5, "max_temp": 30,
            "temp_step": 0.5}"#,
    );
    for (topic, payload) in [
        ("c/mode", "heat"),
        ("c/consigne", "20.5"),
        ("c/mesure", "19.2"),
        ("c/action", "heating"),
    ] {
        bridge.handle_message(topic, payload.as_bytes(), &core.states);
    }
    let climate = core.states.get(&id("climate.radiateur")).unwrap();
    // Exactement ce qu'attend la tuile thermostat du tableau de bord.
    assert_eq!(climate.state(), "heat");
    assert_eq!(climate.attributes()["temperature"], 20.5);
    assert_eq!(climate.attributes()["current_temperature"], 19.2);
    assert_eq!(climate.attributes()["hvac_action"], "heating");
    assert_eq!(climate.attributes()["target_temp_step"], 0.5);
    assert_eq!(
        call(
            &bridge,
            &core,
            "climate.radiateur",
            "set_temperature",
            &json!({"temperature": 21})
        ),
        publish("c/consigne/set", "21")
    );
    assert_eq!(
        call(
            &bridge,
            &core,
            "climate.radiateur",
            "set_hvac_mode",
            &json!({"hvac_mode": "off"})
        ),
        publish("c/mode/set", "off")
    );
    // Un mode que l'appareil n'a pas annoncé.
    assert_eq!(
        call(
            &bridge,
            &core,
            "climate.radiateur",
            "set_hvac_mode",
            &json!({"hvac_mode": "cool"})
        ),
        Vec::<Action>::new()
    );
}

#[test]
fn a_button_number_select_and_text() {
    // Bouton : l'état est l'heure du dernier appui.
    let (core, bridge, _) = announce("button", r#"{"name": "Redémarrer", "cmd_t": "b/set"}"#);
    assert_eq!(
        call(&bridge, &core, "button.redemarrer", "press", &json!({})),
        publish("b/set", "PRESS")
    );
    let pressed = core.states.get(&id("button.redemarrer")).unwrap();
    assert!(
        pressed.state().parse::<jiff::Timestamp>().is_ok(),
        "{}",
        pressed.state()
    );

    // Nombre.
    let (core, mut bridge, _) = announce(
        "number",
        r#"{"name": "Luminosité LED", "stat_t": "n/etat", "cmd_t": "n/set", "min": 0, "max": 254, "step": 1}"#,
    );
    bridge.handle_message("n/etat", b"128", &core.states);
    assert_eq!(
        core.states
            .get(&id("number.luminosite_led"))
            .unwrap()
            .state(),
        "128"
    );
    bridge.handle_message("n/etat", b"pas un nombre", &core.states);
    assert_eq!(
        core.states
            .get(&id("number.luminosite_led"))
            .unwrap()
            .state(),
        "128"
    );
    assert_eq!(
        call(
            &bridge,
            &core,
            "number.luminosite_led",
            "set_value",
            &json!({"value": 200})
        ),
        publish("n/set", "200")
    );
    assert_eq!(
        call(
            &bridge,
            &core,
            "number.luminosite_led",
            "set_value",
            &json!({"value": 300})
        ),
        Vec::<Action>::new()
    );

    // Liste de choix.
    let (core, mut bridge, _) = announce(
        "select",
        r#"{"name": "Effet", "stat_t": "s/etat", "cmd_t": "s/set", "ops": ["fixe", "respiration"]}"#,
    );
    bridge.handle_message("s/etat", b"respiration", &core.states);
    assert_eq!(
        core.states.get(&id("select.effet")).unwrap().state(),
        "respiration"
    );
    bridge.handle_message("s/etat", b"inconnu", &core.states);
    assert_eq!(
        core.states.get(&id("select.effet")).unwrap().state(),
        "respiration"
    );
    assert_eq!(
        call(
            &bridge,
            &core,
            "select.effet",
            "select_option",
            &json!({"option": "fixe"})
        ),
        publish("s/set", "fixe")
    );

    // Texte, sans sujet d'état : on suppose que l'appareil a obéi.
    let (core, bridge, _) = announce(
        "text",
        r#"{"name": "Message", "cmd_t": "t/set", "max": 20}"#,
    );
    assert_eq!(
        call(
            &bridge,
            &core,
            "text.message",
            "set_value",
            &json!({"value": "Bonne nuit"})
        ),
        publish("t/set", "Bonne nuit")
    );
    assert_eq!(
        core.states.get(&id("text.message")).unwrap().state(),
        "Bonne nuit"
    );
    assert_eq!(
        call(
            &bridge,
            &core,
            "text.message",
            "set_value",
            &json!({"value": "beaucoup trop long pour lui"})
        ),
        Vec::<Action>::new()
    );
}

#[test]
fn a_remote_control_fires_events() {
    let (core, mut bridge, _) = announce(
        "event",
        r#"{"name": "Télécommande", "stat_t": "z2m/telecommande", "evt_typ": ["single", "double", "hold"],
            "val_tpl": "{{ value_json.action_event }}"}"#,
    );
    let actions = bridge.handle_message(
        "z2m/telecommande",
        br#"{"action_event": {"event_type": "double"}}"#,
        &core.states,
    );
    let [Action::Fire { event_type, data }] = actions.as_slice() else {
        panic!("un événement attendu : {actions:?}")
    };
    assert_eq!(event_type, "event_received");
    assert_eq!(data["entity_id"], "event.telecommande");
    assert_eq!(data["event_type"], "double");
    let remote = core.states.get(&id("event.telecommande")).unwrap();
    assert_eq!(remote.attributes()["event_type"], "double");
    // Une sorte non annoncée est ignorée.
    let actions = bridge.handle_message(
        "z2m/telecommande",
        br#"{"action_event": {"event_type": "triple"}}"#,
        &core.states,
    );
    assert_eq!(actions, Vec::<Action>::new());
}

#[test]
fn an_alarm_panel() {
    let (core, mut bridge, _) = announce(
        "alarm_control_panel",
        r#"{"name": "Alarme", "stat_t": "a/etat", "cmd_t": "a/set", "sup_feat": ["arm_home", "arm_away"],
            "code": "1234"}"#,
    );
    bridge.handle_message("a/etat", b"armed_away", &core.states);
    let alarm = core.states.get(&id("alarm_control_panel.alarme")).unwrap();
    assert_eq!(alarm.state(), "armed_away");
    // Le code de l'appareil n'apparaît jamais dans les attributs.
    assert!(!alarm.attributes().values().any(|v| v == "1234"));
    assert_eq!(
        alarm.attributes()["supported_features"],
        json!(["arm_home", "arm_away"])
    );
    assert_eq!(
        call(
            &bridge,
            &core,
            "alarm_control_panel.alarme",
            "alarm_disarm",
            &json!({})
        ),
        publish("a/set", "DISARM")
    );
    assert_eq!(
        call(
            &bridge,
            &core,
            "alarm_control_panel.alarme",
            "alarm_arm_home",
            &json!({})
        ),
        publish("a/set", "ARM_HOME")
    );
    // Pas annoncé : pas de mode nuit.
    assert_eq!(
        call(
            &bridge,
            &core,
            "alarm_control_panel.alarme",
            "alarm_arm_night",
            &json!({})
        ),
        Vec::<Action>::new()
    );
}

#[test]
fn risky_descriptions_are_refused() {
    for (component, config) in [
        // Un vrai modèle de commande Jinja : refusé.
        (
            "alarm_control_panel",
            r#"{"stat_t": "a", "cmd_t": "a/set", "cmd_tpl": "{\"action\": \"{{ action }}\", \"code\": \"{{ code }}\"}"}"#,
        ),
        ("select", r#"{"stat_t": "s", "cmd_t": "s/set"}"#),
        ("select", r#"{"stat_t": "s", "cmd_t": "s/set", "ops": []}"#),
        (
            "number",
            r#"{"stat_t": "n", "cmd_t": "n/set", "min": 10, "max": 5}"#,
        ),
        ("text", r#"{"cmd_t": "t/set", "min": 30, "max": 10}"#),
        ("event", r#"{"stat_t": "e"}"#),
        (
            "cover",
            r#"{"cmd_t": "v/set", "pos_open": 50, "pos_clsd": 50}"#,
        ),
        (
            "fan",
            r#"{"cmd_t": "f/set", "spd_rng_min": 10, "spd_rng_max": 5}"#,
        ),
        ("climate", r#"{"mode_cmd_t": "c/set", "temp_step": 0}"#),
    ] {
        let (core, _, actions) = announce(component, config);
        assert_eq!(actions, Vec::<Action>::new(), "{component} {config}");
        assert_eq!(core.states.all().len(), 0, "{component} {config}");
    }
}
