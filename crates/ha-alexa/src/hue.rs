//! Traduction entre le langage d'un pont Philips Hue (version 1 de son API)
//! et nos appareils.
//!
//! Pour Alexa, tout appareil exposé est une **lampe** Hue : allumée ou
//! éteinte (`on`), avec une luminosité (`bri`, de 1 à 254). On traduit :
//!
//! | Appareil | `on: true` / `on: false` | `bri` |
//! |----------|--------------------------|-------|
//! | lampe | allumer / éteindre | luminosité |
//! | prise | allumer / éteindre | — |
//! | volet | ouvrir / fermer | position (« mets le volet à 50 % ») |
//! | script | lancer / arrêter | — |
//! | scène | activer / rien | — |
//!
//! Aucune entrée/sortie ici : uniquement des fonctions, faciles à tester.

use ha_core::{Attributes, EntityId, State};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::config::Device;

/// Volets qui n'en sont pas : un portail ou une porte de garage ne s'ouvre pas
/// à la voix (n'importe qui criant devant la fenêtre pourrait l'ouvrir).
const FORBIDDEN_COVER_CLASSES: [&str; 3] = ["garage", "gate", "door"];

/// Ce qu'Alexa demande : allumer/éteindre, et/ou une luminosité.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Command {
    pub on: Option<bool>,
    /// De 0 à 254.
    pub bri: Option<u8>,
}

/// Un appel de service à faire : (domaine, service, paramètres).
pub type ServiceCall = (&'static str, &'static str, Attributes);

/// Peut-on piloter cet appareil à la voix ? Non pour un portail ou une porte
/// de garage, même s'il est listé dans `alexa.yaml`.
#[must_use]
pub fn is_allowed(entity_id: &EntityId, state: Option<&State>) -> bool {
    if entity_id.domain() != "cover" {
        return true;
    }
    let class = state.and_then(|state| state.attributes().get("device_class"));
    !class
        .and_then(Value::as_str)
        .is_some_and(|class| FORBIDDEN_COVER_CLASSES.contains(&class))
}

/// L'appareil se règle-t-il (luminosité, position) ?
fn is_dimmable(entity_id: &EntityId) -> bool {
    matches!(entity_id.domain(), "light" | "cover")
}

/// Le nom donné à Alexa.
#[must_use]
pub fn name_of(device: &Device, state: Option<&State>) -> String {
    if let Some(name) = &device.name {
        return name.clone();
    }
    state
        .and_then(|state| state.attributes().get("friendly_name"))
        .and_then(Value::as_str)
        .map_or_else(
            || device.entity_id.object_id().replace('_', " "),
            |name| name.chars().take(32).collect(),
        )
}

/// L'état actuel, vu comme une lampe Hue : (allumée ?, luminosité 1 à 254).
#[must_use]
pub fn current(entity_id: &EntityId, state: Option<&State>) -> (bool, u8) {
    let Some(state) = state else {
        return (false, 0);
    };
    let attributes = state.attributes();
    match entity_id.domain() {
        "light" => {
            let on = state.state() == "on";
            let bri = attributes
                .get("brightness")
                .and_then(Value::as_f64)
                .map_or(254, |b| to_bri(b / 255.0));
            (on, if on { bri } else { 0 })
        }
        "cover" => {
            if let Some(position) = attributes.get("current_position").and_then(Value::as_f64) {
                (position > 0.0, to_bri(position / 100.0))
            } else {
                let open = matches!(state.state(), "open" | "opening");
                (open, if open { 254 } else { 0 })
            }
        }
        // Une scène n'a pas d'état « allumé » : elle s'active, c'est tout.
        "scene" => (false, 0),
        _ => (state.state() == "on", 0),
    }
}

/// `fraction` (0 à 1) → luminosité Hue (0 à 254).
fn to_bri(fraction: f64) -> u8 {
    let value = (fraction.clamp(0.0, 1.0) * 254.0).round();
    // Concept Rust : `as` sur un nombre à virgule déjà borné entre 0 et 254
    // est sans surprise ; le compilateur ne peut pas le savoir, d'où le `expect`.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "valeur bornée entre 0 et 254 juste au-dessus"
    )]
    let bri = value as u8;
    bri
}

/// L'appareil répond-il ?
fn reachable(entity_id: &EntityId, state: Option<&State>) -> bool {
    match state {
        None => false,
        // Une scène jamais activée est « unknown » : elle marche quand même.
        Some(_) if entity_id.domain() == "scene" => true,
        Some(state) => !matches!(state.state(), "unavailable" | "unknown"),
    }
}

/// Identifiant unique façon Hue (`00:17:88:01:xx:xx:xx:xx-0b`), stable pour
/// un appareil donné.
#[must_use]
pub fn unique_id(entity_id: &EntityId) -> String {
    let hash = Sha256::digest(entity_id.as_str().as_bytes());
    format!(
        "00:17:88:01:{:02x}:{:02x}:{:02x}:{:02x}-0b",
        hash[0], hash[1], hash[2], hash[3]
    )
}

