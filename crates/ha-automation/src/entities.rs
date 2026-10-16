//! Les **capteurs modèles** (étape 64), comme `template:` dans Home
//! Assistant : un capteur ou un détecteur dont la valeur est un modèle,
//! recalculé dès qu'une entité qu'il lit change.
//!
//! ```yaml
//! # modeles.yaml
//! sensor:
//!   - name: Écart de température
//!     unit_of_measurement: °C
//!     device_class: temperature
//!     state: >-
//!       {{ (states('sensor.salon') | float - states('sensor.dehors') | float) | round(1) }}
//! binary_sensor:
//!   - name: Quelqu'un en bas
//!     device_class: occupancy
//!     state: >-
//!       {{ is_state('binary_sensor.salon', 'on') or is_state('binary_sensor.cuisine', 'on') }}
//! ```
//!
//! Un capteur qui se lit lui-même, ou deux capteurs qui se lisent l'un
//! l'autre, sont refusés au chargement : ils se recalculeraient sans fin.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ha_core::{
    Attributes, Background, CancellationToken, Core, EntityId, Event, MAX_STATE_LEN, slugify,
};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};

use crate::template::{Template, is_true};

/// Le propriétaire des entités (pour le journal et la supervision).
const OWNER: &str = "template";
/// Capteurs modèles au plus.
pub const MAX_ENTITIES: usize = 100;
/// Tout est recalculé de temps en temps, par sécurité.
const REFRESH: Duration = Duration::from_mins(1);

/// Un capteur ou un détecteur modèle, vérifié.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateEntity {
    pub entity_id: EntityId,
    pub name: String,
    pub template: Template,
    /// Le texte du modèle, pour `ha-server modeles check`.
    pub source: String,
    pub unit: Option<String>,
    pub device_class: Option<String>,
    pub icon: Option<String>,
}

impl TemplateEntity {
    fn binary(&self) -> bool {
        self.entity_id.domain() == "binary_sensor"
    }

    /// La valeur à publier : `on` / `off` pour un détecteur ; le texte du
    /// résultat pour un capteur (`unknown` s'il est vide).
    #[must_use]
    pub fn value(&self, states: &ha_core::StateMachine) -> String {
        let result = self.template.evaluate(states, None);
        if self.binary() {
            return if is_true(&result) { "on" } else { "off" }.to_owned();
        }
        let text = match result {
            Value::String(text) => text,
            Value::Null => String::new(),
            Value::Bool(true) => "True".into(),
            Value::Bool(false) => "False".into(),
            other => other.to_string(),
        };
        let text = text.trim();
        if text.is_empty() {
            "unknown".into()
        } else {
            text.chars().take(MAX_STATE_LEN).collect()
        }
    }

