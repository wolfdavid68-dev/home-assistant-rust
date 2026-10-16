//! Le pont entre MQTT et le noyau : il transforme les messages reçus en états
//! d'entités, et les appels de service en messages à publier.
//!
//! Concept : cette partie ne touche pas au réseau. Elle reçoit des messages
//! et renvoie des [`Action`]s (s'abonner, publier…) que la boucle réseau
//! ([`crate::client`]) exécute. On peut donc la tester entièrement sans
//! broker MQTT.

use std::collections::HashMap;

use ha_core::{Attributes, EntityId, StateMachine};
use serde_json::{Value, json};
use tracing::{debug, info, warn};

use crate::discovery::{self, Component, Discovery, EntityConfig};
use crate::profile::{FreeState, Optimistic, attribute_value};

/// Nombre maximal d'entités découvertes : un appareil défaillant (ou
/// malveillant) ne peut pas en créer à l'infini.
pub const MAX_ENTITIES: usize = 1000;
/// Taille maximale d'un message d'état.
const MAX_STATE_PAYLOAD: usize = 64 * 1024;

/// Ce que la boucle réseau doit faire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Subscribe(String),
    Unsubscribe(String),
    Publish {
        topic: String,
        payload: String,
    },
    /// Publier un événement sur le bus du noyau (une télécommande a été
    /// appuyée : `event_received`).
    Fire {
        event_type: String,
        data: Attributes,
    },
}

#[derive(Debug)]
struct Tracked {
    config: EntityConfig,
    entity_id: EntityId,
}

/// Les entités découvertes et les sujets associés.
#[derive(Debug)]
pub struct Bridge {
    prefix: String,
    /// Clé : le sujet de découverte (`homeassistant/sensor/…/config`).
    entities: HashMap<String, Tracked>,
    /// Identifiant d'entité → sujet de découverte, pour les commandes.
    by_entity: HashMap<EntityId, String>,
    /// Sujets d'état écoutés, avec le nombre d'entités qui les utilisent.
    subscriptions: HashMap<String, usize>,
}