/// La description d'une lampe, au format Hue. `expected` : l'état promis à
/// Alexa juste après un ordre (voir `server.rs`).
#[must_use]
pub fn light_json(device: &Device, state: Option<&State>, expected: Option<(bool, u8)>) -> Value {
    let entity_id = &device.entity_id;
    let (on, bri) = expected.unwrap_or_else(|| current(entity_id, state));
    let dimmable = is_dimmable(entity_id);
    let mut light_state = json!({
        "on": on,
        "alert": "none",
        "mode": "homeautomation",
        "reachable": reachable(entity_id, state),
    });
    if dimmable {
        light_state["bri"] = json!(bri.max(1));
    }
    json!({
        "state": light_state,
        "type": if dimmable { "Dimmable light" } else { "On/Off light" },
        "name": name_of(device, state),
        "modelid": if dimmable { "HASS123" } else { "HASS321" },
        "manufacturername": "ha-server",
        "productname": if dimmable { "Dimmable light" } else { "On/Off light" },
        "uniqueid": unique_id(entity_id),
        "swversion": "1.0",
    })
}

/// Lit un ordre d'Alexa : `{"on": true}`, `{"bri": 127}`, ou les deux. Les
/// autres champs (couleur, durée de transition…) sont ignorés.
pub fn parse_command(body: &Value) -> Result<Command, String> {
    let Some(object) = body.as_object() else {
        return Err("objet JSON attendu".into());
    };
    let on = match object.get("on") {
        None => None,
        Some(Value::Bool(on)) => Some(*on),
        Some(_) => return Err("« on » doit être true ou false".into()),
    };
    let bri = match object.get("bri") {
        None => None,
        Some(value) => {
            let bri = value
                .as_u64()
                .ok_or_else(|| "« bri » doit être un nombre de 0 à 254".to_owned())?;
            Some(u8::try_from(bri.min(254)).unwrap_or(254))
        }
    };
    if on.is_none() && bri.is_none() {
        return Err("rien à faire (ni « on », ni « bri »)".into());
    }
    Ok(Command { on, bri })
}

/// Les appels de service qui exécutent l'ordre, et l'état qu'on promet.
#[must_use]
pub fn service_calls(entity_id: &EntityId, command: Command) -> (Vec<ServiceCall>, (bool, u8)) {
    let mut data = Attributes::new();
    let on = command.on.unwrap_or(true);
    let calls: Vec<ServiceCall> = match (entity_id.domain(), command.bri) {
        ("light", Some(bri)) if on && bri > 0 => {
            // Hue : 1 à 254 ; chez nous : 1 à 255.
            let brightness = (u32::from(bri) * 255).div_ceil(254).clamp(1, 255);
            data.insert("brightness".into(), brightness.into());
            vec![("light", "turn_on", data)]
        }
        ("cover", Some(bri)) => {
            let position = (u32::from(bri) * 100 + 127) / 254;
            data.insert("position".into(), position.into());
            vec![("cover", "set_cover_position", data)]
        }
        ("light", _) => vec![("light", if on { "turn_on" } else { "turn_off" }, data)],
        ("switch", _) => vec![("switch", if on { "turn_on" } else { "turn_off" }, data)],
        ("cover", None) => vec![("cover", if on { "open_cover" } else { "close_cover" }, data)],
        ("script", _) => vec![("script", if on { "turn_on" } else { "turn_off" }, data)],
        ("scene", _) if on => vec![("scene", "turn_on", data)],
        _ => Vec::new(),
    };
    let bri = match command.bri {
        Some(bri) => bri,
        None if on => 254,
        None => 0,
    };
    let on = if entity_id.domain() == "cover" && command.bri.is_some() {
        bri > 0
    } else {
        on && bri > 0
    };
    (calls, (on, bri))
}

/// La réponse attendue par Alexa : un succès par valeur modifiée.
#[must_use]
pub fn success_response(number: u32, command: Command) -> Value {
    let mut results = Vec::new();
    if let Some(on) = command.on {
        let mut success = Map::new();
        success.insert(format!("/lights/{number}/state/on"), on.into());
        results.push(json!({ "success": success }));
    }
    if let Some(bri) = command.bri {
        let mut success = Map::new();
        success.insert(format!("/lights/{number}/state/bri"), bri.into());
        results.push(json!({ "success": success }));
    }
    Value::Array(results)
}

