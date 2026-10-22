//! Les **automatisations toutes prêtes** (étape 85) : départ, retour, nuit,
//! matin, vacances. On coche ce qu'on veut (« éteindre les lumières »,
//! « chauffage à 17 °C »), et l'automatisation est fabriquée ici, d'après les
//! appareils de la maison, puis rangée avec les autres (`pret_depart`…).
//!
//! Ce module ne fait que fabriquer : une [`AutomationConfig`] ordinaire,
//! vérifiée comme si elle avait été écrite à la main, sans aucun droit de
//! plus. Les appareils sont ceux connus **au moment de l'enregistrement** :
//! un nouvel appareil compte dès qu'on enregistre à nouveau.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::AutomationConfig;

/// Les identifiants commencent tous par ce préfixe.
pub const ID_PREFIX: &str = "pret_";
/// Une liste d'appareils dans une action : au plus autant (comme partout).
const CHUNK: usize = 20;
/// Les ouvertures nommées dans le message, au plus.
const MAX_NAMED_OPENINGS: usize = 12;

/// Les automatisations toutes prêtes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Depart,
    Retour,
    Nuit,
    Matin,
    Vacances,
}

impl Preset {
    pub const ALL: [Self; 5] = [
        Self::Depart,
        Self::Retour,
        Self::Nuit,
        Self::Matin,
        Self::Vacances,
    ];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Depart => "depart",
            Self::Retour => "retour",
            Self::Nuit => "nuit",
            Self::Matin => "matin",
            Self::Vacances => "vacances",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| preset.name() == name)
    }

    /// L'identifiant de son automatisation : `pret_depart`.
    #[must_use]
    pub fn automation_id(self) -> String {
        format!("{ID_PREFIX}{}", self.name())
    }

    /// Son titre dans la liste.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Depart => "Départ",
            Self::Retour => "Retour",
            Self::Nuit => "Nuit",
            Self::Matin => "Matin",
            Self::Vacances => "Vacances",
        }
    }

    /// Ce qui la déclenche, en une phrase.
    #[must_use]
    pub fn when(self) -> &'static str {
        match self {
            Self::Depart => "Quand tout le monde est parti",
            Self::Retour => "Quand le premier rentre",
            Self::Nuit => "Chaque soir, si quelqu'un est là",
            Self::Matin => "Chaque matin, si quelqu'un est là",
            Self::Vacances => "Quand la maison est vide depuis longtemps",
        }
    }

    /// Pourquoi elle n'est pas possible dans cette maison, s'il le faut.
    #[must_use]
    pub fn unavailable(self, house: &House) -> Option<&'static str> {
        let needs_people = matches!(self, Self::Depart | Self::Retour | Self::Vacances);
        if needs_people && !house.people {
            return Some("il faut des personnes suivies (persons.yaml) pour savoir qui est là");
        }
        let nothing = match self {
            Self::Depart => {
                house.lights.is_empty() && house.covers.is_empty() && house.climates.is_empty()
            }
            Self::Retour => house.climates.is_empty() && house.lights.is_empty(),
            Self::Nuit => house.lights.is_empty() && house.covers.is_empty(),
            Self::Matin => house.covers.is_empty() && house.climates.is_empty(),
            Self::Vacances => house.climates.is_empty() && house.simulation.is_none(),
        };
        nothing.then_some("aucun appareil de la maison ne s'y prête")
    }

    /// Les réglages proposés au départ, d'après la maison.
    #[must_use]
    pub fn defaults(self, house: &House) -> Options {
        let heat = Some(!house.climates.is_empty());
        match self {
            Self::Depart => Options {
                delai_minutes: Some(10),
                lumieres: Some(!house.lights.is_empty()),
                volets: Some(false),
                chauffer: heat,
                chauffage: Some(17.0),
                ouvertures: Some(!house.openings.is_empty()),
                ..Options::default()
            },
            Self::Retour => Options {
                chauffer: heat,
                chauffage: Some(20.0),
                lumiere_entree: house
                    .lights
                    .iter()
                    .find(|light| light.entrance)
                    .filter(|_| house.sun)
                    .map(|light| light.entity_id.clone()),
                ..Options::default()
            },
            Self::Nuit => Options {
                heure: Some("23:00".into()),
                lumieres: Some(!house.lights.is_empty()),
                chambres: Some(false),
                volets: Some(!house.covers.is_empty()),
                ouvertures: Some(!house.openings.is_empty()),
                chauffer: Some(false),
                chauffage: Some(18.0),
                ..Options::default()
            },
            Self::Matin => Options {
                heure: Some("07:00".into()),
                heure_weekend: Some("09:00".into()),
                volets: Some(!house.covers.is_empty()),
                chambres: Some(false),
                chauffer: Some(false),
                chauffage: Some(20.0),
                ..Options::default()
            },
            Self::Vacances => Options {
                apres_heures: Some(24),
                simulation: Some(house.simulation.is_some()),
                chauffer: heat,
                chauffage: Some(12.0),
                ..Options::default()
            },
        }
    }
}

