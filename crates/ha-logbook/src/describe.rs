//! Les phrases du journal : « s'est ouverte », « est à la maison »…
//! Aucune base, aucune attente ici : faciles à tester.

use ha_core::{Attributes, EntityId, State};
use serde_json::Value;

/// Le nom affiché d'une entité : son `friendly_name`, sinon son identifiant.
#[must_use]
pub fn name_of(entity_id: &EntityId, state: Option<&State>) -> String {
    state
        .and_then(|s| s.attributes().get("friendly_name"))
        .and_then(Value::as_str)
        .map_or_else(|| entity_id.as_str().to_owned(), str::to_owned)
}

/// La phrase pour un changement d'état, ou `None` s'il ne mérite pas d'être
/// noté : création au démarrage, simple changement d'attributs, mesure
/// (température…), météo, minuteur (ses événements sont plus précis).
#[must_use]
pub fn describe_change(old: Option<&State>, new: Option<&State>) -> Option<String> {
    let (old, new) = (old?, new?);
    let phrase = phrase(old, new)?;
    Some(if is_feminine(&name_of(new.entity_id(), Some(new))) {
        agree(phrase)
    } else {
        phrase
    })
}

fn phrase(old: &State, new: &State) -> Option<String> {
    let value = new.state();
    // Un appareil qui annonce son état après un redémarrage (« inconnu » →
    // « désarmée ») : rien ne s'est passé dans la maison.
    if old.state() == value || value == "unknown" || old.state() == "unknown" {
        return None;
    }
    let attributes = new.attributes();
    let domain = new.entity_id().domain();
    if value == "unavailable" {
        return Some("est indisponible".into());
    }
    let on_off = |on: &str, off: &str| match value {
        "on" => Some(on.to_owned()),
        "off" => Some(off.to_owned()),
        _ => Some(format!("passe à « {value} »")),
    };
    match domain {
        // Mesures et choses qui changent sans cesse : l'historique s'en charge.
        // Une liste : son état n'est qu'un nombre d'éléments.
        // Les notifications : elles s'affichent déjà sur le tableau de bord.
        "weather" | "timer" | "todo" | "persistent_notification" => None,
        // L'agenda : le rendez-vous qui commence, ou celui qui se termine.
        "calendar" => {
            let (state, verb) = match value {
                "on" => (new, "début de"),
                "off" => (old, "fin de"),
                _ => return None,
            };
            Some(match text(state.attributes(), "message") {
                Some(message) => format!("{verb} « {message} »"),
                None => format!("{verb} rendez-vous"),
            })
        }
        "sensor" if attributes.contains_key("unit_of_measurement") => None,
        "sensor" if text(attributes, "device_class") == Some("timestamp") => None,
        "light" | "switch" | "fan" | "humidifier" => on_off("s'est allumé", "s'est éteint"),
        "siren" => on_off("sonne !", "s'est arrêté"),
        "lock" => Some(
            match value {
                "locked" => "s'est verrouillé",
                "unlocked" => "s'est déverrouillé",
                "locking" => "se verrouille",
                "unlocking" => "se déverrouille",
                "jammed" => "est bloqué !",
                "open" => "s'est ouvert",
                "opening" => "s'ouvre",
                _ => return Some(format!("passe à « {value} »")),
            }
            .into(),
        ),
        "vacuum" => Some(
            match value {
                "cleaning" => "fait le ménage",
                "docked" => "est rentré à sa base",
                "paused" => "s'est mis en pause",
                "idle" => "s'est arrêté",
                "returning" => "rentre à sa base",
                "error" => "est en erreur !",
                _ => return Some(format!("passe à « {value} »")),
            }
            .into(),
        ),
        // Une enceinte (étape 73) : « Salon joue », « Chambre s'est arrêtée ».
        "media_player" => Some(
            match value {
                "playing" => "joue",
                "paused" => "est en pause",
                "idle" => "s'est arrêté",
                _ => return Some(format!("passe à « {value} »")),
            }
            .into(),
        ),
        // Une annonce : son texte.
        "tts" => text(attributes, "dernier_message").map(|message| format!("« {message} »")),
        "water_heater" => Some(match value {
            "off" => "s'est arrêté".into(),
            mode => format!("passe en mode {}", water_heater_mode(mode)),
        }),
        "input_boolean" => on_off("a été activé", "a été désactivé"),
        "binary_sensor" => {
            let (on, off) = binary_words(text(attributes, "device_class"));
            on_off(on, off)
        }
        "cover" | "valve" => Some(
            match value {
                "open" => "s'est ouvert",
                "closed" => "s'est fermé",
                "opening" => "s'ouvre",
                "closing" => "se ferme",
                _ => return Some(format!("passe à « {value} »")),
            }
            .into(),
        ),
        "person" | "device_tracker" => Some(
            match value {
                "home" => "est à la maison",
                "not_home" => "n'est plus à la maison",
                _ => return Some(format!("est à « {value} »")),
            }
            .into(),
        ),
        "zone" => Some(match value.parse::<u32>() {
            Ok(0) => "plus personne à la maison".into(),
            Ok(1) => "1 personne à la maison".into(),
            Ok(count) => format!("{count} personnes à la maison"),
            Err(_) => format!("passe à « {value} »"),
        }),
        "sun" => Some(
            match value {
                "above_horizon" => "lever du soleil",
                "below_horizon" => "coucher du soleil",
                _ => return None,
            }
            .into(),
        ),
        "climate" => Some(
            match value {
                "heat" => "chauffage en marche",
                "off" => "chauffage arrêté",
                _ => return Some(format!("passe en mode « {value} »")),
            }
            .into(),
        ),
        // Leur état est une date : celle du dernier usage.
        "scene" => Some("activée".into()),
        // Leur état est l'heure du dernier appui.
        "input_button" | "button" => Some("a été appuyé".into()),
        "notify" => Some("notification envoyée".into()),
        "script" => (value == "on").then(|| "lancé".into()),
        // Une télécommande : laquelle de ses touches.
        "event" => text(attributes, "event_type").map(|kind| format!("signale « {kind} »")),
        // Un texte « mot de passe » n'est jamais écrit dans le journal.
        "text" | "input_text" if text(attributes, "mode") == Some("password") => {
            Some("a été modifié".into())
        }
        // Une heure (« 07:30:00 ») s'écrit sans les secondes.
        "input_datetime" => Some(format!(
            "réglé à {}",
            value
                .strip_suffix(":00")
                .filter(|_| value.len() > 5)
                .unwrap_or(value)
        )),
        "alarm_control_panel" => Some(
            match value {
                "disarmed" => "a été désarmé",
                "armed_home" => "est en mode présence",
                "armed_away" => "est en mode absence",
                "armed_night" => "est en mode nuit",
                "armed_vacation" => "est en mode vacances",
                "armed_custom_bypass" => "est en mode personnalisé",
                "pending" => "va sonner (délai d'entrée)",
                "triggered" => "sonne : intrusion !",
                "arming" => "s'arme…",
                "disarming" => "se désarme…",
                _ => return Some(format!("passe à « {value} »")),
            }
            .into(),
        ),
        // Une plage horaire (« Chauffage du matin »).
        "schedule" => on_off("commence", "se termine"),
        // Un rappel insistant (étape 69) : « Garage ouvert : compris par
        // david ».
        "alert" => Some(match value {
            "on" => "rappel en cours".into(),
            "off" => match text(attributes, "compris_par") {
                Some(who) => format!("compris par {who}"),
                None => "compris".into(),
            },
            "idle" => "c'est réglé".into(),
            _ => format!("passe à « {value} »"),
        }),
        "input_number" | "number" | "counter" => {
            Some(match text(attributes, "unit_of_measurement") {
                Some(unit) => format!("réglé à {value} {unit}"),
                None => format!("réglé à {value}"),
            })
        }
        _ => Some(format!("passe à « {value} »")),
    }
}

