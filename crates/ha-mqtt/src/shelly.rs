//! Les prises et relais **Shelly récents** (Gen2, Gen3, Gen4 : Plus, Pro,
//! Plug S…) par MQTT, sans réglage (étape 66).
//!
//! Ces appareils ne suivent pas la découverte de Home Assistant : sans
//! script, rien n'apparaissait. Ils publient pourtant leur état sous un
//! préfixe (`shellyplugsg3-a1b2c3d4e5f6` par défaut), quand « Generic
//! status update over MQTT » est coché dans leur page de réglages :
//!
//! - `<préfixe>/status/switch:0` : `{"output": true, "apower": 812.4,
//!   "voltage": 231.2, "aenergy": {"total": 12345.6}, …}` ;
//! - `<préfixe>/online` : `true` ou `false` (connecté au broker ou non).
//!
//! Chaque sortie devient une prise (`switch.<préfixe>`), avec sa puissance
//! (W), son énergie (kWh, prête pour le bilan énergétique) et sa tension (V)
//! si l'appareil les mesure. Les ordres partent sur
//! `<préfixe>/command/switch:0` (`on`, `off`, `toggle`). Le nom donné dans
//! l'appli Shelly (« Lave-linge ») est demandé une fois
//! (`Shelly.GetDeviceInfo`), la réponse arrivant sur [`REPLY_TOPIC`].
//!
//! Comme le reste du pont MQTT, cette partie ne touche pas au réseau : elle
//! renvoie des [`Action`]s, ce qui la rend facile à tester.

use ha_core::{Attributes, EntityId, StateMachine, slugify};
use serde_json::{Value, json};
use tracing::{info, warn};

use crate::bridge::Action;

/// Les abonnements : l'état de chaque sortie, la présence, et nos réponses.
pub const FILTERS: [&str; 3] = ["+/status/+", "+/online", REPLY_TOPIC];
/// Notre « adresse » pour les questions posées aux Shelly (`src`).
const CLIENT: &str = "ha-maison";
/// Où les Shelly répondent à nos questions.
pub const REPLY_TOPIC: &str = "ha-maison/rpc";
/// Longueur maximale d'un nom d'appareil.
const MAX_NAME_CHARS: usize = 60;
/// Appareils Shelly suivis au plus.
const MAX_DEVICES: usize = 100;
/// Sorties par appareil au plus (un Pro 4PM en a 4).
const MAX_CHANNELS: u8 = 4;
/// Taille maximale d'un message d'état.
const MAX_PAYLOAD: usize = 8 * 1024;

/// Un préfixe acceptable : 1 à 64 caractères simples. (Un préfixe avec des
/// `/` n'est pas pris en charge : celui d'usine n'en a pas.)
fn valid_prefix(prefix: &str) -> bool {
    (1..=64).contains(&prefix.len())
        && prefix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// `shellyplugsg3-a1b2/status/switch:0` → `("shellyplugsg3-a1b2", 0)`.
fn parse_status_topic(topic: &str) -> Option<(&str, u8)> {
    let mut levels = topic.split('/');
    let (prefix, status, component) = (levels.next()?, levels.next()?, levels.next()?);
    if levels.next().is_some() || status != "status" || !valid_prefix(prefix) {
        return None;
    }
    let channel: u8 = component.strip_prefix("switch:")?.parse().ok()?;
    (channel < MAX_CHANNELS).then_some((prefix, channel))
}

/// Une sortie d'un appareil Shelly : ses entités.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Channel {
    prefix: String,
    number: u8,
    switch: EntityId,
    /// La fin des identifiants des capteurs : `shellyplugsg3_a1b2`.
    base: String,
    /// Le nom donné dans l'appli Shelly, une fois connu.
    label: Option<String>,
}

/// Les appareils Shelly vus sur le broker.
#[derive(Debug, Default)]
pub struct Shellies {
    channels: Vec<Channel>,
}