/// Un appareil de la maison, pour les automatisations toutes prêtes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Device {
    pub entity_id: String,
    pub name: String,
    /// Dans une chambre : gardé allumé la nuit, volet fermé le matin.
    pub bedroom: bool,
    /// Dans l'entrée ou le couloir : la lumière du retour.
    pub entrance: bool,
}

/// Une porte, une fenêtre, un portail : ouvert quand son état vaut `open_state`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Opening {
    pub entity_id: String,
    pub name: String,
    pub open_state: String,
}

/// Ce que la maison a, pour fabriquer les automatisations.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct House {
    /// Les lumières (pas les prises).
    pub lights: Vec<Device>,
    /// Les volets et stores (pas le garage ni le portail).
    pub covers: Vec<Device>,
    pub climates: Vec<Device>,
    pub openings: Vec<Opening>,
    /// Où envoyer les messages (`notify.appli`) ; sinon, le tableau de bord.
    pub notify: Option<String>,
    /// La simulation de présence (`switch.simulation_presence`).
    pub simulation: Option<String>,
    /// `zone.home` existe : on sait qui est là.
    pub people: bool,
    /// `sun.sun` existe : on sait s'il fait nuit.
    pub sun: bool,
}

/// Ce qu'on a coché. Chaque automatisation n'en lit qu'une partie.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    /// Départ : depuis combien de minutes la maison est vide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delai_minutes: Option<u16>,
    /// Vacances : depuis combien d'heures la maison est vide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apres_heures: Option<u16>,
    /// Nuit, matin (en semaine) : `HH:MM`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heure: Option<String>,
    /// Matin, le samedi et le dimanche.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heure_weekend: Option<String>,
    /// Éteindre les lumières.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lumieres: Option<bool>,
    /// Fermer (ou ouvrir, le matin) les volets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volets: Option<bool>,
    /// Les chambres aussi (lumières la nuit, volets le matin).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chambres: Option<bool>,
    /// Prévenir si une porte ou une fenêtre est restée ouverte.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ouvertures: Option<bool>,
    /// Changer la consigne des thermostats…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chauffer: Option<bool>,
    /// … à cette température, en °C.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chauffage: Option<f64>,
    /// Retour : la lumière à allumer s'il fait nuit (vide : aucune).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lumiere_entree: Option<String>,
    /// Vacances : armer la simulation de présence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulation: Option<bool>,
}

impl Options {
    /// Les réglages vides prennent la valeur proposée (`defaults`).
    #[must_use]
    pub fn or(self, defaults: Self) -> Self {
        Self {
            delai_minutes: self.delai_minutes.or(defaults.delai_minutes),
            apres_heures: self.apres_heures.or(defaults.apres_heures),
            heure: self.heure.or(defaults.heure),
            heure_weekend: self.heure_weekend.or(defaults.heure_weekend),
            lumieres: self.lumieres.or(defaults.lumieres),
            volets: self.volets.or(defaults.volets),
            chambres: self.chambres.or(defaults.chambres),
            ouvertures: self.ouvertures.or(defaults.ouvertures),
            chauffer: self.chauffer.or(defaults.chauffer),
            chauffage: self.chauffage.or(defaults.chauffage),
            lumiere_entree: self.lumiere_entree.or(defaults.lumiere_entree),
            simulation: self.simulation.or(defaults.simulation),
        }
    }
}

