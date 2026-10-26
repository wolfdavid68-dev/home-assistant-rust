//! La **confirmation après un ordre** (étape 95) : Assist ne dit « C'est
//! fait » que si l'appareil a vraiment changé d'état.
//!
//! Après l'envoi des ordres d'une phrase, on attend au plus
//! [`CONFIRM_WITHIN`] que chaque appareil visé montre l'état attendu
//! (allumé, éteint, luminosité, volet ouvert ou en train de s'ouvrir, à sa
//! position…), en écoutant les changements d'état sur le bus : pas de
//! relecture en boucle. On répond dès que tout est confirmé ; un appareil
//! déjà dans le bon état ne fait pas attendre.
//!
//! On n'attend que là où l'état réel arrive vite et ne peut pas venir de
//! nous-mêmes (voir [`QUICK`] et `docs/confirmation.md`) :
//!
//! - **Hue** (flux d'événements du pont), **Sonos** (relu aussitôt après
//!   l'ordre) et **MQTT** avec un sujet d'état (Zigbee2MQTT, Shelly) : l'état
//!   réel arrive en moins d'une seconde ;
//! - **MQTT sans sujet d'état** : l'état est supposé par notre propre code
//!   juste après l'ordre (`assumed_state`), ce n'est donc jamais une
//!   confirmation : on n'attend pas ;
//! - **Somfy** (volets : 15 à 30 s de course, pas d'état « en mouvement »),
//!   **chauffage Somfy** (relu 5 s après l'ordre) et **SmartThings** (relu
//!   2 s après la réponse du nuage) : l'état ne peut pas arriver en 3 s.
//!   Leur passerelle répond déjà si elle a accepté l'ordre (`Outcome::Done`) :
//!   c'est la confirmation qu'on garde ;
//! - les appareils du serveur lui-même (entrées, thermostat virtuel, scènes,
//!   scripts, groupes) : leur état est écrit au moment de l'ordre.
//!
//! Dans tous les cas, un appareil indisponible est signalé.

use std::sync::Arc;
use std::time::Duration;

use ha_core::{Attributes, EntityId, Event, State, StateMachine};
use serde_json::Value;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

/// L'attente au plus, après les ordres d'une phrase.
pub(crate) const CONFIRM_WITHIN: Duration = Duration::from_secs(3);

/// Les intégrations dont l'état réel arrive en moins d'une seconde après un
/// ordre, et jamais écrit d'avance par notre code (sauf MQTT sans sujet
/// d'état, marqué `assumed_state`).
const QUICK: &[&str] = &["hue", "sonos", "mqtt"];

/// Un appareil à surveiller après un ordre.
#[derive(Debug)]
pub(crate) struct Watch {
    pub entity_id: EntityId,
    /// Le nom dit dans la réponse (« Volet chambre »).
    pub name: String,
    /// L'état attendu ; `None` : on ne l'attend pas (intégration lente, état
    /// supposé…), on vérifie seulement que l'appareil est disponible.
    check: Option<Check>,
}

/// Ce que devient un ordre envoyé mais non confirmé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Unconfirmed {
    /// Pas encore dans l'état attendu au bout de l'attente.
    Late(EntityId, String),
    /// L'appareil est indisponible.
    Unavailable(EntityId, String),
}

impl Unconfirmed {
    pub(crate) fn entity_id(&self) -> &EntityId {
        match self {
            Self::Late(entity_id, _) | Self::Unavailable(entity_id, _) => entity_id,
        }
    }
}

/// L'état attendu.
#[derive(Debug, Clone, PartialEq)]
enum Check {
    /// L'une de ces valeurs.
    States(&'static [&'static str]),
    /// Toute autre valeur que celle-ci (allumer un thermostat).
    Not(&'static str),
    /// Exactement cette valeur (un mode choisi).
    Is(String),
    /// Un attribut proche de cette valeur.
    Near {
        key: &'static str,
        value: f64,
        tolerance: f64,
    },
    /// Un volet à cette position (100 : ouvert, 0 : fermé), ou en route :
    /// « en train de s'ouvrir », ou une position qui a bougé dans le bon
    /// sens depuis l'ordre (certains volets disent « ouvert » à 30 % pendant
    /// leur course). Un volet qui commence à bouger a obéi.
    Cover { target: f64, before: Option<f64> },
}