/// Une erreur au format Hue.
#[must_use]
pub fn error_response(kind: u32, address: &str, description: &str) -> Value {
    json!([{ "error": { "type": kind, "address": address, "description": description } }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use ha_core::Core;

    fn id(value: &str) -> EntityId {
        value.parse().unwrap()
    }

    fn state(core: &Core, entity: &str, value: &str, attributes: Value) -> std::sync::Arc<State> {
        let Value::Object(map) = attributes else {
            panic!("objet attendu")
        };
        core.states.set(id(entity), value, map).unwrap();
        core.states.get(&id(entity)).unwrap()
    }

    #[test]
    fn states_are_seen_as_hue_lights() {
        let core = Core::new();
        let lamp = state(&core, "light.salon", "on", json!({"brightness": 128}));
        assert_eq!(current(&id("light.salon"), Some(&lamp)), (true, 127));
        let off = state(&core, "light.salon", "off", json!({}));
        assert_eq!(current(&id("light.salon"), Some(&off)), (false, 0));

        let cover = state(
            &core,
            "cover.volet",
            "open",
            json!({"current_position": 50}),
        );
        assert_eq!(current(&id("cover.volet"), Some(&cover)), (true, 127));
        let closed = state(&core, "cover.volet", "closed", json!({}));
        assert_eq!(current(&id("cover.volet"), Some(&closed)), (false, 0));

        let plug = state(
            &core,
            "switch.cafe",
            "on",
            json!({"friendly_name": "prise cafe"}),
        );
        let device = Device {
            entity_id: id("switch.cafe"),
            name: None,
        };
        let light = light_json(&device, Some(&plug), None);
        assert_eq!(light["state"]["on"], true);
        assert_eq!(light["state"]["reachable"], true);
        assert_eq!(light["type"], "On/Off light");
        assert_eq!(light["name"], "prise cafe");
        assert!(light["state"].get("bri").is_none());

        // Absent ou indisponible : « injoignable » pour Alexa.
        assert_eq!(light_json(&device, None, None)["state"]["reachable"], false);
    }

    #[test]
    fn commands_become_service_calls() {
        let on = Command {
            on: Some(true),
            bri: None,
        };
        let off = Command {
            on: Some(false),
            bri: None,
        };
        let dim = Command {
            on: Some(true),
            bri: Some(127),
        };
        let names = |calls: Vec<ServiceCall>| -> Vec<String> {
            calls
                .into_iter()
                .map(|(domain, service, data)| {
                    format!("{domain}.{service} {}", Value::Object(data))
                })
                .collect()
        };
        let cases = [
            ("light.a", on, "light.turn_on {}"),
            ("light.a", off, "light.turn_off {}"),
            ("light.a", dim, r#"light.turn_on {"brightness":128}"#),
            ("switch.a", on, "switch.turn_on {}"),
            ("switch.a", off, "switch.turn_off {}"),
            ("cover.a", on, "cover.open_cover {}"),
            ("cover.a", off, "cover.close_cover {}"),
            (
                "cover.a",
                dim,
                r#"cover.set_cover_position {"position":50}"#,
            ),
            ("script.a", on, "script.turn_on {}"),
            ("scene.a", on, "scene.turn_on {}"),
        ];
        for (entity, command, expected) in cases {
            let (calls, _) = service_calls(&id(entity), command);
            assert_eq!(names(calls), [expected], "{entity} {command:?}");
        }
        // Éteindre une scène : rien à faire.
        assert_eq!(
            names(service_calls(&id("scene.a"), off).0),
            Vec::<String>::new()
        );
        // Luminosité 254 = 255 chez nous ; 1 reste au moins 1.
        let (calls, promised) = service_calls(
            &id("light.a"),
            Command {
                on: None,
                bri: Some(254),
            },
        );
        assert_eq!(names(calls), [r#"light.turn_on {"brightness":255}"#]);
        assert_eq!(promised, (true, 254));
        let (calls, _) = service_calls(
            &id("light.a"),
            Command {
                on: None,
                bri: Some(1),
            },
        );
        assert_eq!(names(calls), [r#"light.turn_on {"brightness":2}"#]);
        // Volet à 0 % : fermé.
        let (_, promised) = service_calls(
            &id("cover.a"),
            Command {
                on: None,
                bri: Some(0),
            },
        );
        assert_eq!(promised, (false, 0));
    }

    #[test]
    fn commands_are_parsed_strictly() {
        assert_eq!(
            parse_command(&json!({"on": true, "transitiontime": 4})),
            Ok(Command {
                on: Some(true),
                bri: None
            })
        );
        assert_eq!(
            parse_command(&json!({"bri": 900})).unwrap().bri,
            Some(254),
            "borné à 254"
        );
        for bad in [
            json!({"on": "oui"}),
            json!({"bri": -1}),
            json!({"bri": 1.5}),
            json!({}),
            json!([]),
        ] {
            assert!(parse_command(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn garage_doors_and_gates_are_never_allowed() {
        let core = Core::new();
        let garage = state(
            &core,
            "cover.garage",
            "closed",
            json!({"device_class": "garage"}),
        );
        assert!(!is_allowed(&id("cover.garage"), Some(&garage)));
        let gate = state(
            &core,
            "cover.portail",
            "closed",
            json!({"device_class": "gate"}),
        );
        assert!(!is_allowed(&id("cover.portail"), Some(&gate)));
        let shutter = state(
            &core,
            "cover.volet",
            "closed",
            json!({"device_class": "shutter"}),
        );
        assert!(is_allowed(&id("cover.volet"), Some(&shutter)));
        assert!(is_allowed(&id("light.a"), None));
    }

    #[test]
    fn unique_ids_are_stable_and_distinct() {
        assert_eq!(unique_id(&id("light.a")), unique_id(&id("light.a")));
        assert_ne!(unique_id(&id("light.a")), unique_id(&id("light.b")));
        assert!(unique_id(&id("light.a")).starts_with("00:17:88:01:"));
    }
}