impl Shellies {
    /// Combien de sorties sont suivies.
    #[must_use]
    pub fn len(&self) -> usize {
        self.channels.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    /// Traite un message. `None` : ce n'est pas un message Shelly, le pont de
    /// découverte s'en occupe.
    pub fn handle_message(
        &mut self,
        topic: &str,
        payload: &[u8],
        states: &StateMachine,
    ) -> Option<Vec<Action>> {
        if topic == REPLY_TOPIC {
            self.named(payload, states);
            return Some(Vec::new());
        }
        if let Some(prefix) = topic.strip_suffix("/online") {
            return self.online(prefix, payload, states);
        }
        let (prefix, number) = parse_status_topic(topic)?;
        if payload.len() > MAX_PAYLOAD {
            warn!("Shelly : message trop gros ignoré ({topic})");
            return Some(Vec::new());
        }
        let Ok(Value::Object(report)) = serde_json::from_slice::<Value>(payload) else {
            return None;
        };
        // Un vrai Shelly dit toujours si sa sortie est allumée.
        let on = report.get("output").and_then(Value::as_bool)?;
        let new = !self.channels.iter().any(|c| c.prefix == prefix);
        let channel = self.channel(prefix, number)?;
        publish(states, &channel, on, &report);
        // Un appareil jamais vu : on lui demande son nom.
        Some(if new {
            vec![Action::Publish {
                topic: format!("{prefix}/rpc"),
                payload: json!({"id": 1, "src": CLIENT, "method": "Shelly.GetDeviceInfo"})
                    .to_string(),
            }]
        } else {
            Vec::new()
        })
    }

    /// La réponse à `Shelly.GetDeviceInfo` : le nom de l'appareil, appliqué
    /// à toutes ses entités.
    fn named(&mut self, payload: &[u8], states: &StateMachine) {
        if payload.len() > MAX_PAYLOAD {
            return;
        }
        let Ok(reply) = serde_json::from_slice::<Value>(payload) else {
            return;
        };
        let (Some(device), Some(name)) = (
            reply.get("src").and_then(Value::as_str),
            reply
                .get("result")
                .and_then(|r| r.get("name"))
                .and_then(Value::as_str),
        ) else {
            return;
        };
        let name: String = name
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_NAME_CHARS)
            .collect::<String>()
            .trim()
            .to_owned();
        if name.is_empty() {
            return;
        }
        for channel in self.channels.iter_mut().filter(|c| c.prefix == device) {
            channel.label = Some(name.clone());
            let display = channel.name();
            for (entity_id, suffix) in channel.labelled_entities() {
                let Some(state) = states.get(&entity_id) else {
                    continue;
                };
                let mut attributes = state.attributes().clone();
                let wanted = match suffix {
                    Some(label) => format!("{display}, {label}"),
                    None => display.clone(),
                };
                attributes.insert("friendly_name".into(), wanted.into());
                set(states, entity_id, state.state().to_owned(), attributes);
            }
        }
    }

    /// `<préfixe>/online` : `false` rend ses entités indisponibles ; `true`
    /// (aussi au démarrage, le message étant gardé par le broker) demande
    /// l'état complet, pour ne pas attendre le prochain changement.
    fn online(&self, prefix: &str, payload: &[u8], states: &StateMachine) -> Option<Vec<Action>> {
        if !valid_prefix(prefix) {
            return None;
        }
        let known = self.channels.iter().any(|c| c.prefix == prefix);
        // Les préfixes d'usine commencent par « shelly » ; un appareil déjà
        // vu peut en avoir un autre. Les autres « …/online » ne nous
        // regardent pas.
        if !known && !prefix.starts_with("shelly") {
            return None;
        }
        match payload {
            b"true" => Some(vec![Action::Publish {
                topic: format!("{prefix}/command"),
                payload: "status_update".into(),
            }]),
            b"false" => {
                for channel in self.channels.iter().filter(|c| c.prefix == prefix) {
                    for entity_id in channel.entities() {
                        let attributes = states
                            .get(&entity_id)
                            .map(|state| state.attributes().clone())
                            .unwrap_or_default();
                        if let Err(error) =
                            states.set_by(crate::ID, entity_id, "unavailable", attributes)
                        {
                            warn!("Shelly : état refusé ({error})");
                        }
                    }
                }
                Some(Vec::new())
            }
            _ => Some(Vec::new()),
        }
    }

