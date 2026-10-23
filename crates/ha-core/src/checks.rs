//! Vérifier un ordre **avant** de l'envoyer (étape 53) : l'appareil sait-il
//! le faire, la valeur est-elle dans sa plage ? Un ordre refusé ici n'est ni
//! envoyé, ni retenu par la garde : la personne voit tout de suite pourquoi
//! (« Radiateur salon : consigne entre 7 et 30 (reçu 40) »), au lieu d'un
//! ordre perdu en route.
//!
//! Les limites viennent de ce que l'appareil annonce dans ses attributs, avec
//! les mêmes noms que Home Assistant (`min_temp`, `options`, `hvac_modes`…).
//! Un service inconnu, ou un appareil qui n'annonce rien, n'est pas bloqué :
//! l'intégration garde ses propres vérifications.

use serde_json::Value;

use crate::{Attributes, State};

/// Pourquoi un ordre est refusé, en français, pour la personne.
pub type Refusal = String;

/// Un nombre fini, ou un message d'erreur.
fn as_number(value: &Value, label: &str) -> Result<f64, Refusal> {
    value
        .as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| format!("{label} : un nombre est attendu"))
}

fn attr_number(state: &State, key: &str) -> Option<f64> {
    state
        .attributes()
        .get(key)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
}

fn attr_list<'a>(state: &'a State, key: &str) -> Option<Vec<&'a str>> {
    let list = state.attributes().get(key)?.as_array()?;
    Some(list.iter().filter_map(Value::as_str).collect())
}

/// « 21,5 » plutôt que « 21.5 ».
fn show(n: f64) -> String {
    let text = if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{n:.0}")
    } else {
        format!("{n}")
    };
    text.replace('.', ",")
}

/// `field` (facultatif sauf si `required`) doit être un nombre entre `min`
/// et `max`.
fn in_range(
    data: &Attributes,
    field: &str,
    label: &str,
    (min, max): (f64, f64),
    required: bool,
) -> Result<(), Refusal> {
    let Some(value) = data.get(field) else {
        return if required {
            Err(format!("{label} : valeur manquante (« {field} »)"))
        } else {
            Ok(())
        };
    };
    let n = as_number(value, label)?;
    if n < min || n > max {
        return Err(format!(
            "{label} entre {} et {} (reçu {})",
            show(min),
            show(max),
            show(n)
        ));
    }
    Ok(())
}

/// Les bornes annoncées par l'appareil, sinon celles par défaut.
fn bounds(state: &State, (min_key, max_key): (&str, &str), default: (f64, f64)) -> (f64, f64) {
    let min = attr_number(state, min_key).unwrap_or(default.0);
    let max = attr_number(state, max_key).unwrap_or(default.1);
    if min <= max { (min, max) } else { default }
}

/// `field` doit être l'un des choix annoncés dans `list_key` (si
/// l'appareil en annonce).
fn one_of(
    state: &State,
    data: &Attributes,
    field: &str,
    list_key: &str,
    label: &str,
    required: bool,
) -> Result<(), Refusal> {
    let Some(value) = data.get(field) else {
        return if required {
            Err(format!("{label} : valeur manquante (« {field} »)"))
        } else {
            Ok(())
        };
    };
    let wanted = value
        .as_str()
        .ok_or_else(|| format!("{label} : un texte est attendu"))?;
    let Some(choices) = attr_list(state, list_key) else {
        return Ok(());
    };
    if choices.contains(&wanted) {
        return Ok(());
    }
    let shown: Vec<&str> = choices.iter().take(8).copied().collect();
    let more = if choices.len() > shown.len() {
        ", …"
    } else {
        ""
    };
    Err(format!(
        "{label} « {} » inconnu : {}{more}",
        wanted.chars().take(40).collect::<String>(),
        shown.join(", ")
    ))
}

