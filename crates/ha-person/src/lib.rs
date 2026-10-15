//! **Personnes** et **zone Maison**, comme dans Home Assistant.
//!
//! - Une personne (`person.david`) regroupe plusieurs appareils : le
//!   téléphone et l'ordinateur vus par la Freebox, par exemple. Elle est « à
//!   la maison » (`home`) dès que l'un d'eux l'est.
//! - La zone Maison (`zone.home`) a pour état le **nombre** de personnes
//!   présentes : « quand le dernier part » s'écrit `to: "0"`.
//!
//! Le fichier `persons.yaml` :
//!
//! ```yaml
//! - id: david
//!   name: David
//!   trackers: [device_tracker.telephone_david, device_tracker.pc_david]
//! ```

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use ha_core::{Attributes, Background, CancellationToken, Core, EntityId, Event, State};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};

const MAX_FILE_SIZE: u64 = 256 * 1024;
const MAX_PERSONS: usize = 20;
const MAX_TRACKERS: usize = 10;
const MAX_NAME_CHARS: usize = 60;
/// La zone Maison.
pub const HOME_ZONE: &str = "zone.home";

#[derive(Debug, thiserror::Error)]
pub enum PersonError {
    #[error("lecture du fichier impossible : {0}")]
    Io(#[from] std::io::Error),

    #[error("fichier trop gros (256 Kio au plus)")]
    TooLarge,

    #[error("YAML invalide : {0}")]
    Yaml(String),

    #[error("{0}")]
    Invalid(String),
}

/// Une personne, vérifiée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub id: String,
    pub name: String,
    /// `device_tracker.*` (`home` / `not_home`) ou `binary_sensor.*`
    /// (`on` = présent : un capteur de lit, par exemple).
    pub trackers: Vec<EntityId>,
}