    /// La sortie, créée à sa première apparition.
    fn channel(&mut self, prefix: &str, number: u8) -> Option<Channel> {
        if let Some(channel) = self
            .channels
            .iter()
            .find(|c| c.prefix == prefix && c.number == number)
        {
            return Some(channel.clone());
        }
        let devices = self
            .channels
            .iter()
            .map(|c| c.prefix.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len();
        if devices >= MAX_DEVICES {
            warn!("Shelly : {MAX_DEVICES} appareils au plus, {prefix} ignoré");
            return None;
        }
        let mut base = slugify(prefix);
        if number > 0 {
            base = format!("{base}_{number}");
        }
        let switch: EntityId = format!("switch.{base}").parse().ok()?;
        info!("Shelly trouvé : {switch} ({prefix}, sortie {number})");
        let channel = Channel {
            prefix: prefix.to_owned(),
            number,
            switch,
            base,
            label: None,
        };
        self.channels.push(channel.clone());
        Some(channel)
    }

    /// Un ordre pour une prise Shelly : le message à publier.
    #[must_use]
    pub fn handle_service(&self, domain: &str, service: &str, entity_id: &EntityId) -> Vec<Action> {
        let Some(channel) = self.channels.iter().find(|c| &c.switch == entity_id) else {
            return Vec::new();
        };
        if domain != "switch" {
            return Vec::new();
        }
        let command = match service {
            "turn_on" => "on",
            "turn_off" => "off",
            "toggle" => "toggle",
            other => {
                warn!("Shelly : service switch.{other} non pris en charge");
                return Vec::new();
            }
        };
        vec![Action::Publish {
            topic: format!("{}/command/switch:{}", channel.prefix, channel.number),
            payload: command.into(),
        }]
    }
}

impl Channel {
    fn sensor(&self, key: &str) -> Option<EntityId> {
        format!("sensor.{}_{key}", self.base).parse().ok()
    }

    /// Toutes ses entités (pour les rendre indisponibles).
    fn entities(&self) -> Vec<EntityId> {
        std::iter::once(self.switch.clone())
            .chain(
                ["puissance", "energie", "tension"]
                    .into_iter()
                    .filter_map(|key| self.sensor(key)),
            )
            .collect()
    }

    /// Chaque entité, avec ce qui suit le nom (« puissance »…).
    fn labelled_entities(&self) -> Vec<(EntityId, Option<&'static str>)> {
        std::iter::once((self.switch.clone(), None))
            .chain(
                [
                    ("puissance", "puissance"),
                    ("energie", "énergie"),
                    ("tension", "tension"),
                ]
                .into_iter()
                .filter_map(|(key, label)| self.sensor(key).map(|id| (id, Some(label)))),
            )
            .collect()
    }

    /// « Lave-linge » (le nom donné dans l'appli), sinon « Shelly <préfixe> » ;
    /// « (2) » pour la deuxième sortie.
    fn name(&self) -> String {
        let base = self
            .label
            .clone()
            .unwrap_or_else(|| format!("Shelly {}", self.prefix));
        if self.number == 0 {
            base
        } else {
            format!("{base} ({})", self.number + 1)
        }
    }
}

fn set(states: &StateMachine, entity_id: EntityId, value: String, attributes: Attributes) {
    // Rien de changé : pas de nouvel état (l'historique reste léger).
    if states
        .get(&entity_id)
        .is_some_and(|s| s.state() == value && s.attributes() == &attributes)
    {
        return;
    }
    if let Err(error) = states.set_by(crate::ID, entity_id, value, attributes) {
        warn!("Shelly : état refusé ({error})");
    }
}

fn attributes(name: String, extra: &[(&str, Value)]) -> Attributes {
    let mut attributes = Attributes::new();
    attributes.insert("friendly_name".into(), name.into());
    for (key, value) in extra {
        attributes.insert((*key).into(), value.clone());
    }
    attributes
}

/// Un nombre fini, raisonnable.
fn number(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && v.abs() < 1e12)
}