/// `HH:MM`, de 00:00 à 23:59.
fn check_time(name: &str, value: Option<&String>) -> Result<String, String> {
    let value = value.ok_or_else(|| format!("{name} : heure manquante"))?;
    let valid = value.len() == 5
        && value.is_ascii()
        && value.as_bytes()[2] == b':'
        && value[..2].parse::<u8>().is_ok_and(|h| h < 24)
        && value[3..].parse::<u8>().is_ok_and(|m| m < 60)
        && value
            .bytes()
            .enumerate()
            .all(|(i, b)| i == 2 || b.is_ascii_digit());
    if valid {
        Ok(format!("{value}:00"))
    } else {
        Err(format!("{name} : « {value} » n'est pas une heure (HH:MM)"))
    }
}

/// La consigne : de 5 à 30 °C, comme les thermostats.
fn check_heating(value: Option<f64>) -> Result<Option<f64>, String> {
    match value {
        Some(t) if !(5.0..=30.0).contains(&t) || !t.is_finite() => {
            Err(format!("chauffage : {t} °C hors de la plage (5 à 30 °C)"))
        }
        other => Ok(other),
    }
}

fn ids(devices: &[&Device]) -> Vec<String> {
    devices.iter().map(|d| d.entity_id.clone()).collect()
}

/// Un ordre aux appareils, par paquets de 20.
fn order(action: &str, entities: &[String], data: Option<&Value>) -> Vec<Value> {
    entities
        .chunks(CHUNK)
        .map(|chunk| {
            let mut out = json!({ "action": action, "target": { "entity_id": chunk } });
            if let Some(data) = data {
                out["data"] = data.clone();
            }
            out
        })
        .collect()
}

fn heating(house: &House, temperature: Option<f64>) -> Vec<Value> {
    match temperature {
        Some(t) if !house.climates.is_empty() => order(
            "climate.set_temperature",
            &ids(&house.climates.iter().collect::<Vec<_>>()),
            Some(&json!({ "temperature": t })),
        ),
        _ => Vec::new(),
    }
}

/// Un message : sur le téléphone (`notify`), sinon sur le tableau de bord
/// (`persistent_notification.pret_…`, un par automatisation).
fn message(house: &House, preset: Preset, title: &str, text: &str) -> Value {
    match &house.notify {
        Some(notify) => json!({
            "action": "notify.send_message",
            "target": { "entity_id": notify },
            "data": { "title": title, "message": text },
        }),
        None => json!({
            "action": "persistent_notification.create",
            "target": { "entity_id": format!("persistent_notification.{}", preset.automation_id()) },
            "data": { "title": title, "message": text },
        }),
    }
}

/// « S'il reste une porte ou une fenêtre ouverte : prévenir, avec leurs noms. »
fn openings_check(house: &House, preset: Preset, title: &str) -> Option<Value> {
    if house.openings.is_empty() {
        return None;
    }
    let openings = &house.openings[..house.openings.len().min(CHUNK)];
    let conditions: Vec<Value> = openings
        .iter()
        .map(|o| json!({ "condition": "state", "entity_id": o.entity_id, "state": o.open_state }))
        .collect();
    let named: Vec<String> = openings
        .iter()
        .take(MAX_NAMED_OPENINGS)
        .map(|o| {
            let name: String = o.name.chars().filter(|c| *c != '"' && *c != '\\').collect();
            format!(
                "(\"{name}. \" if states('{}') == '{}' else \"\")",
                o.entity_id, o.open_state
            )
        })
        .collect();
    let text = format!("Encore ouvert : {{{{ {} }}}}", named.join(" ~ "));
    Some(json!({
        "if": [{ "condition": "or", "conditions": conditions }],
        "then": [message(house, preset, title, &text)],
    }))
}

fn someone_home() -> Value {
    json!({ "condition": "not", "conditions": [
        { "condition": "state", "entity_id": "zone.home", "state": "0" }
    ]})
}