impl Bridge {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            entities: HashMap::new(),
            by_entity: HashMap::new(),
            subscriptions: HashMap::new(),
        }
    }

    /// Les abonnements nécessaires pour recevoir les descriptions.
    #[must_use]
    pub fn discovery_filters(&self) -> [String; 2] {
        [
            format!("{}/+/+/config", self.prefix),
            format!("{}/+/+/+/config", self.prefix),
        ]
    }

    /// Tous les sujets d'état à écouter (après une reconnexion).
    pub fn state_topics(&self) -> impl Iterator<Item = &String> {
        self.subscriptions.keys()
    }

    /// Nombre d'entités suivies.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Traite un message reçu.
    pub fn handle_message(
        &mut self,
        topic: &str,
        payload: &[u8],
        states: &StateMachine,
    ) -> Vec<Action> {
        if let Some((component, object_id)) = discovery::parse_topic(&self.prefix, topic) {
            return match discovery::parse(&self.prefix, component, object_id, payload) {
                Ok(Discovery::Config(config)) => self.add(topic, *config, object_id, states),
                Ok(Discovery::Remove) => self.remove(topic, states),
                Err(error) => {
                    warn!("découverte MQTT ignorée ({topic}) : {error}");
                    Vec::new()
                }
            };
        }
        self.update_states(topic, payload, states)
    }

    /// Traite un appel de service. Renvoie le message à publier, s'il y en a un.
    pub fn handle_service(
        &self,
        domain: &str,
        service: &str,
        entity_id: &EntityId,
        data: &Attributes,
        states: &StateMachine,
    ) -> Vec<Action> {
        let Some(tracked) = self
            .by_entity
            .get(entity_id)
            .and_then(|topic| self.entities.get(topic))
        else {
            return Vec::new(); // Pas une entité MQTT : une autre intégration s'en occupe.
        };
        let config = &tracked.config;
        if domain != config.component.domain() {
            warn!("service {domain}.{service} incompatible avec {entity_id}");
            return Vec::new();
        }
        // Une vanne s'ouvre si elle est fermée, et inversement.
        let service = match (config.component, service) {
            (Component::Valve, "toggle") => {
                let open = states
                    .get(entity_id)
                    .is_some_and(|s| matches!(s.state(), "open" | "opening"));
                if open { "close_valve" } else { "open_valve" }
            }
            _ => service,
        };
        // Les commandes propres au type : verrou, vanne, aspirateur, consignes.
        if let Some(command) = config.profile.command(service) {
            let payload = match command.render(data) {
                Ok(payload) => payload,
                Err(error) => {
                    warn!("{domain}.{service} sur {entity_id} refusé : {error}");
                    return Vec::new();
                }
            };
            // Sans sujet d'état, on suppose que l'appareil a obéi. Un bouton,
            // lui, n'a jamais d'état : c'est la date du dernier appui.
            let supposed = match command.optimistic {
                Optimistic::Keep => None,
                Optimistic::State(state) => Some(state.to_owned()),
                Optimistic::Sent => Some(payload.clone()),
                Optimistic::Now => Some(jiff::Timestamp::now().to_string()),
            };
            if let Some(state) = supposed
                && (config.state_topic.is_none() || command.optimistic == Optimistic::Now)
            {
                let attributes = current_attributes(config, entity_id, states);
                let _ = states.set_by(crate::ID, entity_id.clone(), state, attributes);
            }
            debug!("{entity_id} : envoi de {payload:?} sur {}", command.topic);
            return vec![Action::Publish {
                topic: command.topic.clone(),
                payload,
            }];
        }
        if !config.component.is_on_off() {
            warn!("service {domain}.{service} non pris en charge par {entity_id}");
            return Vec::new();
        }
        let Some(command_topic) = &config.command_topic else {
            warn!("{entity_id} ne peut pas être commandé (pas de sujet de commande)");
            return Vec::new();
        };
        let turn_on = match service {
            "turn_on" => true,
            "turn_off" => false,
            "toggle" => states
                .get(entity_id)
                .is_none_or(|state| state.state() != "on"),
            _ => {
                warn!("service {domain}.{service} non pris en charge");
                return Vec::new();
            }
        };

        let payload = if config.json_schema {
            let mut command = json!({
                "state": if turn_on { &config.payload_on } else { &config.payload_off },
            });
            if let (true, Some(brightness)) = (turn_on, data.get("brightness")) {
                let Some(brightness) = brightness.as_u64().filter(|b| *b <= 255) else {
                    warn!("luminosité invalide pour {entity_id} : {brightness} (0 à 255)");
                    return Vec::new();
                };
                command["brightness"] =
                    json!(brightness * u64::from(config.brightness_scale) / 255);
            }
            command.to_string()
        } else if turn_on {
            config.payload_on.clone()
        } else {
            config.payload_off.clone()
        };

        // Sans sujet d'état, on ne saura jamais si l'appareil a obéi : on
        // suppose que oui (mode « optimiste », comme Home Assistant).
        if config.state_topic.is_none() {
            let value = if turn_on { "on" } else { "off" };
            let attributes = current_attributes(config, entity_id, states);
            let _ = states.set_by(crate::ID, entity_id.clone(), value, attributes);
        }
        debug!("{entity_id} : envoi de {payload:?} sur {command_topic}");
        vec![Action::Publish {
            topic: command_topic.clone(),
            payload,
        }]
    }

    fn add(
        &mut self,
        discovery_topic: &str,
        config: EntityConfig,
        topic_object_id: &str,
        states: &StateMachine,
    ) -> Vec<Action> {
        let mut actions = Vec::new();

        // Mise à jour d'une entité déjà connue.
        if let Some(existing) = self.entities.get(discovery_topic) {
            if existing.config == config {
                return actions;
            }
            if existing.config.component == config.component {
                let entity_id = existing.entity_id.clone();
                let old_topics = listened_topics(&existing.config);
                // D'abord les nouveaux, puis les anciens : un sujet gardé n'est
                // jamais désabonné entre les deux.
                for topic in listened_topics(&config) {
                    actions.extend(self.acquire(&topic));
                }
                for topic in old_topics {
                    actions.extend(self.release(&topic));
                }
                self.entities
                    .insert(discovery_topic.to_owned(), Tracked { config, entity_id });
                return actions;
            }
            // Le type a changé : on repart de zéro.
            actions.extend(self.remove(discovery_topic, states));
        }

        if self.entities.len() >= MAX_ENTITIES {
            warn!("trop d'entités MQTT ({MAX_ENTITIES}) : {discovery_topic} ignoré");
            return actions;
        }
        let Some(entity_id) = self.allocate_id(&config, topic_object_id) else {
            warn!("aucun identifiant libre pour {discovery_topic}");
            return actions;
        };

        // Un état restauré depuis la base (même entité avant redémarrage) est
        // conservé ; sinon l'entité démarre en « unknown » comme dans Home Assistant.
        if states.get(&entity_id).is_none() {
            let _ = states.set_by(
                crate::ID,
                entity_id.clone(),
                "unknown",
                base_attributes(&config),
            );
        }
        info!("entité MQTT découverte : {entity_id}");
        for topic in listened_topics(&config) {
            actions.extend(self.acquire(&topic));
        }
        self.by_entity
            .insert(entity_id.clone(), discovery_topic.to_owned());
        self.entities
            .insert(discovery_topic.to_owned(), Tracked { config, entity_id });
        actions
    }

    fn remove(&mut self, discovery_topic: &str, states: &StateMachine) -> Vec<Action> {
        let Some(tracked) = self.entities.remove(discovery_topic) else {
            return Vec::new();
        };
        self.by_entity.remove(&tracked.entity_id);
        states.remove(&tracked.entity_id);
        info!("entité MQTT retirée : {}", tracked.entity_id);
        listened_topics(&tracked.config)
            .into_iter()
            .filter_map(|topic| self.release(&topic))
            .collect()
    }

    /// Choisit un identifiant libre : `sensor.temperature_salon`, sinon `_2`, `_3`…
    fn allocate_id(&self, config: &EntityConfig, topic_object_id: &str) -> Option<EntityId> {
        let domain = config.component.domain();
        let slug = [config.object_id.as_str(), topic_object_id]
            .into_iter()
            .map(ha_core::slugify)
            .find(|slug| !slug.is_empty())
            .unwrap_or_else(|| "mqtt".to_owned());
        (1..=MAX_ENTITIES).find_map(|n| {
            let candidate = if n == 1 {
                format!("{domain}.{slug}")
            } else {
                format!("{domain}.{slug}_{n}")
            };
            EntityId::new(&candidate)
                .ok()
                .filter(|id| !self.by_entity.contains_key(id))
        })
    }

    fn acquire(&mut self, topic: &str) -> Option<Action> {
        let count = self.subscriptions.entry(topic.to_owned()).or_insert(0);
        *count += 1;
        (*count == 1).then(|| Action::Subscribe(topic.to_owned()))
    }

    fn release(&mut self, topic: &str) -> Option<Action> {
        let count = self.subscriptions.get_mut(topic)?;
        *count -= 1;
        if *count > 0 {
            return None;
        }
        self.subscriptions.remove(topic);
        Some(Action::Unsubscribe(topic.to_owned()))
    }

    /// Un message d'état : met à jour toutes les entités qui écoutent ce sujet
    /// (Zigbee2MQTT envoie température, humidité et batterie dans le même message).
    fn update_states(&self, topic: &str, payload: &[u8], states: &StateMachine) -> Vec<Action> {
        if payload.len() > MAX_STATE_PAYLOAD {
            warn!(
                "message trop gros sur {topic} ({} octets), ignoré",
                payload.len()
            );
            return Vec::new();
        }
        let Ok(payload) = std::str::from_utf8(payload) else {
            warn!("message non UTF-8 sur {topic}, ignoré");
            return Vec::new();
        };
        let mut actions = Vec::new();
        for tracked in self.entities.values() {
            let config = &tracked.config;
            let id = &tracked.entity_id;
            // Une télécommande : chaque appui est un événement.
            if config.component == Component::Event {
                if config.state_topic.as_deref() == Some(topic)
                    && let Some(action) = event_received(config, id, payload, states)
                {
                    actions.push(action);
                }
                continue;
            }
            // L'état principal…
            if config.state_topic.as_deref() == Some(topic)
                && let Some((value, mut attributes)) = interpret(config, payload)
            {
                keep_profile_attributes(config, id, states, &mut attributes);
                if let Err(error) = states.set_by(crate::ID, id.clone(), value, attributes) {
                    warn!("état refusé pour {id} : {error}");
                }
            }
            // … puis les attributs (consigne, batterie…) arrivés sur ce sujet.
            for attribute in config
                .profile
                .attributes
                .iter()
                .filter(|a| a.topic == topic)
            {
                let Some(value) = attribute_value(attribute, payload) else {
                    continue;
                };
                let current = states.get(id);
                let mut state = current.as_ref().map_or("unknown", |s| s.state()).to_owned();
                // Un volet sans sujet d'état : fermé à 0 %, ouvert sinon.
                if config.component == Component::Cover
                    && config.state_topic.is_none()
                    && attribute.name == "current_position"
                    && let Some(position) = value.as_f64()
                {
                    if position <= 0.0 { "closed" } else { "open" }.clone_into(&mut state);
                }
                let mut attributes = current_attributes(config, id, states);
                attributes.insert(attribute.name.into(), value);
                if let Err(error) = states.set_by(crate::ID, id.clone(), state, attributes) {
                    warn!("attribut refusé pour {id} : {error}");
                }
            }
        }
        actions
    }
}