impl Check {
    fn reached(&self, state: &State) -> bool {
        let value = state.state();
        let number = |key: &str| state.attributes().get(key).and_then(Value::as_f64);
        match self {
            Self::States(accepted) => accepted.contains(&value),
            Self::Not(refused) => value != *refused && !matches!(value, "unknown" | "unavailable"),
            Self::Is(expected) => value == expected,
            Self::Near {
                key,
                value: expected,
                tolerance,
            } => number(key).is_some_and(|got| (got - expected).abs() <= *tolerance),
            Self::Cover { target, before } => {
                if matches!(value, "opening" | "closing") {
                    return true;
                }
                match number("current_position") {
                    Some(position) => {
                        (position - target).abs() <= 2.0
                            || before
                                .is_some_and(|before| (target - before) * (position - before) > 0.0)
                    }
                    // Sans position connue : ouvert ou fermé seulement.
                    None if *target >= 100.0 => value == "open",
                    None if *target <= 0.0 => value == "closed",
                    None => false,
                }
            }
        }
    }
}

/// Ce qu'on surveillera après cet ordre. À appeler **avant** l'envoi (pour
/// savoir ce que « bascule » veut dire). `None` : appareil inconnu.
pub(crate) fn watch(
    states: &StateMachine,
    domain: &str,
    service: &str,
    entity_id: &EntityId,
    data: &Attributes,
    name: &str,
) -> Option<Watch> {
    let current = states.get(entity_id)?;
    let quick = states
        .owner_of(entity_id)
        .is_some_and(|owner| QUICK.contains(&owner.as_str()));
    let assumed = current
        .attributes()
        .get("assumed_state")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let check = if quick && !assumed {
        expectation(domain, service, data, &current)
    } else {
        None
    };
    Some(Watch {
        entity_id: entity_id.clone(),
        name: name.to_owned(),
        check,
    })
}

/// L'état attendu après cet ordre, s'il y en a un clair.
fn expectation(domain: &str, service: &str, data: &Attributes, current: &State) -> Option<Check> {
    let number = |key: &str| data.get(key).and_then(Value::as_f64);
    let toggled = || match current.state() {
        "on" => Some(Check::States(&["off"])),
        "off" => Some(Check::States(&["on"])),
        _ => None,
    };
    let check = match (domain, service) {
        ("light" | "switch" | "fan" | "climate", "turn_off") => Check::States(&["off"]),
        ("light", "turn_on") => {
            // La luminosité revient en 0-255, arrondie par l'appareil.
            let brightness = number("brightness_pct")
                .map(|percent| percent * 255.0 / 100.0)
                .or_else(|| number("brightness"));
            match brightness {
                Some(value) if value > 0.0 => Check::Near {
                    key: "brightness",
                    value,
                    tolerance: 4.0,
                },
                _ => Check::States(&["on"]),
            }
        }
        ("switch" | "fan", "turn_on") => Check::States(&["on"]),
        ("light" | "switch" | "fan", "toggle") => return toggled(),
        ("cover", "open_cover" | "close_cover" | "set_cover_position") => {
            let target = match service {
                "open_cover" => 100.0,
                "close_cover" => 0.0,
                _ => number("position")?,
            };
            let before = current
                .attributes()
                .get("current_position")
                .and_then(Value::as_f64);
            // Une position demandée à un volet qui n'en donne pas : rien à attendre.
            if before.is_none() && (1.0..100.0).contains(&target) {
                return None;
            }
            Check::Cover { target, before }
        }
        ("lock", "lock") => Check::States(&["locked", "locking"]),
        ("lock", "unlock") => Check::States(&["unlocked", "unlocking", "open", "opening"]),
        ("lock", "open") => Check::States(&["open", "opening"]),
        ("media_player", "media_play") => Check::States(&["playing"]),
        ("media_player", "media_pause") => Check::States(&["paused", "idle"]),
        ("media_player", "media_stop") => Check::States(&["idle", "paused", "off"]),
        ("media_player", "volume_set") => Check::Near {
            key: "volume_level",
            value: number("volume_level")?,
            tolerance: 0.02,
        },
        ("climate", "turn_on") => Check::Not("off"),
        ("climate", "set_hvac_mode") => {
            Check::Is(data.get("hvac_mode").and_then(Value::as_str)?.to_owned())
        }
        ("climate", "set_temperature") => Check::Near {
            key: "temperature",
            value: number("temperature")?,
            tolerance: 0.05,
        },
        // Arrêter, régler une vitesse, une source… : pas d'état attendu clair.
        _ => return None,
    };
    Some(check)
}

enum Judgement {
    Reached,
    Waiting,
    Unavailable,
}

fn judge(watch: &Watch, state: Option<&State>) -> Judgement {
    let Some(state) = state else {
        return Judgement::Waiting;
    };
    if state.state() == "unavailable" {
        return Judgement::Unavailable;
    }
    match &watch.check {
        Some(check) if !check.reached(state) => Judgement::Waiting,
        _ => Judgement::Reached,
    }
}