    fn attributes(&self) -> Attributes {
        let mut attributes = Attributes::new();
        attributes.insert("friendly_name".into(), self.name.clone().into());
        for (key, value) in [
            ("unit_of_measurement", &self.unit),
            ("device_class", &self.device_class),
            ("icon", &self.icon),
        ] {
            if let Some(value) = value {
                attributes.insert(key.into(), value.clone().into());
            }
        }
        attributes
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    sensor: Vec<EntityConfig>,
    #[serde(default)]
    binary_sensor: Vec<EntityConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntityConfig {
    name: String,
    /// La fin de l'identifiant ; sinon, tirée du nom.
    #[serde(default)]
    id: Option<String>,
    state: String,
    #[serde(default)]
    unit_of_measurement: Option<String>,
    #[serde(default)]
    device_class: Option<String>,
    #[serde(default)]
    icon: Option<String>,
}

/// Un texte court et propre (nom, unité, classe).
fn label(text: &str, what: &str, max: usize) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > max || text.chars().any(char::is_control) {
        return Err(format!(
            "{what} : de 1 à {max} caractères, sans retour à la ligne"
        ));
    }
    Ok(text.to_owned())
}

/// Lit et vérifie le contenu de `modeles.yaml`.
///
/// # Errors
/// Le fichier est mal écrit, un modèle est faux, deux capteurs ont le même
/// identifiant, ou des capteurs se lisent en boucle.
pub fn parse_template_entities(text: &str) -> Result<Vec<TemplateEntity>, String> {
    let file: File = serde_saphyr::from_str(text).map_err(|error| error.to_string())?;
    let configs = file
        .sensor
        .into_iter()
        .map(|config| ("sensor", config))
        .chain(
            file.binary_sensor
                .into_iter()
                .map(|config| ("binary_sensor", config)),
        );
    let mut entities: Vec<TemplateEntity> = Vec::new();
    for (domain, config) in configs {
        let name = label(&config.name, "name", 100)?;
        let object = slugify(config.id.as_deref().unwrap_or(&name));
        let entity_id: EntityId = format!("{domain}.{object}")
            .parse()
            .map_err(|_| format!("« {name} » : identifiant impossible ({domain}.{object})"))?;
        if entities.iter().any(|other| other.entity_id == entity_id) {
            return Err(format!(
                "{entity_id} : deux capteurs portent ce nom (ajoute « id: »)"
            ));
        }
        let template = Template::parse(&config.state)
            .map_err(|error| format!("{entity_id} : {error}"))?
            .ok_or_else(|| format!("{entity_id} : state doit être un modèle {{{{ … }}}}"))?;
        let optional = |value: Option<String>, what: &str| {
            value.map(|text| label(&text, what, 40)).transpose()
        };
        entities.push(TemplateEntity {
            entity_id,
            name,
            template,
            source: config.state.trim().to_owned(),
            unit: optional(config.unit_of_measurement, "unit_of_measurement")?,
            device_class: optional(config.device_class, "device_class")?,
            icon: optional(config.icon, "icon")?,
        });
        if entities.len() > MAX_ENTITIES {
            return Err(format!("{MAX_ENTITIES} capteurs modèles au plus"));
        }
    }
    check_cycles(&entities)?;
    Ok(entities)
}

/// Refuse les capteurs qui se lisent eux-mêmes, directement ou en boucle
/// (A lit B qui lit A) : ils se recalculeraient sans fin.
fn check_cycles(entities: &[TemplateEntity]) -> Result<(), String> {
    let reads: HashMap<&EntityId, Vec<&EntityId>> = entities
        .iter()
        .map(|entity| (&entity.entity_id, entity.template.entity_ids().collect()))
        .collect();
    for start in entities {
        // Parcours depuis chaque capteur : retombe-t-on sur lui ?
        let mut seen: HashSet<&EntityId> = HashSet::new();
        let mut todo: Vec<&EntityId> = reads.get(&start.entity_id).cloned().unwrap_or_default();
        while let Some(next) = todo.pop() {
            if *next == start.entity_id {
                return Err(format!(
                    "{} : se lit lui-même (directement ou par un autre capteur modèle) : il se \
                     recalculerait sans fin",
                    start.entity_id
                ));
            }
            if seen.insert(next) {
                todo.extend(reads.get(next).into_iter().flatten());
            }
        }
    }
    Ok(())
}

/// Lit `modeles.yaml` ; une liste vide si le fichier est absent.
///
/// # Errors
/// Le fichier existe mais ne peut pas être lu, ou il est faux.
pub fn load_template_entities(path: &Path) -> Result<Vec<TemplateEntity>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            parse_template_entities(&text).map_err(|error| format!("{} : {error}", path.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!(
            "lecture de {} impossible : {error}",
            path.display()
        )),
    }
}

/// Les capteurs modèles, recalculés en arrière-plan.
#[derive(Debug)]
pub struct TemplateEntities(Background);