/// Fabrique l'automatisation `preset` avec ces réglages. `enabled` : active
/// ou non (décochée, elle est gardée désactivée).
pub fn build(
    preset: Preset,
    options: &Options,
    house: &House,
    enabled: bool,
) -> Result<AutomationConfig, String> {
    if let Some(why) = preset.unavailable(house) {
        return Err(format!("« {} » impossible : {why}", preset.title()));
    }
    let o = options.clone().or(preset.defaults(house));
    let chauffage = check_heating(o.chauffage)?.filter(|_| o.chauffer == Some(true));
    let lights: Vec<&Device> = house.lights.iter().collect();
    let covers: Vec<&Device> = house.covers.iter().collect();
    let (triggers, conditions, actions, alias): (Vec<Value>, Vec<Value>, Vec<Value>, &str) =
        match preset {
            Preset::Depart => {
                let delay = o.delai_minutes.unwrap_or(10);
                if !(1..=240).contains(&delay) {
                    return Err("délai : de 1 à 240 minutes".into());
                }
                let mut actions = Vec::new();
                if o.lumieres == Some(true) {
                    actions.extend(order("light.turn_off", &ids(&lights), None));
                }
                if o.volets == Some(true) {
                    actions.extend(order("cover.close_cover", &ids(&covers), None));
                }
                actions.extend(heating(house, chauffage));
                if o.ouvertures == Some(true) {
                    actions.extend(openings_check(house, preset, "Tout le monde est parti"));
                }
                (
                    vec![json!({
                        "trigger": "state", "entity_id": "zone.home", "to": "0",
                        "for": format!("{:02}:{:02}:00", delay / 60, delay % 60),
                    })],
                    Vec::new(),
                    actions,
                    "Départ : la maison se met en veille",
                )
            }
            Preset::Retour => {
                let mut actions = heating(house, chauffage);
                if let Some(light) = o.lumiere_entree.as_ref().filter(|l| !l.is_empty()) {
                    if !house.lights.iter().any(|l| l.entity_id == *light) {
                        return Err(format!("lumière inconnue : {light}"));
                    }
                    let turn_on =
                        json!({ "action": "light.turn_on", "target": { "entity_id": light } });
                    actions.push(if house.sun {
                        json!({
                            "if": [{ "condition": "state", "entity_id": "sun.sun", "state": "below_horizon" }],
                            "then": [turn_on],
                        })
                    } else {
                        turn_on
                    });
                }
                (
                    vec![json!({
                        "trigger": "state", "entity_id": "zone.home", "from": "0",
                        "not_to": ["unavailable", "unknown"],
                    })],
                    Vec::new(),
                    actions,
                    "Retour : la maison accueille",
                )
            }
            Preset::Nuit => {
                let at = check_time("heure", o.heure.as_ref())?;
                let mut actions = Vec::new();
                if o.lumieres == Some(true) {
                    let keep_bedrooms = o.chambres != Some(true);
                    let off: Vec<&Device> = lights
                        .iter()
                        .copied()
                        .filter(|l| !(keep_bedrooms && l.bedroom))
                        .collect();
                    actions.extend(order("light.turn_off", &ids(&off), None));
                }
                if o.volets == Some(true) {
                    actions.extend(order("cover.close_cover", &ids(&covers), None));
                }
                actions.extend(heating(house, chauffage));
                if o.ouvertures == Some(true) {
                    actions.extend(openings_check(house, preset, "Bonne nuit"));
                }
                (
                    vec![json!({ "trigger": "time", "at": at })],
                    if house.people {
                        vec![someone_home()]
                    } else {
                        Vec::new()
                    },
                    actions,
                    "Nuit : la maison se couche",
                )
            }
            Preset::Matin => {
                let week = check_time("heure", o.heure.as_ref())?;
                let weekend = check_time("heure du week-end", o.heure_weekend.as_ref())?;
                let mut actions = Vec::new();
                if o.volets == Some(true) {
                    let with_bedrooms = o.chambres == Some(true);
                    let open: Vec<&Device> = covers
                        .iter()
                        .copied()
                        .filter(|c| with_bedrooms || !c.bedroom)
                        .collect();
                    actions.extend(order("cover.open_cover", &ids(&open), None));
                }
                actions.extend(heating(house, chauffage));
                let mut conditions = vec![json!({ "condition": "or", "conditions": [
                    { "condition": "and", "conditions": [
                        { "condition": "trigger", "id": "semaine" },
                        { "condition": "time", "weekday": ["mon", "tue", "wed", "thu", "fri"] },
                    ]},
                    { "condition": "and", "conditions": [
                        { "condition": "trigger", "id": "week-end" },
                        { "condition": "time", "weekday": ["sat", "sun"] },
                    ]},
                ]})];
                if house.people {
                    conditions.push(someone_home());
                }
                (
                    vec![
                        json!({ "trigger": "time", "at": week, "id": "semaine" }),
                        json!({ "trigger": "time", "at": weekend, "id": "week-end" }),
                    ],
                    conditions,
                    actions,
                    "Matin : la maison se réveille",
                )
            }
            Preset::Vacances => {
                let hours = o.apres_heures.unwrap_or(24);
                if !(1..=24).contains(&hours) {
                    return Err("vacances : de 1 à 24 heures".into());
                }
                let simulation = house
                    .simulation
                    .as_ref()
                    .filter(|_| o.simulation == Some(true));
                let mut away = heating(house, chauffage);
                let mut back = Vec::new();
                if let Some(switch) = simulation {
                    away.push(
                        json!({ "action": "switch.turn_on", "target": { "entity_id": switch } }),
                    );
                    back.push(
                        json!({ "action": "switch.turn_off", "target": { "entity_id": switch } }),
                    );
                }
                let mut said = Vec::new();
                if chauffage.is_some() && !house.climates.is_empty() {
                    said.push(format!(
                        "chauffage à {} °C",
                        show(chauffage.unwrap_or_default())
                    ));
                }
                if simulation.is_some() {
                    said.push("simulation de présence armée".to_owned());
                }
                away.push(message(
                    house,
                    preset,
                    "Mode vacances",
                    &format!("La maison est vide : {}.", said.join(", ")),
                ));
                let mut choose = vec![json!({
                    "conditions": [{ "condition": "trigger", "id": "absents" }],
                    "sequence": away,
                })];
                if !back.is_empty() {
                    choose.push(json!({
                        "conditions": [{ "condition": "trigger", "id": "retour" }],
                        "sequence": back,
                    }));
                }
                (
                    vec![
                        json!({
                            "trigger": "state", "entity_id": "zone.home", "to": "0",
                            "for": format!("{hours}h"), "id": "absents",
                        }),
                        json!({
                            "trigger": "state", "entity_id": "zone.home", "from": "0",
                            "not_to": ["unavailable", "unknown"], "id": "retour",
                        }),
                    ],
                    Vec::new(),
                    vec![json!({ "choose": choose })],
                    "Vacances : la maison se garde seule",
                )
            }
        };
    if actions.is_empty() {
        return Err(format!(
            "« {} » : rien à faire, coche au moins une chose",
            preset.title()
        ));
    }
    let mut config = json!({
        "id": preset.automation_id(),
        "alias": alias,
        "triggers": triggers,
        "actions": actions,
    });
    if !conditions.is_empty() {
        config["conditions"] = Value::from(conditions);
    }
    if !enabled {
        config["enabled"] = Value::from(false);
    }
    let config = AutomationConfig::from_json(config).map_err(|error| error.to_string())?;
    // Vérifiée comme une automatisation écrite à la main.
    config.check().map_err(|error| error.to_string())?;
    Ok(config)
}