/// Transforme un message d'état en (valeur, attributs). `None` si le message
/// ne concerne pas cette entité ou n'est pas compris.
fn interpret(config: &EntityConfig, payload: &str) -> Option<(String, Attributes)> {
    let mut attributes = base_attributes(config);

    if config.json_schema {
        let message: Value = serde_json::from_str(payload).ok()?;
        let state = message.get("state")?.as_str()?;
        let on = on_off(config, state)?;
        if let (true, Some(brightness)) = (on, message.get("brightness").and_then(Value::as_u64)) {
            let scale = u64::from(config.brightness_scale);
            attributes.insert(
                "brightness".into(),
                json!(brightness.min(scale) * 255 / scale),
            );
        }
        return Some((if on { "on" } else { "off" }.to_owned(), attributes));
    }

    let value = config.value_template.render(payload)?;
    if config.component == Component::Sensor {
        return Some((value, attributes));
    }
    if config.component.is_on_off() {
        let Some(on) = on_off(config, &value) else {
            debug!("valeur inattendue {value:?} pour {}", config.object_id);
            return None;
        };
        return Some((if on { "on" } else { "off" }.to_owned(), attributes));
    }
    // Verrou, vanne, aspirateur, chauffe-eau… : seulement les états annoncés.
    if let Some(state) = config.profile.state_for(&value) {
        return Some((state.to_owned(), attributes));
    }
    let free = match config.profile.free {
        FreeState::Listed => None,
        // Un nombre : écrit simplement (« 21.5 »), fini.
        FreeState::Number => value
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map(crate::profile::format_number),
        // Un texte : pas trop long, sans caractère de contrôle.
        FreeState::Text { max } => (value.chars().count() <= max
            && !value.chars().any(char::is_control))
        .then_some(value.clone()),
    };
    if free.is_none() {
        debug!("valeur inattendue {value:?} pour {}", config.object_id);
    }
    free.map(|state| (state, attributes))
}