impl TemplateEntities {
    /// Publie chaque capteur, puis le recalcule quand une entité qu'il lit
    /// change. L'abonnement au bus est fait avant de rendre la main.
    #[must_use]
    pub fn start(entities: Vec<TemplateEntity>, core: Arc<Core>) -> Self {
        let events = core.bus.subscribe();
        Self(Background::spawn("les capteurs modèles", move |token| {
            run(entities, core, events, token)
        }))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

fn publish(core: &Core, entity: &TemplateEntity) {
    let value = entity.value(&core.states);
    let attributes = entity.attributes();
    let unchanged = core
        .states
        .get(&entity.entity_id)
        .is_some_and(|state| state.state() == value && state.attributes() == &attributes);
    if unchanged {
        return;
    }
    if let Err(error) = core
        .states
        .set_by(OWNER, entity.entity_id.clone(), value, attributes)
    {
        warn!("{} : état refusé ({error})", entity.entity_id);
    }
}

async fn run(
    entities: Vec<TemplateEntity>,
    core: Arc<Core>,
    mut events: tokio::sync::broadcast::Receiver<Arc<Event>>,
    token: CancellationToken,
) {
    // Pour chaque entité lue : les capteurs à recalculer quand elle change.
    let mut readers: HashMap<EntityId, Vec<usize>> = HashMap::new();
    for (index, entity) in entities.iter().enumerate() {
        for read in entity.template.entity_ids() {
            readers.entry(read.clone()).or_default().push(index);
        }
    }
    info!("capteurs modèles : {}", entities.len());
    for entity in &entities {
        publish(&core, entity);
    }
    let mut refresh = tokio::time::interval(REFRESH);
    refresh.tick().await;
    loop {
        tokio::select! {
            () = token.cancelled() => break,
            _ = refresh.tick() => {
                for entity in &entities {
                    publish(&core, entity);
                }
            }
            received = events.recv() => match received {
                Ok(event) => {
                    let Event::StateChanged { entity_id, .. } = event.as_ref() else {
                        continue;
                    };
                    for index in readers.get(entity_id).into_iter().flatten() {
                        if let Some(entity) = entities.get(*index) {
                            publish(&core, entity);
                        }
                    }
                }
                Err(RecvError::Lagged(missed)) => {
                    warn!("capteurs modèles : {missed} événements manqués, tout est recalculé");
                    for entity in &entities {
                        publish(&core, entity);
                    }
                }
                Err(RecvError::Closed) => break,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    const FILE: &str = r#"
sensor:
  - name: Écart de température
    unit_of_measurement: °C
    device_class: temperature
    state: "{{ (states('sensor.salon') | float - states('sensor.dehors') | float) | round(1) }}"
  - name: Ambiance
    state: "{{ 'chaud' if states('sensor.ecart_de_temperature') | float(0) > 5 else 'doux' }}"
binary_sensor:
  - name: Quelqu'un en bas
    id: en_bas
    device_class: occupancy
    state: "{{ is_state('binary_sensor.salon', 'on') or is_state('binary_sensor.cuisine', 'on') }}"
"#;

    #[test]
    fn the_file_is_checked() {
        let entities = parse_template_entities(FILE).unwrap();
        let ids: Vec<&str> = entities.iter().map(|e| e.entity_id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "sensor.ecart_de_temperature",
                "sensor.ambiance",
                "binary_sensor.en_bas"
            ]
        );
        assert_eq!(entities[0].unit.as_deref(), Some("°C"));
        for (bad, hint) in [
            ("sensor: [{name: A, state: 'pas de modèle'}]", "modèle"),
            ("sensor: [{name: A, state: '{{ 1 +'}]", "sensor.a"),
            (
                "sensor: [{name: A, state: '{{ 1 }}'}, {name: a, state: '{{ 2 }}'}]",
                "deux capteurs",
            ),
            (
                "sensor: [{name: A, state: '{{ 1 }}', couleur: bleu}]",
                "couleur",
            ),
            ("sensor: [{name: '', state: '{{ 1 }}'}]", "name"),
            (
                "sensor: [{name: A, state: \"{{ states('sensor.a') | float + 1 }}\"}]",
                "se lit lui-même",
            ),
            (
                "sensor:\n  - {name: A, state: \"{{ states('sensor.b') }}\"}\n  - {name: B, state: \"{{ states('sensor.a') }}\"}",
                "se lit lui-même",
            ),
        ] {
            let error = parse_template_entities(bad).unwrap_err();
            assert!(error.contains(hint), "{bad} : {error}");
        }
        let example =
            parse_template_entities(include_str!("../../../modeles.example.yaml")).unwrap();
        assert_eq!(example.len(), 3);
    }

    #[tokio::test]
    async fn template_entities_follow_what_they_read() {
        let core = Arc::new(Core::new());
        let set = |entity: &str, value: &str| {
            core.states
                .set(entity.parse().unwrap(), value, Attributes::new())
                .unwrap();
        };
        set("sensor.salon", "21");
        set("sensor.dehors", "18");
        set("binary_sensor.salon", "off");
        let entities = parse_template_entities(FILE).unwrap();
        let service = TemplateEntities::start(entities, Arc::clone(&core));
        let read = |entity: &str| {
            core.states
                .get(&entity.parse().unwrap())
                .map(|s| s.state().to_owned())
        };
        let wait = |entity: &'static str, expected: &'static str| {
            let read = &read;
            async move {
                for _ in 0..100 {
                    if read(entity).as_deref() == Some(expected) {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                panic!("{entity} : {:?} au lieu de {expected}", read(entity));
            }
        };
        wait("sensor.ecart_de_temperature", "3").await;
        wait("sensor.ambiance", "doux").await;
        wait("binary_sensor.en_bas", "off").await;
        let state = core
            .states
            .get(&"sensor.ecart_de_temperature".parse().unwrap())
            .unwrap();
        assert_eq!(state.attributes()["unit_of_measurement"], "°C");
        assert_eq!(state.attributes()["friendly_name"], "Écart de température");

        // Il fait plus froid dehors : l'écart, puis l'ambiance (qui lit
        // l'écart) suivent.
        set("sensor.dehors", "12.5");
        wait("sensor.ecart_de_temperature", "8.5").await;
        wait("sensor.ambiance", "chaud").await;
        set("binary_sensor.salon", "on");
        wait("binary_sensor.en_bas", "on").await;
        // Une mesure illisible : « unknown », pas une erreur.
        set("sensor.salon", "unavailable");
        wait("sensor.ecart_de_temperature", "unknown").await;
        service.stop().await;
    }
}