/// Attend que chaque appareil montre l'état attendu, au plus `within`.
/// `events` doit avoir été abonné **avant** l'envoi des ordres, pour ne
/// manquer aucun changement. Renvoie les appareils non confirmés, dans
/// l'ordre des ordres.
pub(crate) async fn wait(
    events: &mut broadcast::Receiver<Arc<Event>>,
    states: &StateMachine,
    watches: Vec<Watch>,
    within: Duration,
) -> Vec<Unconfirmed> {
    let deadline = tokio::time::Instant::now() + within;
    let mut unconfirmed = Vec::new();
    let mut pending = Vec::new();
    // D'abord l'état actuel : déjà bon, rien à attendre.
    for watch in watches {
        match judge(&watch, states.get(&watch.entity_id).as_ref()) {
            Judgement::Reached => {}
            Judgement::Unavailable => {
                unconfirmed.push(Unconfirmed::Unavailable(watch.entity_id, watch.name));
            }
            Judgement::Waiting => pending.push(watch),
        }
    }
    while !pending.is_empty() {
        let event = match tokio::time::timeout_at(deadline, events.recv()).await {
            Err(_) | Ok(Err(RecvError::Closed)) => break,
            Ok(Ok(event)) => Some(event),
            // Des événements perdus : on relit l'état de chaque appareil.
            Ok(Err(RecvError::Lagged(_))) => None,
        };
        pending.retain(|watch| {
            let judgement = match event.as_deref() {
                Some(Event::StateChanged {
                    entity_id,
                    new_state,
                    ..
                }) if *entity_id == watch.entity_id => judge(watch, new_state.as_ref()),
                Some(_) => return true,
                None => judge(watch, states.get(&watch.entity_id).as_ref()),
            };
            match judgement {
                Judgement::Reached => false,
                Judgement::Waiting => true,
                Judgement::Unavailable => {
                    unconfirmed.push(Unconfirmed::Unavailable(
                        watch.entity_id.clone(),
                        watch.name.clone(),
                    ));
                    false
                }
            }
        });
    }
    unconfirmed.extend(
        pending
            .into_iter()
            .map(|watch| Unconfirmed::Late(watch.entity_id, watch.name)),
    );
    unconfirmed
}

/// La phrase pour les ordres non confirmés : « J'ai envoyé l'ordre, mais
/// Volet chambre n'a pas encore répondu. »
pub(crate) fn sentence(unconfirmed: &[Unconfirmed]) -> Option<String> {
    let mut late = Vec::new();
    let mut unavailable = Vec::new();
    for item in unconfirmed {
        match item {
            Unconfirmed::Late(_, name) => late.push(name.as_str()),
            Unconfirmed::Unavailable(_, name) => unavailable.push(name.as_str()),
        }
    }
    let mut parts = Vec::new();
    if !late.is_empty() {
        let verb = if late.len() == 1 {
            "n'a pas encore répondu"
        } else {
            "n'ont pas encore répondu"
        };
        parts.push(format!("{} {verb}", list(&late)));
    }
    if !unavailable.is_empty() {
        let verb = if unavailable.len() == 1 {
            "est indisponible"
        } else {
            "sont indisponibles"
        };
        parts.push(format!("{} {verb}", list(&unavailable)));
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!(
        "J'ai envoyé l'ordre, mais {}.",
        parts.join(", et ")
    ))
}