impl Person {
    /// L'entité `person.<id>`.
    #[must_use]
    pub fn entity_id(&self) -> Option<EntityId> {
        format!("person.{}", self.id).parse().ok()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersonConfig {
    id: String,
    #[serde(default)]
    name: Option<String>,
    trackers: Vec<String>,
}

/// Lit le fichier. Un fichier absent donne une liste vide.
pub fn load(path: &Path) -> Result<Vec<Person>, PersonError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    if metadata.len() > MAX_FILE_SIZE {
        return Err(PersonError::TooLarge);
    }
    parse(&std::fs::read_to_string(path)?)
}

/// Lit et vérifie le contenu du fichier.
pub fn parse(yaml: &str) -> Result<Vec<Person>, PersonError> {
    if yaml.trim().is_empty() {
        return Ok(Vec::new());
    }
    let configs: Vec<PersonConfig> =
        serde_saphyr::from_str(yaml).map_err(|error| PersonError::Yaml(error.to_string()))?;
    if configs.len() > MAX_PERSONS {
        return Err(PersonError::Invalid(format!(
            "{MAX_PERSONS} personnes au plus"
        )));
    }
    let mut ids = HashSet::new();
    let mut persons = Vec::with_capacity(configs.len());
    for config in configs {
        let id = config.id;
        let invalid =
            |reason: String| PersonError::Invalid(format!("personne « {id} » : {reason}"));
        if !ha_core::is_valid_name(&id) {
            return Err(invalid(
                "id : 1 à 64 caractères parmi a-z, 0-9 et « _ »".into(),
            ));
        }
        if !ids.insert(id.clone()) {
            return Err(invalid("identifiant en double".into()));
        }
        let name = config.name.unwrap_or_else(|| id.replace('_', " "));
        if name.trim().is_empty()
            || name.chars().count() > MAX_NAME_CHARS
            || name.chars().any(char::is_control)
        {
            return Err(invalid(format!(
                "nom vide, trop long ({MAX_NAME_CHARS} caractères au plus) ou invalide"
            )));
        }
        if config.trackers.is_empty() || config.trackers.len() > MAX_TRACKERS {
            return Err(invalid(format!("trackers : 1 à {MAX_TRACKERS} appareils")));
        }
        let mut trackers = Vec::with_capacity(config.trackers.len());
        for tracker in &config.trackers {
            let entity: EntityId = tracker
                .parse()
                .map_err(|_| invalid(format!("identifiant invalide {tracker:?}")))?;
            if !matches!(entity.domain(), "device_tracker" | "binary_sensor") {
                return Err(invalid(format!(
                    "{entity} : un device_tracker.… ou un binary_sensor.… est attendu"
                )));
            }
            if trackers.contains(&entity) {
                return Err(invalid(format!("{entity} en double")));
            }
            trackers.push(entity);
        }
        persons.push(Person { id, name, trackers });
    }
    Ok(persons)
}

/// Où est un appareil : `Some(true)` à la maison, `Some(false)` absent,
/// `None` inconnu (indisponible, jamais vu…).
fn tracker_home(entity_id: &EntityId, state: Option<&State>) -> Option<bool> {
    let value = state?.state();
    match (entity_id.domain(), value) {
        ("device_tracker", "home") | ("binary_sensor", "on") => Some(true),
        ("device_tracker", "not_home") | ("binary_sensor", "off") => Some(false),
        _ => None,
    }
}

/// L'état d'une personne, d'après ses appareils : présente si **l'un** est à
/// la maison ; absente si tous ceux qu'on connaît sont absents ; inconnue
/// sinon. Renvoie aussi l'appareil qui a décidé.
#[must_use]
pub fn person_state(person: &Person, core: &Core) -> (&'static str, Option<EntityId>) {
    let mut away = None;
    for tracker in &person.trackers {
        match tracker_home(tracker, core.states.get(tracker).as_ref()) {
            Some(true) => return ("home", Some(tracker.clone())),
            Some(false) => away = away.or_else(|| Some(tracker.clone())),
            None => {}
        }
    }
    match away {
        Some(tracker) => ("not_home", Some(tracker)),
        None => ("unknown", None),
    }
}

/// La tâche qui tient les personnes et la zone Maison à jour.
#[derive(Debug)]
pub struct Persons(Background);

impl Persons {
    pub fn start(persons: Vec<Person>, core: Arc<Core>) -> Self {
        let events = core.bus.subscribe();
        // Tout de suite, pour que les entités existent dès le démarrage.
        publish_all(&persons, &core);
        Self(Background::spawn("les personnes", move |token| {
            run(persons, core, events, token)
        }))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

async fn run(
    persons: Vec<Person>,
    core: Arc<Core>,
    mut events: tokio::sync::broadcast::Receiver<Arc<Event>>,
    token: CancellationToken,
) {
    info!("{} personne(s)", persons.len());
    loop {
        tokio::select! {
            () = token.cancelled() => break,
            received = events.recv() => match received {
                Ok(event) => {
                    if let Event::StateChanged { entity_id, .. } = event.as_ref()
                        && persons.iter().any(|p| p.trackers.contains(entity_id))
                    {
                        publish_all(&persons, &core);
                    }
                }
                Err(RecvError::Lagged(_)) => publish_all(&persons, &core),
                Err(RecvError::Closed) => break,
            },
        }
    }
}

/// Met à jour chaque `person.*` et `zone.home`.
fn publish_all(persons: &[Person], core: &Core) {
    let mut at_home = Vec::new();
    for person in persons {
        let (state, source) = person_state(person, core);
        if state == "home" {
            at_home.push(person.name.clone());
        }
        let Some(entity_id) = person.entity_id() else {
            continue;
        };
        let trackers: Vec<&str> = person.trackers.iter().map(EntityId::as_str).collect();
        let mut attributes = object(json!({
            "friendly_name": person.name,
            "device_trackers": trackers,
            "source": source.as_ref().map(EntityId::as_str),
        }));
        attributes.retain(|_, value| !value.is_null());
        set(core, entity_id, state, attributes);
    }
    if let Ok(zone) = HOME_ZONE.parse::<EntityId>() {
        let attributes = object(json!({
            "friendly_name": "Maison",
            "persons": at_home,
        }));
        set(core, zone, &at_home.len().to_string(), attributes);
    }
}

fn set(core: &Core, entity_id: EntityId, state: &str, attributes: Attributes) {
    if let Err(error) = core.states.set(entity_id, state, attributes) {
        warn!("personnes : état refusé ({error})");
    }
}

fn object(value: Value) -> Attributes {
    match value {
        Value::Object(map) => map,
        _ => Attributes::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persons_are_read_and_checked() {
        let persons = parse(
            "- id: david\n  name: David\n  trackers: [device_tracker.telephone, device_tracker.pc]\n\
             - id: alice\n  trackers: [binary_sensor.lit_alice]\n",
        )
        .unwrap();
        assert_eq!(persons[0].entity_id().unwrap().as_str(), "person.david");
        assert_eq!(persons[1].name, "alice");
        for bad in [
            "- id: David\n  trackers: [device_tracker.a]",
            "- id: a\n  trackers: []",
            "- id: a\n  trackers: [light.salon]",
            "- id: a\n  trackers: [device_tracker.a, device_tracker.a]",
            "- id: a\n  trackers: [device_tracker.a]\n- id: a\n  trackers: [device_tracker.b]",
            "- id: a\n  nom: A\n  trackers: [device_tracker.a]",
            "- id: a\n  name: ''\n  trackers: [device_tracker.a]",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert_eq!(parse("").unwrap(), Vec::new());
    }

    #[test]
    fn a_person_is_home_if_any_device_is() {
        let core = Core::new();
        let person =
            &parse("- id: d\n  trackers: [device_tracker.tel, binary_sensor.lit]").unwrap()[0];
        let set = |id: &str, value: &str| {
            core.states
                .set(id.parse().unwrap(), value, Attributes::new())
                .unwrap();
        };
        assert_eq!(person_state(person, &core).0, "unknown", "jamais vu");
        set("device_tracker.tel", "not_home");
        assert_eq!(person_state(person, &core).0, "not_home");
        set("binary_sensor.lit", "on");
        let (state, source) = person_state(person, &core);
        assert_eq!(
            (state, source.unwrap().as_str()),
            ("home", "binary_sensor.lit")
        );
        set("binary_sensor.lit", "unavailable");
        assert_eq!(
            person_state(person, &core).0,
            "not_home",
            "indisponible : ignoré"
        );
        set("device_tracker.tel", "unavailable");
        assert_eq!(person_state(person, &core).0, "unknown");
    }
}