/// Les noms féminins les plus courants dans une maison. Sans eux, le journal
/// écrirait « Porte d'entrée s'est ouvert ». La même liste existe dans
/// l'interface (`app.js`, `FEMININE`) : les garder pareilles.
const FEMININE: &[&str] = &[
    "alarme",
    "allée",
    "ampoule",
    "applique",
    "baie",
    "barrière",
    "bouilloire",
    "box",
    "buanderie",
    "cafetière",
    "caméra",
    "cave",
    "chambre",
    "chaudière",
    "climatisation",
    "clim",
    "cour",
    "cuisine",
    "électrovanne",
    "enceinte",
    "entrée",
    "fenêtre",
    "friteuse",
    "guirlande",
    "hotte",
    "imprimante",
    "lampe",
    "lucarne",
    "lumière",
    "machine",
    "multiprise",
    "piscine",
    "plaque",
    "pompe",
    "porte",
    "prise",
    "salle",
    "serrure",
    "sirène",
    "sonde",
    "sonnette",
    "station",
    "suspension",
    "télé",
    "télévision",
    "terrasse",
    "trappe",
    "vanne",
    "véranda",
    "ventilation",
    "verrière",
    "veilleuse",
    "vmc",
];

/// Le nom désigne-t-il une chose féminine ? On regarde son premier mot
/// (« Porte du garage » → « porte »), ou l'article s'il y en a un.
/// Un identifiant comme `binary_sensor.porte_entree` compte aussi.
fn is_feminine(name: &str) -> bool {
    // Sans nom lisible, c'est l'identifiant : on garde ce qui suit le point.
    let name = if name.contains(char::is_whitespace) {
        name
    } else {
        name.rsplit_once('.').map_or(name, |(_, object)| object)
    };
    let lower = name.to_lowercase();
    let mut words = lower
        .split(|c: char| c.is_whitespace() || matches!(c, '_' | '-' | '\'' | '’'))
        .filter(|word| !word.is_empty());
    match words.next() {
        Some("la" | "une") => true,
        Some("le" | "un" | "l") => words.next().is_some_and(|word| FEMININE.contains(&word)),
        Some(word) => FEMININE.contains(&word),
        None => false,
    }
}