/// Un message d'une entité `event` (`{"event_type": "double"}`) : l'état
/// devient l'heure de l'appui (comme dans Home Assistant), l'attribut
/// `event_type` dit lequel, et l'événement `event_received` est publié sur
/// le bus pour les automatisations. Une sorte non annoncée est ignorée.
fn event_received(
    config: &EntityConfig,
    id: &EntityId,
    payload: &str,
    states: &StateMachine,
) -> Option<Action> {
    let rendered = config.value_template.render(payload)?;
    let message: Value = serde_json::from_str(&rendered).ok()?;
    let event_type = message.get("event_type")?.as_str()?;
    if !config
        .profile
        .event_types
        .iter()
        .any(|known| known == event_type)
    {
        debug!("sorte d'événement inattendue {event_type:?} pour {id}");
        return None;
    }
    let mut attributes = base_attributes(config);
    attributes.insert("event_type".into(), json!(event_type));
    if let Err(error) = states.set_by(
        crate::ID,
        id.clone(),
        jiff::Timestamp::now().to_string(),
        attributes,
    ) {
        warn!("état refusé pour {id} : {error}");
    }
    let mut data = Attributes::new();
    data.insert("entity_id".into(), json!(id.as_str()));
    data.insert("event_type".into(), json!(event_type));
    Some(Action::Fire {
        event_type: "event_received".into(),
        data,
    })
}

/// Les sujets qu'écoute une entité (état et attributs), sans doublon.
fn listened_topics(config: &EntityConfig) -> Vec<String> {
    let mut topics: Vec<String> = config.state_topic.iter().cloned().collect();
    for attribute in &config.profile.attributes {
        if !topics.contains(&attribute.topic) {
            topics.push(attribute.topic.clone());
        }
    }
    topics
}

/// Les attributs de base, plus ceux venus des sujets d'attributs (déjà reçus).
fn current_attributes(config: &EntityConfig, id: &EntityId, states: &StateMachine) -> Attributes {
    let mut attributes = base_attributes(config);
    keep_profile_attributes(config, id, states, &mut attributes);
    attributes
}

/// Un nouvel état ne doit pas effacer la consigne ou la batterie reçues avant.
fn keep_profile_attributes(
    config: &EntityConfig,
    id: &EntityId,
    states: &StateMachine,
    attributes: &mut Attributes,
) {
    let Some(current) = states.get(id) else {
        return;
    };
    for attribute in &config.profile.attributes {
        if attributes.contains_key(attribute.name) {
            continue;
        }
        if let Some(value) = current.attributes().get(attribute.name) {
            attributes.insert(attribute.name.into(), value.clone());
        }
    }
}

fn on_off(config: &EntityConfig, value: &str) -> Option<bool> {
    if value == config.state_on {
        Some(true)
    } else if value == config.state_off {
        Some(false)
    } else {
        None
    }
}

fn base_attributes(config: &EntityConfig) -> Attributes {
    let mut attributes = Attributes::new();
    let fields = [
        ("friendly_name", &config.friendly_name),
        ("unit_of_measurement", &config.unit_of_measurement),
        ("device_class", &config.device_class),
    ];
    for (key, value) in fields {
        if let Some(value) = value {
            attributes.insert(key.into(), json!(value));
        }
    }
    config.profile.add_fixed(&mut attributes);
    attributes
}