fn show(n: f64) -> String {
    let text = if n.fract() == 0.0 {
        format!("{n:.0}")
    } else {
        format!("{n}")
    };
    text.replace('.', ",")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    fn device(id: &str, name: &str) -> Device {
        Device {
            entity_id: id.into(),
            name: name.into(),
            bedroom: id.contains("chambre"),
            entrance: id.contains("entree"),
        }
    }

    fn house() -> House {
        House {
            lights: vec![
                device("light.salon", "Salon"),
                device("light.entree", "Entrée"),
                device("light.chambre", "Chambre"),
            ],
            covers: vec![
                device("cover.salon", "Volet salon"),
                device("cover.chambre", "Volet chambre"),
            ],
            climates: vec![device("climate.salon", "Chauffage salon")],
            openings: vec![
                Opening {
                    entity_id: "binary_sensor.porte_entree".into(),
                    name: "Porte d'entrée".into(),
                    open_state: "on".into(),
                },
                Opening {
                    entity_id: "cover.garage".into(),
                    name: "Garage".into(),
                    open_state: "open".into(),
                },
            ],
            notify: Some("notify.appli".into()),
            simulation: Some("switch.simulation_presence".into()),
            people: true,
            sun: true,
        }
    }

    #[allow(clippy::needless_pass_by_value, reason = "plus lisible dans les tests")]
    fn built(preset: Preset, options: Options) -> Value {
        build(preset, &options, &house(), true).unwrap().to_json()
    }

    #[test]
    fn every_preset_builds_a_valid_automation() {
        for preset in Preset::ALL {
            let config = build(preset, &Options::default(), &house(), true).unwrap();
            assert_eq!(config.id(), format!("pret_{}", preset.name()));
            assert!(config.enabled());
            let off = build(preset, &Options::default(), &house(), false).unwrap();
            assert!(!off.enabled());
            assert_eq!(Preset::from_name(preset.name()), Some(preset));
        }
    }

    #[test]
    fn departure_turns_everything_down() {
        let config = built(Preset::Depart, Options::default());
        assert_eq!(config["triggers"][0]["to"], "0");
        assert_eq!(config["triggers"][0]["for"], "00:10:00");
        let actions = config["actions"].as_array().unwrap();
        assert_eq!(actions[0]["action"], "light.turn_off");
        assert_eq!(
            actions[0]["target"]["entity_id"],
            json!(["light.salon", "light.entree", "light.chambre"])
        );
        assert_eq!(actions[1]["action"], "climate.set_temperature");
        assert_eq!(actions[1]["data"]["temperature"], 17.0);
        // Les ouvertures : un message avec leurs noms.
        let message = &actions[2]["then"][0];
        assert_eq!(message["action"], "notify.send_message");
        let text = message["data"]["message"].as_str().unwrap();
        assert!(
            text.contains("\"Porte d'entrée. \" if states('binary_sensor.porte_entree') == 'on'"),
            "{text}"
        );
        assert_eq!(actions.len(), 3, "les volets ne sont pas cochés");

        // Les volets cochés, le chauffage décoché, 45 minutes.
        let config = built(
            Preset::Depart,
            Options {
                volets: Some(true),
                chauffer: Some(false),
                delai_minutes: Some(45),
                ouvertures: Some(false),
                ..Options::default()
            },
        );
        let actions: Vec<&str> = config["actions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["action"].as_str().unwrap())
            .collect();
        assert_eq!(actions, ["light.turn_off", "cover.close_cover"]);
        assert_eq!(config["triggers"][0]["for"], "00:45:00");
    }

    /// Le message dit ce qui est resté ouvert, et seulement ça.
    #[test]
    fn the_message_names_what_is_open() {
        let config = built(Preset::Nuit, Options::default());
        let last = config["actions"].as_array().unwrap().last().unwrap();
        let text = last["then"][0]["data"]["message"].as_str().unwrap();
        let template = crate::Template::parse(text).unwrap().unwrap();
        let core = ha_core::Core::new();
        let states = &core.states;
        let set = |id: &str, state: &str| {
            states
                .set(id.parse().unwrap(), state, ha_core::Attributes::new())
                .unwrap();
        };
        set("binary_sensor.porte_entree", "off");
        set("cover.garage", "open");
        assert_eq!(template.evaluate(states, None), "Encore ouvert : Garage. ");
        set("binary_sensor.porte_entree", "on");
        assert_eq!(
            template.evaluate(states, None),
            "Encore ouvert : Porte d'entrée. Garage. "
        );
    }

    #[test]
    fn night_keeps_bedrooms_and_morning_spares_them() {
        let config = built(Preset::Nuit, Options::default());
        assert_eq!(config["triggers"][0]["at"], "23:00:00");
        assert_eq!(
            config["actions"][0]["target"]["entity_id"],
            json!(["light.salon", "light.entree"])
        );
        assert_eq!(config["conditions"][0]["condition"], "not");
        let config = built(
            Preset::Nuit,
            Options {
                chambres: Some(true),
                heure: Some("22:30".into()),
                ..Options::default()
            },
        );
        assert_eq!(config["triggers"][0]["at"], "22:30:00");
        assert_eq!(
            config["actions"][0]["target"]["entity_id"]
                .as_array()
                .unwrap()
                .len(),
            3
        );

        let config = built(Preset::Matin, Options::default());
        assert_eq!(config["actions"][0]["action"], "cover.open_cover");
        assert_eq!(
            config["actions"][0]["target"]["entity_id"],
            json!(["cover.salon"])
        );
        assert_eq!(config["triggers"][1]["at"], "09:00:00");
    }

    #[test]
    fn return_lights_the_entrance_at_night() {
        let config = built(Preset::Retour, Options::default());
        assert_eq!(config["triggers"][0]["from"], "0");
        assert_eq!(config["actions"][0]["data"]["temperature"], 20.0);
        assert_eq!(config["actions"][1]["if"][0]["entity_id"], "sun.sun");
        assert_eq!(
            config["actions"][1]["then"][0]["target"]["entity_id"],
            "light.entree"
        );
        // Aucune lumière choisie.
        let config = built(
            Preset::Retour,
            Options {
                lumiere_entree: Some(String::new()),
                ..Options::default()
            },
        );
        assert_eq!(config["actions"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn holidays_arm_the_simulation_and_disarm_it_back() {
        let config = built(Preset::Vacances, Options::default());
        assert_eq!(config["triggers"][0]["for"], "24h");
        let away = &config["actions"][0]["choose"][0]["sequence"];
        assert_eq!(away[0]["data"]["temperature"], 12.0);
        assert_eq!(away[1]["action"], "switch.turn_on");
        assert_eq!(
            away[2]["data"]["message"],
            "La maison est vide : chauffage à 12 °C, simulation de présence armée."
        );
        let back = &config["actions"][0]["choose"][1]["sequence"];
        assert_eq!(back[0]["action"], "switch.turn_off");
    }

    #[test]
    fn wrong_options_are_refused() {
        let refused = |preset, options| build(preset, &options, &house(), true).unwrap_err();
        let error = refused(
            Preset::Nuit,
            Options {
                heure: Some("25:00".into()),
                ..Options::default()
            },
        );
        assert!(error.contains("n'est pas une heure"), "{error}");
        let error = refused(
            Preset::Depart,
            Options {
                chauffage: Some(40.0),
                ..Options::default()
            },
        );
        assert!(error.contains("hors de la plage"), "{error}");
        let error = refused(
            Preset::Retour,
            Options {
                lumiere_entree: Some("light.inconnue".into()),
                ..Options::default()
            },
        );
        assert!(error.contains("inconnue"), "{error}");
        let error = refused(
            Preset::Depart,
            Options {
                lumieres: Some(false),
                chauffer: Some(false),
                ouvertures: Some(false),
                ..Options::default()
            },
        );
        assert!(error.contains("coche au moins"), "{error}");
        assert!(check_time("heure", Some(&"é:000".to_owned())).is_err());
    }

    #[test]
    fn a_house_without_people_cannot_leave() {
        let mut lonely = house();
        lonely.people = false;
        assert!(Preset::Depart.unavailable(&lonely).is_some());
        assert!(build(Preset::Depart, &Options::default(), &lonely, true).is_err());
        // La nuit, si : sans savoir qui est là.
        let config = build(Preset::Nuit, &Options::default(), &lonely, true)
            .unwrap()
            .to_json();
        assert!(config.get("conditions").is_none());
        // Sans téléphone : le message va sur le tableau de bord.
        lonely.notify = None;
        let config = build(Preset::Nuit, &Options::default(), &lonely, true)
            .unwrap()
            .to_json();
        let last = config["actions"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .clone();
        assert_eq!(last["then"][0]["action"], "persistent_notification.create");
    }

    #[test]
    fn many_lights_are_split_in_chunks() {
        let mut big = house();
        big.lights = (0..45)
            .map(|i| device(&format!("light.l{i}"), &format!("L{i}")))
            .collect();
        let config = built_with(&big);
        let lights: Vec<usize> = config["actions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| a["action"] == "light.turn_off")
            .map(|a| a["target"]["entity_id"].as_array().unwrap().len())
            .collect();
        assert_eq!(lights, [20, 20, 5]);
    }

    fn built_with(house: &House) -> Value {
        build(Preset::Depart, &Options::default(), house, true)
            .unwrap()
            .to_json()
    }
}