/// Accorde le participe au féminin : « s'est ouvert » → « s'est ouverte ».
/// Seules les phrases « s'est … » et « a été … » s'accordent ; « détecte de
/// la fumée » ne change pas.
fn agree(mut phrase: String) -> String {
    let agrees = (phrase.starts_with("s'est ") || phrase.starts_with("a été "))
        && (phrase.ends_with('é') || phrase.ends_with('t'));
    if agrees {
        phrase.push('e');
    }
    phrase
}

/// Les mots d'un détecteur, d'après sa sorte (`device_class`).
fn binary_words(device_class: Option<&str>) -> (&'static str, &'static str) {
    match device_class {
        Some("door" | "window" | "garage_door" | "opening") => ("s'est ouvert", "s'est fermé"),
        Some("motion" | "occupancy" | "presence") => {
            ("détecte un mouvement", "ne détecte plus de mouvement")
        }
        Some("smoke") => ("détecte de la fumée !", "ne détecte plus de fumée"),
        Some("moisture") => ("détecte une fuite d'eau !", "ne détecte plus de fuite"),
        Some("connectivity") => ("s'est connecté", "s'est déconnecté"),
        _ => ("s'est activé", "s'est désactivé"),
    }
}

/// Le mode d'un chauffe-eau, en français (« pompe à chaleur »).
fn water_heater_mode(mode: &str) -> String {
    match mode {
        "eco" => "éco".into(),
        "electric" => "électrique".into(),
        "gas" => "gaz".into(),
        "heat_pump" => "pompe à chaleur".into(),
        "high_demand" => "forte demande".into(),
        "performance" => "performance".into(),
        other => format!("« {other} »"),
    }
}

/// La phrase pour un événement d'un minuteur (`timer.finished`…).
#[must_use]
pub fn describe_timer_event(event_type: &str) -> Option<&'static str> {
    Some(match event_type {
        "timer.started" => "démarré",
        "timer.restarted" => "relancé",
        "timer.paused" => "en pause",
        "timer.cancelled" => "annulé",
        "timer.finished" => "terminé",
        _ => return None,
    })
}