fn publish(
    states: &StateMachine,
    channel: &Channel,
    on: bool,
    report: &serde_json::Map<String, Value>,
) {
    let name = channel.name();
    // Une prise (« plug » dans le préfixe d'usine) ou un relais.
    let mut switch = vec![("prefix", json!(channel.prefix))];
    if channel.prefix.to_lowercase().contains("plug") {
        switch.push(("device_class", json!("outlet")));
    }
    set(
        states,
        channel.switch.clone(),
        if on { "on" } else { "off" }.into(),
        attributes(name.clone(), &switch),
    );
    let measures = [
        (
            "puissance",
            "puissance",
            number(report.get("apower")),
            "W",
            "power",
            "measurement",
        ),
        (
            "energie",
            "énergie",
            // Le Shelly compte en Wh ; le bilan énergétique attend des kWh.
            number(report.get("aenergy").and_then(|e| e.get("total"))).map(|wh| wh / 1000.0),
            "kWh",
            "energy",
            "total_increasing",
        ),
        (
            "tension",
            "tension",
            number(report.get("voltage")),
            "V",
            "voltage",
            "measurement",
        ),
    ];
    for (key, label, value, unit, class, state_class) in measures {
        let (Some(value), Some(entity_id)) = (value, channel.sensor(key)) else {
            continue;
        };
        let text = if key == "energie" {
            format!("{value:.3}")
        } else {
            format!("{}", (value * 10.0).round() / 10.0)
        };
        set(
            states,
            entity_id,
            text,
            attributes(
                format!("{name}, {label}"),
                &[
                    ("unit_of_measurement", json!(unit)),
                    ("device_class", json!(class)),
                    ("state_class", json!(state_class)),
                ],
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;
    use ha_core::Core;

    const PLUG: &str = "shellyplugsg3-a1b2c3d4e5f6";

    fn state(core: &Core, id: &str) -> Option<(String, Attributes)> {
        core.states
            .get(&id.parse().unwrap())
            .map(|s| (s.state().to_owned(), s.attributes().clone()))
    }

    #[test]
    fn a_plug_appears_with_its_power_and_energy() {
        let core = Core::new();
        let mut shellies = Shellies::default();
        let status = br#"{"id":0,"source":"button","output":true,"apower":812.43,"voltage":231.2,"current":3.6,"aenergy":{"total":12345.6,"by_minute":[1,2,3]},"temperature":{"tC":41.2}}"#;
        let actions = shellies
            .handle_message(&format!("{PLUG}/status/switch:0"), status, &core.states)
            .unwrap();
        // Un nouvel appareil : on lui demande son nom.
        assert_eq!(
            actions,
            vec![Action::Publish {
                topic: format!("{PLUG}/rpc"),
                payload: r#"{"id":1,"method":"Shelly.GetDeviceInfo","src":"ha-maison"}"#.into(),
            }]
        );
        let (on, attributes) = state(&core, "switch.shellyplugsg3_a1b2c3d4e5f6").unwrap();
        assert_eq!(on, "on");
        assert_eq!(attributes["device_class"], "outlet");
        let (power, attributes) =
            state(&core, "sensor.shellyplugsg3_a1b2c3d4e5f6_puissance").unwrap();
        assert_eq!(power, "812.4");
        assert_eq!(attributes["unit_of_measurement"], "W");
        let (energy, attributes) =
            state(&core, "sensor.shellyplugsg3_a1b2c3d4e5f6_energie").unwrap();
        assert_eq!(energy, "12.346");
        assert_eq!(attributes["state_class"], "total_increasing");
        assert_eq!(
            state(&core, "sensor.shellyplugsg3_a1b2c3d4e5f6_tension")
                .unwrap()
                .0,
            "231.2"
        );
        // Les entités appartiennent à l'intégration MQTT (supervision, rapidité).
        assert_eq!(
            core.states
                .owner_of(&"switch.shellyplugsg3_a1b2c3d4e5f6".parse().unwrap())
                .as_deref(),
            Some(crate::ID)
        );

        // Son nom arrive : toutes ses entités le prennent.
        shellies.handle_message(
            REPLY_TOPIC,
            format!(r#"{{"id":1,"src":"{PLUG}","dst":"ha-maison","result":{{"name":"Lave-linge","id":"{PLUG}","model":"S3PL-00112EU"}}}}"#).as_bytes(),
            &core.states,
        );
        assert_eq!(
            state(&core, "switch.shellyplugsg3_a1b2c3d4e5f6").unwrap().1["friendly_name"],
            "Lave-linge"
        );
        assert_eq!(
            state(&core, "sensor.shellyplugsg3_a1b2c3d4e5f6_puissance")
                .unwrap()
                .1["friendly_name"],
            "Lave-linge, puissance"
        );

        // Éteinte : la prise suit, et garde son nom.
        shellies.handle_message(
            &format!("{PLUG}/status/switch:0"),
            br#"{"id":0,"output":false,"apower":0,"aenergy":{"total":12345.9}}"#,
            &core.states,
        );
        let (value, attributes) = state(&core, "switch.shellyplugsg3_a1b2c3d4e5f6").unwrap();
        assert_eq!(value, "off");
        assert_eq!(attributes["friendly_name"], "Lave-linge");

        // Hors ligne : tout devient indisponible.
        shellies.handle_message(&format!("{PLUG}/online"), b"false", &core.states);
        assert_eq!(
            state(&core, "sensor.shellyplugsg3_a1b2c3d4e5f6_puissance")
                .unwrap()
                .0,
            "unavailable"
        );
    }

    #[test]
    fn a_relay_without_metering_and_its_second_output() {
        let core = Core::new();
        let mut shellies = Shellies::default();
        shellies.handle_message(
            "shellyplus2pm-001122/status/switch:1",
            br#"{"id":1,"output":false}"#,
            &core.states,
        );
        let (value, attributes) = state(&core, "switch.shellyplus2pm_001122_1").unwrap();
        assert_eq!(value, "off");
        assert!(attributes.get("device_class").is_none(), "un relais");
        assert!(state(&core, "sensor.shellyplus2pm_001122_1_puissance").is_none());
        assert_eq!(
            shellies.handle_service(
                "switch",
                "toggle",
                &"switch.shellyplus2pm_001122_1".parse().unwrap()
            ),
            vec![Action::Publish {
                topic: "shellyplus2pm-001122/command/switch:1".into(),
                payload: "toggle".into(),
            }]
        );
    }

    #[test]
    fn orders_and_online_messages() {
        let core = Core::new();
        let mut shellies = Shellies::default();
        // En ligne (message gardé par le broker) : on demande l'état complet.
        assert_eq!(
            shellies.handle_message(&format!("{PLUG}/online"), b"true", &core.states),
            Some(vec![Action::Publish {
                topic: format!("{PLUG}/command"),
                payload: "status_update".into(),
            }])
        );
        // Un autre appareil qui publie « …/online » ne nous regarde pas.
        assert_eq!(
            shellies.handle_message("capteur-cave/online", b"true", &core.states),
            None
        );
        shellies.handle_message(
            &format!("{PLUG}/status/switch:0"),
            br#"{"output":false}"#,
            &core.states,
        );
        let switch: EntityId = "switch.shellyplugsg3_a1b2c3d4e5f6".parse().unwrap();
        assert_eq!(
            shellies.handle_service("switch", "turn_on", &switch),
            vec![Action::Publish {
                topic: format!("{PLUG}/command/switch:0"),
                payload: "on".into(),
            }]
        );
        assert_eq!(
            shellies.handle_service("light", "turn_on", &switch),
            Vec::new()
        );
        assert_eq!(
            shellies.handle_service("switch", "turn_on", &"switch.autre".parse().unwrap()),
            Vec::new()
        );
    }

    #[test]
    fn strange_messages_are_ignored() {
        let core = Core::new();
        let mut shellies = Shellies::default();
        for (topic, payload) in [
            // Pas un état de sortie, ou un sujet inattendu.
            ("zigbee2mqtt/status/bridge", &br#"{"output":true}"#[..]),
            ("shellyplug/status/switch:9", br#"{"output":true}"#),
            ("shellyplug/status/switch:x", br#"{"output":true}"#),
            ("a/b/status/switch:0", br#"{"output":true}"#),
            ("sh lly/status/switch:0", br#"{"output":true}"#),
            // Pas un Shelly : pas de « output ».
            ("capteur/status/switch:0", br#"{"temperature":21}"#),
            ("shellyplug/status/switch:0", b"pas du json"),
        ] {
            let _ = shellies.handle_message(topic, payload, &core.states);
        }
        assert!(shellies.is_empty());
        assert_eq!(core.states.all().len(), 0);
        // Un message géant est refusé.
        let huge = vec![b' '; MAX_PAYLOAD + 1];
        assert_eq!(
            shellies.handle_message("shellyplug/status/switch:0", &huge, &core.states),
            Some(Vec::new())
        );
        assert!(shellies.is_empty());
    }
}