/// Vérifie `domain.service` sur l'appareil `state` avec `data`. Ne vérifie
/// que les services de son propre domaine (`light.turn_on` sur une lampe).
pub fn check(domain: &str, service: &str, data: &Attributes, state: &State) -> Result<(), Refusal> {
    if state.entity_id().domain() != domain {
        return Ok(());
    }
    match (domain, service) {
        ("light", "turn_on") => {
            in_range(data, "brightness", "luminosité", (0.0, 255.0), false)?;
            in_range(data, "brightness_pct", "luminosité", (0.0, 100.0), false)?;
            in_range(
                data,
                "color_temp_kelvin",
                "température de couleur",
                bounds(
                    state,
                    ("min_color_temp_kelvin", "max_color_temp_kelvin"),
                    (1000.0, 12_000.0),
                ),
                false,
            )?;
            in_range(data, "transition", "transition", (0.0, 300.0), false)
        }
        ("cover", "set_cover_position") | ("valve", "set_valve_position") => {
            in_range(data, "position", "position", (0.0, 100.0), true)
        }
        ("cover", "set_cover_tilt_position") => {
            in_range(data, "tilt_position", "inclinaison", (0.0, 100.0), true)
        }
        ("climate" | "water_heater", "set_temperature") => {
            let range = bounds(state, ("min_temp", "max_temp"), (-50.0, 150.0));
            let label = "consigne";
            let has_target =
                data.contains_key("temperature") || data.contains_key("target_temp_low");
            if !has_target {
                return Err(format!("{label} : valeur manquante (« temperature »)"));
            }
            in_range(data, "temperature", label, range, false)?;
            in_range(data, "target_temp_low", label, range, false)?;
            in_range(data, "target_temp_high", label, range, false)?;
            if domain == "climate" {
                one_of(state, data, "hvac_mode", "hvac_modes", "mode", false)?;
            }
            Ok(())
        }
        ("climate", "set_hvac_mode") => {
            one_of(state, data, "hvac_mode", "hvac_modes", "mode", true)
        }
        ("climate" | "fan" | "humidifier", "set_preset_mode") => one_of(
            state,
            data,
            "preset_mode",
            "preset_modes",
            "préréglage",
            true,
        ),
        ("climate" | "fan", "set_fan_mode") => {
            one_of(state, data, "fan_mode", "fan_modes", "ventilation", true)
        }
        ("water_heater", "set_operation_mode") => one_of(
            state,
            data,
            "operation_mode",
            "operation_list",
            "mode",
            true,
        ),
        ("fan", "set_percentage") => {
            in_range(data, "percentage", "vitesse (%)", (0.0, 100.0), true)
        }
        ("fan", "turn_on") => {
            in_range(data, "percentage", "vitesse (%)", (0.0, 100.0), false)?;
            one_of(
                state,
                data,
                "preset_mode",
                "preset_modes",
                "préréglage",
                false,
            )
        }
        ("humidifier", "set_humidity") => in_range(
            data,
            "humidity",
            "humidité visée (%)",
            bounds(state, ("min_humidity", "max_humidity"), (0.0, 100.0)),
            true,
        ),
        ("number" | "input_number", "set_value") => in_range(
            data,
            "value",
            "valeur",
            bounds(state, ("min", "max"), (f64::MIN, f64::MAX)),
            true,
        ),
        ("select" | "input_select", "select_option") => {
            one_of(state, data, "option", "options", "choix", true)
        }
        ("media_player", "volume_set") => {
            in_range(data, "volume_level", "volume (0 à 1)", (0.0, 1.0), true)
        }
        // Un favori Sonos (étape 91) parmi ceux de l'enceinte.
        ("media_player", "select_source") => {
            one_of(state, data, "source", "source_list", "favori", true)
        }
        // Une annonce (étape 73) : le volume en %, un message court.
        ("tts", "speak") => {
            in_range(
                data,
                "volume",
                "volume de l'annonce (%)",
                (0.0, 100.0),
                false,
            )?;
            if let Some(message) = data.get("message").and_then(Value::as_str)
                && message.chars().count() > 500
            {
                return Err("annonce : 500 caractères au plus".to_owned());
            }
            Ok(())
        }
        ("text" | "input_text", "set_value") => {
            let value = data
                .get("value")
                .ok_or_else(|| "texte : valeur manquante (« value »)".to_owned())?
                .as_str()
                .ok_or_else(|| "texte : un texte est attendu".to_owned())?;
            let length = u32::try_from(value.chars().count()).map_or(f64::MAX, f64::from);
            let (min, max) = bounds(state, ("min", "max"), (0.0, 255.0));
            if length < min || length > max {
                return Err(format!(
                    "texte : entre {} et {} caractères (reçu {})",
                    show(min),
                    show(max),
                    show(length)
                ));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;
    use serde_json::json;

    fn device(id: &str, attributes: Value) -> State {
        let core = crate::Core::new();
        let states = &core.states;
        let id: crate::EntityId = id.parse().unwrap();
        let Value::Object(attributes) = attributes else {
            panic!("un objet est attendu");
        };
        states.set(id.clone(), "on", attributes).unwrap();
        states.get(&id).unwrap()
    }

    fn data(value: Value) -> Attributes {
        match value {
            Value::Object(map) => map,
            _ => Attributes::new(),
        }
    }

    #[test]
    fn values_must_be_in_the_range_the_device_announces() {
        let radiateur = device(
            "climate.salon",
            json!({"min_temp": 7, "max_temp": 30, "hvac_modes": ["off", "heat"]}),
        );
        assert!(
            check(
                "climate",
                "set_temperature",
                &data(json!({"temperature": 21.5})),
                &radiateur
            )
            .is_ok()
        );
        assert_eq!(
            check(
                "climate",
                "set_temperature",
                &data(json!({"temperature": 40})),
                &radiateur
            ),
            Err("consigne entre 7 et 30 (reçu 40)".into())
        );
        assert_eq!(
            check(
                "climate",
                "set_temperature",
                &data(json!({"temperature": 6.5})),
                &radiateur
            ),
            Err("consigne entre 7 et 30 (reçu 6,5)".into())
        );
        assert_eq!(
            check(
                "climate",
                "set_temperature",
                &data(json!({"temperature": "chaud"})),
                &radiateur
            ),
            Err("consigne : un nombre est attendu".into())
        );
        assert!(check("climate", "set_temperature", &data(json!({})), &radiateur).is_err());
        assert_eq!(
            check(
                "climate",
                "set_hvac_mode",
                &data(json!({"hvac_mode": "cool"})),
                &radiateur
            ),
            Err("mode « cool » inconnu : off, heat".into())
        );
        assert!(
            check(
                "climate",
                "set_hvac_mode",
                &data(json!({"hvac_mode": "heat"})),
                &radiateur
            )
            .is_ok()
        );

        let lampe = device("light.salon", json!({}));
        assert!(
            check(
                "light",
                "turn_on",
                &data(json!({"brightness": 128})),
                &lampe
            )
            .is_ok()
        );
        assert!(check("light", "turn_on", &data(json!({})), &lampe).is_ok());
        assert_eq!(
            check(
                "light",
                "turn_on",
                &data(json!({"brightness": 300})),
                &lampe
            ),
            Err("luminosité entre 0 et 255 (reçu 300)".into())
        );

        let volet = device("cover.chambre", json!({}));
        assert!(
            check(
                "cover",
                "set_cover_position",
                &data(json!({"position": 0})),
                &volet
            )
            .is_ok()
        );
        assert!(
            check(
                "cover",
                "set_cover_position",
                &data(json!({"position": 101})),
                &volet
            )
            .is_err()
        );
        assert!(check("cover", "set_cover_position", &data(json!({})), &volet).is_err());
        assert!(check("cover", "close_cover", &data(json!({})), &volet).is_ok());
    }

    #[test]
    fn choices_and_texts() {
        let liste = device("input_select.mode", json!({"options": ["Jour", "Nuit"]}));
        assert!(
            check(
                "input_select",
                "select_option",
                &data(json!({"option": "Nuit"})),
                &liste
            )
            .is_ok()
        );
        assert_eq!(
            check(
                "input_select",
                "select_option",
                &data(json!({"option": "Fête"})),
                &liste
            ),
            Err("choix « Fête » inconnu : Jour, Nuit".into())
        );
        let nombre = device("number.seuil", json!({"min": 1, "max": 5}));
        assert!(check("number", "set_value", &data(json!({"value": 5})), &nombre).is_ok());
        assert!(check("number", "set_value", &data(json!({"value": 9})), &nombre).is_err());
        let texte = device("input_text.message", json!({"min": 1, "max": 5}));
        assert!(
            check(
                "input_text",
                "set_value",
                &data(json!({"value": "Salut"})),
                &texte
            )
            .is_ok()
        );
        assert_eq!(
            check(
                "input_text",
                "set_value",
                &data(json!({"value": "Bonjour"})),
                &texte
            ),
            Err("texte : entre 1 et 5 caractères (reçu 7)".into())
        );
        // Sans limites annoncées : seulement le type.
        let libre = device("select.libre", json!({}));
        assert!(
            check(
                "select",
                "select_option",
                &data(json!({"option": "x"})),
                &libre
            )
            .is_ok()
        );
        assert!(
            check(
                "select",
                "select_option",
                &data(json!({"option": 3})),
                &libre
            )
            .is_err()
        );
    }

    #[test]
    fn speaker_volume_and_announcements() {
        let speaker = device("media_player.salon", json!({}));
        assert!(
            check(
                "media_player",
                "volume_set",
                &data(json!({"volume_level": 0.4})),
                &speaker
            )
            .is_ok()
        );
        let error = check(
            "media_player",
            "volume_set",
            &data(json!({"volume_level": 40})),
            &speaker,
        )
        .unwrap_err();
        assert!(error.contains("entre 0 et 1"), "{error}");
        assert!(check("media_player", "volume_set", &data(json!({})), &speaker).is_err());
        let favorites = device(
            "media_player.salle_a_manger",
            json!({"source_list": ["Ma playlist", "FIP"]}),
        );
        let select = |source: Value| {
            check(
                "media_player",
                "select_source",
                &data(json!({ "source": source })),
                &favorites,
            )
        };
        assert!(select(json!("FIP")).is_ok());
        let error = select(json!("Absente")).unwrap_err();
        assert!(error.contains("favori « Absente » inconnu"), "{error}");
        assert!(
            check(
                "media_player",
                "select_source",
                &data(json!({})),
                &favorites
            )
            .is_err()
        );
        let tts = device("tts.annonces", json!({}));
        assert!(
            check(
                "tts",
                "speak",
                &data(json!({"message": "Bonjour", "volume": 40})),
                &tts
            )
            .is_ok()
        );
        assert!(check("tts", "speak", &data(json!({"volume": 140})), &tts).is_err());
        assert!(
            check(
                "tts",
                "speak",
                &data(json!({"message": "a".repeat(501)})),
                &tts
            )
            .is_err()
        );
    }

    #[test]
    fn unknown_services_and_other_domains_are_left_to_the_integration() {
        let lampe = device("light.salon", json!({}));
        assert!(check("light", "clignoter", &data(json!({"x": 1})), &lampe).is_ok());
        // `homeassistant.turn_on` sur une lampe : pas de règle ici.
        assert!(
            check(
                "homeassistant",
                "turn_on",
                &data(json!({"brightness": 999})),
                &lampe
            )
            .is_ok()
        );
        // Bornes incohérentes annoncées : celles par défaut.
        let bizarre = device("climate.x", json!({"min_temp": 30, "max_temp": 7}));
        assert!(
            check(
                "climate",
                "set_temperature",
                &data(json!({"temperature": 20})),
                &bizarre
            )
            .is_ok()
        );
    }
}