fn text<'a>(attributes: &'a Attributes, key: &str) -> Option<&'a str> {
    attributes.get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ha_core::Core;
    use serde_json::json;

    /// Un état, tel que le noyau le fabrique.
    fn state(entity: &str, value: &str, attributes: Value) -> State {
        let Value::Object(attributes) = attributes else {
            panic!("un objet est attendu")
        };
        let core = Core::new();
        let id: EntityId = entity.parse().unwrap();
        core.states.set(id.clone(), value, attributes).unwrap();
        core.states.get(&id).unwrap()
    }

    fn change(entity: &str, from: &str, to: &str, attributes: Value) -> Option<String> {
        let old = state(entity, from, attributes.clone());
        let new = state(entity, to, attributes);
        describe_change(Some(&old), Some(&new))
    }

    #[test]
    fn changes_become_sentences() {
        let none = json!({});
        for (entity, from, to, attributes, expected) in [
            ("light.salon", "off", "on", &none, "s'est allumé"),
            ("switch.prise", "on", "off", &none, "s'est éteinte"),
            ("input_boolean.vacances", "off", "on", &none, "a été activé"),
            (
                "binary_sensor.portail",
                "off",
                "on",
                &json!({"device_class": "door"}),
                "s'est ouvert",
            ),
            (
                "binary_sensor.couloir",
                "on",
                "off",
                &json!({"device_class": "motion"}),
                "ne détecte plus de mouvement",
            ),
            (
                "binary_sensor.fumee",
                "off",
                "on",
                &json!({"device_class": "smoke"}),
                "détecte de la fumée !",
            ),
            ("cover.salon", "open", "closing", &none, "se ferme"),
            ("media_player.salon", "idle", "playing", &none, "joue"),
            (
                "media_player.cuisine",
                "playing",
                "idle",
                &json!({"friendly_name": "Cuisine"}),
                "s'est arrêtée",
            ),
            (
                "tts.annonces",
                "2026-10-21T10:00:00Z",
                "2026-10-21T10:05:00Z",
                &json!({"dernier_message": "Le repas est prêt"}),
                "« Le repas est prêt »",
            ),
            ("person.david", "not_home", "home", &none, "est à la maison"),
            ("zone.home", "1", "0", &none, "plus personne à la maison"),
            ("zone.home", "1", "2", &none, "2 personnes à la maison"),
            (
                "sun.sun",
                "below_horizon",
                "above_horizon",
                &none,
                "lever du soleil",
            ),
            ("climate.salon", "off", "heat", &none, "chauffage en marche"),
            (
                "input_select.mode",
                "Normal",
                "Nuit",
                &none,
                "passe à « Nuit »",
            ),
            (
                "input_number.consigne",
                "19",
                "21.5",
                &json!({"unit_of_measurement": "°C"}),
                "réglé à 21.5 °C",
            ),
            (
                "light.salon",
                "on",
                "unavailable",
                &none,
                "est indisponible",
            ),
            (
                "sensor.alarme",
                "disarmed",
                "armed",
                &json!({"device_class": "alarm"}),
                "passe à « armed »",
            ),
        ] {
            assert_eq!(
                change(entity, from, to, attributes.clone()).as_deref(),
                Some(expected),
                "{entity} {from} → {to}"
            );
        }
    }

    #[test]
    fn noise_is_left_out() {
        let none = json!({});
        for (entity, from, to, attributes) in [
            (
                "sensor.temperature",
                "19.5",
                "19.6",
                json!({"unit_of_measurement": "°C"}),
            ),
            (
                "sensor.derniere_sauvegarde",
                "a",
                "b",
                json!({"device_class": "timestamp"}),
            ),
            ("weather.maison", "sunny", "rainy", none.clone()),
            ("timer.lessive", "idle", "active", none.clone()),
            ("light.salon", "on", "on", none.clone()),
            ("light.salon", "unavailable", "unknown", none.clone()),
            (
                "alarm_control_panel.maison",
                "unknown",
                "disarmed",
                none.clone(),
            ),
            ("script.depart", "on", "off", none.clone()),
        ] {
            assert_eq!(
                change(entity, from, to, attributes),
                None,
                "{entity} {from} → {to}"
            );
        }
        // Création (démarrage) et suppression : rien.
        let light = state("light.salon", "on", json!({}));
        assert_eq!(describe_change(None, Some(&light)), None);
        assert_eq!(describe_change(Some(&light), None), None);
    }

    #[test]
    fn new_device_types_have_sentences() {
        let none = json!({});
        for (entity, from, to, attributes, expected) in [
            (
                "lock.entree",
                "locked",
                "unlocked",
                json!({"friendly_name": "Porte d'entrée"}),
                "s'est déverrouillée",
            ),
            (
                "lock.garage",
                "unlocked",
                "jammed",
                json!({"friendly_name": "Verrou du garage"}),
                "est bloqué !",
            ),
            (
                "valve.arrosage",
                "closed",
                "open",
                json!({"friendly_name": "Vanne d'arrosage"}),
                "s'est ouverte",
            ),
            (
                "vacuum.robot",
                "cleaning",
                "docked",
                none.clone(),
                "est rentré à sa base",
            ),
            (
                "water_heater.ballon",
                "eco",
                "heat_pump",
                none.clone(),
                "passe en mode pompe à chaleur",
            ),
            (
                "siren.alarme",
                "off",
                "on",
                json!({"friendly_name": "Sirène"}),
                "sonne !",
            ),
            (
                "siren.alarme",
                "on",
                "off",
                json!({"friendly_name": "Sirène"}),
                "s'est arrêtée",
            ),
            (
                "humidifier.salon",
                "off",
                "on",
                none.clone(),
                "s'est allumé",
            ),
            (
                "alarm_control_panel.maison",
                "armed_away",
                "disarmed",
                json!({"friendly_name": "Alarme"}),
                "a été désarmée",
            ),
            (
                "event.telecommande",
                "2026-10-15T05:00:00Z",
                "2026-10-15T05:00:09Z",
                json!({"event_type": "double"}),
                "signale « double »",
            ),
            (
                "text.code_portail",
                "a",
                "b",
                json!({"mode": "password"}),
                "a été modifié",
            ),
            (
                "number.luminosite_led",
                "100",
                "200",
                none.clone(),
                "réglé à 200",
            ),
            (
                "schedule.chauffage_matin",
                "off",
                "on",
                none.clone(),
                "commence",
            ),
            (
                "input_datetime.reveil",
                "07:00:00",
                "06:45:00",
                none.clone(),
                "réglé à 06:45",
            ),
            (
                "input_text.code",
                "a",
                "b",
                json!({"mode": "password"}),
                "a été modifié",
            ),
            ("counter.lessives", "2", "3", none.clone(), "réglé à 3"),
            (
                "alert.garage_ouvert",
                "idle",
                "on",
                none.clone(),
                "rappel en cours",
            ),
            (
                "alert.garage_ouvert",
                "on",
                "off",
                json!({"compris_par": "david"}),
                "compris par david",
            ),
            (
                "alert.fenetre",
                "off",
                "idle",
                json!({"friendly_name": "Fenêtre de la chambre"}),
                "c'est réglé",
            ),
            (
                "calendar.maison",
                "off",
                "on",
                json!({"message": "Dentiste"}),
                "début de « Dentiste »",
            ),
        ] {
            assert_eq!(
                change(entity, from, to, attributes).as_deref(),
                Some(expected),
                "{entity} {from} → {to}"
            );
        }
    }

    #[test]
    fn participles_agree_with_feminine_names() {
        for (entity, from, to, attributes, expected) in [
            (
                "binary_sensor.porte_entree",
                "off",
                "on",
                json!({"device_class": "door"}),
                "s'est ouverte",
            ),
            (
                "cover.garage",
                "open",
                "closed",
                json!({"friendly_name": "Porte du garage"}),
                "s'est fermée",
            ),
            (
                "light.salon",
                "on",
                "off",
                json!({"friendly_name": "Lampe de chevet"}),
                "s'est éteinte",
            ),
            (
                "switch.cafe",
                "off",
                "on",
                json!({"friendly_name": "La cafetière"}),
                "s'est allumée",
            ),
            (
                "light.plafond",
                "off",
                "on",
                json!({"friendly_name": "Plafonnier"}),
                "s'est allumé",
            ),
            // Une phrase qui ne s'accorde pas reste telle quelle.
            (
                "binary_sensor.sonde",
                "off",
                "on",
                json!({"device_class": "smoke", "friendly_name": "Sonde du couloir"}),
                "détecte de la fumée !",
            ),
            ("cover.porte", "closed", "opening", json!({}), "s'ouvre"),
        ] {
            assert_eq!(
                change(entity, from, to, attributes).as_deref(),
                Some(expected),
                "{entity} {from} → {to}"
            );
        }
    }

    #[test]
    fn names_and_timer_events() {
        let id: EntityId = "light.salon".parse().unwrap();
        let named = state("light.salon", "on", json!({"friendly_name": "Salon"}));
        assert_eq!(name_of(&id, Some(&named)), "Salon");
        assert_eq!(name_of(&id, None), "light.salon");
        assert_eq!(describe_timer_event("timer.finished"), Some("terminé"));
        assert_eq!(describe_timer_event("timer.inconnu"), None);
    }
}