/// « a », « a et b », « a, b et c ».
fn list(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [first @ .., last] => format!("{} et {last}", first.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn id(text: &str) -> EntityId {
        text.parse().unwrap()
    }

    fn attributes(value: Value) -> Attributes {
        let Value::Object(map) = value else {
            panic!("un objet est attendu")
        };
        map
    }

    /// Une maison avec une lampe Hue éteinte et un volet MQTT ouvert.
    fn home() -> ha_core::Core {
        let core = ha_core::Core::new();
        core.states
            .set_by("hue", id("light.salon"), "off", Attributes::new())
            .unwrap();
        core.states
            .set_by(
                "mqtt",
                id("cover.chambre"),
                "open",
                attributes(json!({"current_position": 100})),
            )
            .unwrap();
        core
    }

    fn watch_on(core: &ha_core::Core) -> Watch {
        watch(
            &core.states,
            "light",
            "turn_on",
            &id("light.salon"),
            &Attributes::new(),
            "Lampe salon",
        )
        .unwrap()
    }

    const SHORT: Duration = Duration::from_millis(300);

    #[tokio::test]
    async fn confirmed_at_once() {
        let core = home();
        let mut events = core.bus.subscribe();
        let watch = watch_on(&core);
        // L'appareil répond avant même qu'on attende.
        core.states
            .set(id("light.salon"), "on", Attributes::new())
            .unwrap();
        let started = std::time::Instant::now();
        assert_eq!(
            wait(&mut events, &core.states, vec![watch], SHORT).await,
            []
        );
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[tokio::test]
    async fn confirmed_after_a_moment() {
        let core = Arc::new(home());
        let mut events = core.bus.subscribe();
        let watch = watch_on(&core);
        let later = Arc::clone(&core);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            // Un autre appareil d'abord, puis un attribut seul : on attend encore.
            later
                .states
                .set(id("cover.chambre"), "closed", Attributes::new())
                .unwrap();
            later
                .states
                .set(
                    id("light.salon"),
                    "off",
                    attributes(json!({"friendly_name": "Lampe"})),
                )
                .unwrap();
            later
                .states
                .set(id("light.salon"), "on", Attributes::new())
                .unwrap();
        });
        let started = std::time::Instant::now();
        assert_eq!(
            wait(
                &mut events,
                &core.states,
                vec![watch],
                Duration::from_secs(2)
            )
            .await,
            []
        );
        // Réponse dès la confirmation, sans attendre la fin.
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn never_confirmed() {
        let core = home();
        let mut events = core.bus.subscribe();
        let watch = watch_on(&core);
        let started = std::time::Instant::now();
        let late = wait(&mut events, &core.states, vec![watch], SHORT).await;
        assert!(started.elapsed() >= SHORT);
        assert_eq!(
            late,
            [Unconfirmed::Late(id("light.salon"), "Lampe salon".into())]
        );
        assert_eq!(
            sentence(&late).as_deref(),
            Some("J'ai envoyé l'ordre, mais Lampe salon n'a pas encore répondu.")
        );
    }

    #[tokio::test]
    async fn already_in_the_right_state() {
        let core = home();
        let mut events = core.bus.subscribe();
        // Le volet est déjà tout ouvert : on n'attend rien.
        let watch = watch(
            &core.states,
            "cover",
            "open_cover",
            &id("cover.chambre"),
            &Attributes::new(),
            "Volet chambre",
        )
        .unwrap();
        let started = std::time::Instant::now();
        assert_eq!(
            wait(&mut events, &core.states, vec![watch], SHORT).await,
            []
        );
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[tokio::test]
    async fn an_unavailable_device_is_reported_at_once() {
        let core = home();
        core.states
            .set(id("light.salon"), "unavailable", Attributes::new())
            .unwrap();
        let mut events = core.bus.subscribe();
        let watch = watch_on(&core);
        let started = std::time::Instant::now();
        let result = wait(&mut events, &core.states, vec![watch], SHORT).await;
        assert!(started.elapsed() < Duration::from_millis(100));
        assert_eq!(
            sentence(&result).as_deref(),
            Some("J'ai envoyé l'ordre, mais Lampe salon est indisponible.")
        );
    }

    #[test]
    fn slow_or_assumed_states_are_not_awaited() {
        let core = ha_core::Core::new();
        // Somfy, SmartThings : la passerelle a déjà dit oui, l'état viendra
        // bien plus tard.
        for owner in ["somfy", "somfy_chauffage", "smartthings"] {
            let entity = id(&format!("cover.{owner}"));
            core.states
                .set_by(owner, entity.clone(), "closed", Attributes::new())
                .unwrap();
            let watched = watch(
                &core.states,
                "cover",
                "open_cover",
                &entity,
                &Attributes::new(),
                "Volet",
            )
            .unwrap();
            assert_eq!(watched.check, None, "{owner}");
        }
        // MQTT sans sujet d'état : l'état est supposé par notre code.
        core.states
            .set_by(
                "mqtt",
                id("switch.prise"),
                "off",
                attributes(json!({"assumed_state": true})),
            )
            .unwrap();
        let watched = watch(
            &core.states,
            "switch",
            "turn_on",
            &id("switch.prise"),
            &Attributes::new(),
            "Prise",
        )
        .unwrap();
        assert_eq!(watched.check, None);
        // Un appareil du serveur lui-même (sans intégration) : écrit tout de suite.
        core.states
            .set(id("input_boolean.vacances"), "off", Attributes::new())
            .unwrap();
        let watched = watch(
            &core.states,
            "input_boolean",
            "turn_on",
            &id("input_boolean.vacances"),
            &Attributes::new(),
            "Vacances",
        )
        .unwrap();
        assert_eq!(watched.check, None);
    }

    #[test]
    fn what_is_expected() {
        let state = |value: &str, attrs: Value| {
            let core = ha_core::Core::new();
            core.states
                .set(id("cover.x"), value, attributes(attrs))
                .unwrap();
            core.states.get(&id("cover.x")).unwrap()
        };
        let check = |domain: &str, service: &str, data: Value, current: &State| {
            expectation(domain, service, &attributes(data), current)
        };
        let off = state("off", json!({}));
        let on = check("light", "toggle", json!({}), &off).unwrap();
        assert!(on.reached(&state("on", json!({}))));
        assert!(!on.reached(&off));

        // Luminosité : arrondie par l'appareil.
        let dimmed = check("light", "turn_on", json!({"brightness_pct": 40}), &off).unwrap();
        assert!(dimmed.reached(&state("on", json!({"brightness": 101}))));
        assert!(!dimmed.reached(&state("on", json!({"brightness": 255}))));

        // Volet : en mouvement, c'est que l'appareil a obéi.
        let half = state("open", json!({"current_position": 50}));
        let open = check("cover", "open_cover", json!({}), &half).unwrap();
        assert!(!open.reached(&half), "à moitié ouvert n'est pas ouvert");
        assert!(open.reached(&state("opening", json!({"current_position": 55}))));
        assert!(open.reached(&state("open", json!({"current_position": 100}))));
        // Certains volets disent « ouvert » pendant la course : la position a monté.
        assert!(open.reached(&state("open", json!({"current_position": 60}))));
        assert!(!open.reached(&state("open", json!({"current_position": 40}))));
        let close = check("cover", "close_cover", json!({}), &half).unwrap();
        assert!(close.reached(&state("closing", json!({}))));
        assert!(close.reached(&state("open", json!({"current_position": 45}))));
        assert!(close.reached(&state("closed", json!({"current_position": 0}))));
        // Sans position : ouvert ou fermé seulement.
        let shut = state("closed", json!({}));
        let open_plain = check("cover", "open_cover", json!({}), &shut).unwrap();
        assert!(open_plain.reached(&state("open", json!({}))));
        assert!(!open_plain.reached(&shut));
        assert_eq!(
            check(
                "cover",
                "set_cover_position",
                json!({"position": 30}),
                &shut
            ),
            None
        );
        let to_30 = check(
            "cover",
            "set_cover_position",
            json!({"position": 30}),
            &half,
        )
        .unwrap();
        assert!(to_30.reached(&state("open", json!({"current_position": 31}))));
        assert!(to_30.reached(&state("closing", json!({"current_position": 45}))));
        assert!(!to_30.reached(&half));

        // Chauffage, musique, serrure.
        let heat = check(
            "climate",
            "set_hvac_mode",
            json!({"hvac_mode": "heat"}),
            &off,
        )
        .unwrap();
        assert!(heat.reached(&state("heat", json!({}))));
        let warm = check(
            "climate",
            "set_temperature",
            json!({"temperature": 21.5}),
            &off,
        )
        .unwrap();
        assert!(warm.reached(&state("heat", json!({"temperature": 21.5}))));
        assert!(!warm.reached(&state("heat", json!({"temperature": 19}))));
        let louder = check(
            "media_player",
            "volume_set",
            json!({"volume_level": 0.3}),
            &off,
        )
        .unwrap();
        assert!(louder.reached(&state("playing", json!({"volume_level": 0.31}))));
        let locked = check("lock", "lock", json!({}), &off).unwrap();
        assert!(locked.reached(&state("locking", json!({}))));

        // Pas d'état attendu clair : on n'attend pas.
        assert_eq!(check("cover", "stop_cover", json!({}), &half), None);
        assert_eq!(
            check("light", "toggle", json!({}), &state("unknown", json!({}))),
            None
        );
    }

    #[test]
    fn sentences() {
        let late = |name: &str| Unconfirmed::Late(id("light.a"), name.into());
        let away = |name: &str| Unconfirmed::Unavailable(id("light.a"), name.into());
        assert_eq!(sentence(&[]), None);
        assert_eq!(
            sentence(&[late("Volet chambre"), late("Volet salon")]).as_deref(),
            Some(
                "J'ai envoyé l'ordre, mais Volet chambre et Volet salon n'ont pas encore répondu."
            )
        );
        assert_eq!(
            sentence(&[late("A"), away("B"), away("C")]).as_deref(),
            Some(
                "J'ai envoyé l'ordre, mais A n'a pas encore répondu, et B et C sont indisponibles."
            )
        );
    }
}
