//! Les appareils SmartThings comme entités de la maison. Cette partie ne
//! touche pas au réseau : elle est testée sans compte Samsung.
//!
//! | SmartThings (capacités) | Entité | Commandable |
//! |---|---|---|
//! | `switch` + `switchLevel`/`colorTemperature`, ou catégorie `Light` | `light.*` | allumer, éteindre, luminosité, blanc |
//! | `windowShade` (`windowShadeLevel`) | `cover.*` (volet) | ouvrir, fermer, arrêter, position |
//! | `doorControl` | `cover.*` (garage) | ouvrir, fermer (protégé comme un garage) |
//! | `thermostatHeatingSetpoint` (`thermostatMode`) | `climate.*` | consigne, mode |
//! | `airConditionerMode` (climatiseur Samsung) | `climate.*` | consigne, mode, marche/arrêt |
//! | `lock` | `lock.*` | verrouiller, déverrouiller (mot de passe redemandé) |
//! | `washerOperatingState`, `dryerOperatingState`, `dishwasherOperatingState` | `sensor.*` : « en marche », « arrêté »… | non |
//! | `switch` seul (prise, télé…) | `switch.*` | allumer, éteindre |
//! | `contactSensor`, `motionSensor`, `waterSensor`, `smokeDetector`, `presenceSensor` | `binary_sensor.*` | non |
//! | `temperatureMeasurement`, `relativeHumidityMeasurement`, `illuminanceMeasurement`, `battery`, `powerMeter`, `energyMeter` | `sensor.*` | non |

use std::collections::HashMap;

use ha_core::{Attributes, EntityId, StateMachine};
use serde_json::{Value, json};
use tracing::{debug, info, warn};

use crate::api::{Command, Device, MAX_DEVICES};

/// Ce que fait l'appareil, avant tout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Light,
    Switch,
    Shade,
    Garage,
    Thermostat,
    AirConditioner,
    Lock,
    /// Un lave-linge, sèche-linge ou lave-vaisselle.
    Appliance(Machine),
}

/// Les machines à cycle de Samsung (et des autres marques SmartThings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Machine {
    Washer,
    Dryer,
    Dishwasher,
}

impl Machine {
    const ALL: [Self; 3] = [Self::Washer, Self::Dryer, Self::Dishwasher];

    /// La capacité standard : `machineState` (`run`, `pause`, `stop`),
    /// `…JobState`, `completionTime`.
    fn capability(self) -> &'static str {
        match self {
            Self::Washer => "washerOperatingState",
            Self::Dryer => "dryerOperatingState",
            Self::Dishwasher => "dishwasherOperatingState",
        }
    }

    /// La capacité propre à Samsung : `operatingState` (`running`, `paused`,
    /// `ready`), `jobState`, `remainingTime` (minutes), `progress` (%).
    fn samsung_capability(self) -> &'static str {
        match self {
            Self::Washer => "samsungce.washerOperatingState",
            Self::Dryer => "samsungce.dryerOperatingState",
            Self::Dishwasher => "samsungce.dishwasherOperatingState",
        }
    }

    fn job_attribute(self) -> &'static str {
        match self {
            Self::Washer => "washerJobState",
            Self::Dryer => "dryerJobState",
            Self::Dishwasher => "dishwasherJobState",
        }
    }

    /// « lave-linge », pour l'attribut `appareil`.
    fn name(self) -> &'static str {
        match self {
            Self::Washer => "lave-linge",
            Self::Dryer => "sèche-linge",
            Self::Dishwasher => "lave-vaisselle",
        }
    }
}

/// Où en est une machine : les mêmes états que les appareils à cycle
/// suivis par leur prise (étape 67), plus la pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Off,
    Running,
    Paused,
    Finished,
}

impl Phase {
    /// L'état publié.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "Arrêt",
            Self::Running => "En marche",
            Self::Paused => "En pause",
            Self::Finished => "Terminé",
        }
    }
}

/// Le cycle en cours d'une machine, et le dernier terminé.
#[derive(Debug, Clone, Default)]
struct Cycle {
    phase: Phase,
    started: Option<std::time::SystemTime>,
    energy_at_start: Option<f64>,
    /// Le dernier cycle : (minutes, kWh si connus).
    last: Option<(u64, Option<f64>)>,
}

/// Un cycle qui vient de se terminer : l'évènement `appareil_termine`.
#[derive(Debug, Clone, PartialEq)]
pub struct Finished {
    pub entity_id: EntityId,
    pub name: String,
    pub minutes: u64,
    pub energy_kwh: Option<f64>,
}

impl Kind {
    fn of(device: &Device) -> Option<Self> {
        let has = |name: &str| device.capabilities.iter().any(|c| c == name);
        let category = |name: &str| device.categories.iter().any(|c| c == name);
        for machine in Machine::ALL {
            if has(machine.capability()) || has(machine.samsung_capability()) {
                return Some(Self::Appliance(machine));
            }
        }
        Some(if has("airConditionerMode") {
            Self::AirConditioner
        } else if has("thermostatHeatingSetpoint") {
            Self::Thermostat
        } else if has("windowShade") {
            Self::Shade
        } else if has("doorControl") {
            Self::Garage
        } else if has("lock") {
            Self::Lock
        } else if has("switch")
            && (category("Light")
                || has("colorTemperature")
                || has("colorControl")
                || (has("switchLevel") && !category("SmartPlug") && !category("Switch")))
        {
            Self::Light
        } else if has("switch") {
            Self::Switch
        } else {
            return None;
        })
    }

    fn domain(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Switch => "switch",
            Self::Shade | Self::Garage => "cover",
            Self::Thermostat | Self::AirConditioner => "climate",
            Self::Lock => "lock",
            Self::Appliance(_) => "sensor",
        }
    }
}

/// Une mesure ou un détecteur, à côté de l'appareil.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sensor {
    Contact,
    Motion,
    Water,
    Smoke,
    Presence,
    Temperature,
    Humidity,
    Illuminance,
    Battery,
    Power,
    Energy,
}

impl Sensor {
    /// Dans l'ordre de préférence pour l'entité principale d'un capteur seul.
    const ALL: [Self; 11] = [
        Self::Contact,
        Self::Motion,
        Self::Water,
        Self::Smoke,
        Self::Presence,
        Self::Temperature,
        Self::Humidity,
        Self::Illuminance,
        Self::Power,
        Self::Energy,
        Self::Battery,
    ];

    fn capability(self) -> (&'static str, &'static str) {
        match self {
            Self::Contact => ("contactSensor", "contact"),
            Self::Motion => ("motionSensor", "motion"),
            Self::Water => ("waterSensor", "water"),
            Self::Smoke => ("smokeDetector", "smoke"),
            Self::Presence => ("presenceSensor", "presence"),
            Self::Temperature => ("temperatureMeasurement", "temperature"),
            Self::Humidity => ("relativeHumidityMeasurement", "humidity"),
            Self::Illuminance => ("illuminanceMeasurement", "illuminance"),
            Self::Battery => ("battery", "battery"),
            Self::Power => ("powerMeter", "power"),
            Self::Energy => ("energyMeter", "energy"),
        }
    }

    fn binary(self) -> bool {
        matches!(
            self,
            Self::Contact | Self::Motion | Self::Water | Self::Smoke | Self::Presence
        )
    }

    /// Le mot ajouté au nom : « Porte entrée Batterie ».
    fn suffix(self) -> (&'static str, &'static str) {
        match self {
            Self::Contact => ("ouverture", "Ouverture"),
            Self::Motion => ("mouvement", "Mouvement"),
            Self::Water => ("fuite", "Fuite"),
            Self::Smoke => ("fumee", "Fumée"),
            Self::Presence => ("presence", "Présence"),
            Self::Temperature => ("temperature", "Température"),
            Self::Humidity => ("humidite", "Humidité"),
            Self::Illuminance => ("luminosite", "Luminosité"),
            Self::Battery => ("batterie", "Batterie"),
            Self::Power => ("puissance", "Puissance"),
            Self::Energy => ("energie", "Énergie"),
        }
    }
}

/// Un appareil suivi : ce qu'il est, ses entités, son dernier état.
#[derive(Debug)]
struct Tracked {
    label: String,
    room: Option<String>,
    kind: Option<Kind>,
    capabilities: Vec<String>,
    categories: Vec<String>,
    primary: Option<EntityId>,
    sensors: Vec<(Sensor, EntityId)>,
    status: Value,
    available: bool,
    cycle: Cycle,
}

impl Tracked {
    fn has(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c == capability)
    }

    /// `status[capacité][attribut].value`.
    fn value(&self, capability: &str, attribute: &str) -> Option<&Value> {
        self.status
            .get(capability)?
            .get(attribute)?
            .get("value")
            .filter(|v| !v.is_null())
    }

    fn text(&self, capability: &str, attribute: &str) -> Option<&str> {
        self.value(capability, attribute).and_then(Value::as_str)
    }

    fn number(&self, capability: &str, attribute: &str) -> Option<f64> {
        self.value(capability, attribute).and_then(as_f64)
    }

    /// L'énergie totale consommée, en kWh : `energyMeter`, sinon le relevé
    /// des appareils Samsung (`powerConsumptionReport`, en Wh).
    fn energy_kwh(&self) -> Option<f64> {
        if let Some(value) = self.number("energyMeter", "energy") {
            return Some(if self.unit("energyMeter", "energy") == Some("Wh") {
                value / 1000.0
            } else {
                value
            });
        }
        self.value("powerConsumptionReport", "powerConsumption")?
            .get("energy")
            .and_then(as_f64)
            .map(|wh| wh / 1000.0)
    }

    /// Ce que dit la machine : en marche, en pause, à l'arrêt (`None` :
    /// inconnu), et si son programme est fini.
    fn machine(&self, machine: Machine) -> (Option<Phase>, bool) {
        let samsung = machine.samsung_capability();
        let phase = match self.text(machine.capability(), "machineState") {
            Some("run") => Some(Phase::Running),
            Some("pause") => Some(Phase::Paused),
            Some("stop") => Some(Phase::Off),
            _ => match self.text(samsung, "operatingState") {
                Some("running") => Some(Phase::Running),
                Some("paused") => Some(Phase::Paused),
                Some("ready") => Some(Phase::Off),
                _ => None,
            },
        };
        let finished = matches!(self.job(machine), Some("finish" | "finished"));
        (phase, finished)
    }

    fn job(&self, machine: Machine) -> Option<&str> {
        self.text(machine.capability(), machine.job_attribute())
            .or_else(|| self.text(machine.samsung_capability(), "jobState"))
            .filter(|job| *job != "none")
    }

    fn unit(&self, capability: &str, attribute: &str) -> Option<&str> {
        self.status
            .get(capability)?
            .get(attribute)?
            .get("unit")?
            .as_str()
    }
}

/// Correspondance appareils SmartThings ⇄ entités.
#[derive(Debug, Default)]
pub struct Devices {
    by_device: HashMap<String, Tracked>,
    by_entity: HashMap<EntityId, String>,
}

/// Les commandes à envoyer pour un appel de service.
#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    pub device_id: String,
    pub commands: Vec<Command>,
}

impl Devices {
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_device.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_device.is_empty()
    }

    /// Les identifiants des appareils suivis, pour relever leur état.
    #[must_use]
    pub fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.by_device.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// L'appareil d'une entité, s'il est à SmartThings.
    #[must_use]
    pub fn device_of(&self, entity_id: &EntityId) -> Option<&str> {
        self.by_entity.get(entity_id).map(String::as_str)
    }

    /// Les entités de chaque appareil (pour `smartthings devices`).
    #[must_use]
    pub fn summary(&self) -> Vec<(String, Vec<String>)> {
        let mut out: Vec<(String, Vec<String>)> = self
            .by_device
            .values()
            .map(|t| {
                let mut entities: Vec<String> = t.primary.iter().map(ToString::to_string).collect();
                entities.extend(t.sensors.iter().map(|(_, id)| id.to_string()));
                (t.label.clone(), entities)
            })
            .collect();
        out.sort();
        out
    }

    /// Met à jour la liste des appareils (sans leur état) : nouveaux,
    /// renommés, retirés. `rooms` : nom de chaque pièce.
    pub fn sync(
        &mut self,
        devices: Vec<Device>,
        rooms: &HashMap<String, String>,
        states: &StateMachine,
    ) {
        let mut present = Vec::new();
        let mut sorted = devices;
        sorted.sort_by(|a, b| a.label.cmp(&b.label).then(a.id.cmp(&b.id)));
        for device in sorted.into_iter().take(MAX_DEVICES) {
            let kind = Kind::of(&device);
            let sensors: Vec<Sensor> = Sensor::ALL
                .into_iter()
                .filter(|s| {
                    device.capabilities.iter().any(|c| {
                        c == s.capability().0
                            || (*s == Sensor::Energy && c == "powerConsumptionReport")
                    })
                })
                // La température d'un thermostat est dans le thermostat.
                .filter(|s| {
                    !(*s == Sensor::Temperature
                        && matches!(kind, Some(Kind::Thermostat | Kind::AirConditioner)))
                })
                .collect();
            if kind.is_none() && sensors.is_empty() {
                debug!("appareil SmartThings ignoré : {}", device.label);
                continue;
            }
            present.push(device.id.clone());
            let room = device
                .room_id
                .as_ref()
                .and_then(|id| rooms.get(id))
                .cloned();
            let previous = self.by_device.remove(&device.id);
            let keep = previous
                .as_ref()
                .filter(|p| p.kind == kind && p.capabilities == device.capabilities);
            let (primary, sensor_ids) = if let Some(previous) = keep {
                (previous.primary.clone(), previous.sensors.clone())
            } else {
                if let Some(previous) = &previous {
                    self.forget(previous, states);
                }
                self.allocate(&device.label, kind, &sensors, states)
            };
            if previous.is_none() {
                info!(
                    "appareil SmartThings trouvé : {} ({})",
                    device.label,
                    describe(kind, &sensor_ids)
                );
            }
            for entity in primary.iter().chain(sensor_ids.iter().map(|(_, id)| id)) {
                self.by_entity.insert(entity.clone(), device.id.clone());
            }
            let tracked = Tracked {
                label: device.label,
                room,
                kind,
                capabilities: device.capabilities,
                categories: device.categories,
                primary,
                sensors: sensor_ids,
                cycle: previous
                    .as_ref()
                    .map(|p| p.cycle.clone())
                    .unwrap_or_default(),
                status: previous.map_or(Value::Null, |p| p.status),
                available: true,
            };
            publish(&tracked, states);
            self.by_device.insert(device.id, tracked);
        }
        let gone: Vec<String> = self
            .by_device
            .keys()
            .filter(|id| !present.contains(id))
            .cloned()
            .collect();
        for id in gone {
            if let Some(tracked) = self.by_device.remove(&id) {
                info!("appareil SmartThings retiré : {}", tracked.label);
                self.forget(&tracked, states);
            }
        }
    }

    fn forget(&mut self, tracked: &Tracked, states: &StateMachine) {
        for entity in tracked
            .primary
            .iter()
            .chain(tracked.sensors.iter().map(|(_, id)| id))
        {
            self.by_entity.remove(entity);
            states.remove(entity);
        }
    }

    /// Choisit les identifiants : `light.lampe_salon`, `sensor.lampe_salon_batterie`.
    fn allocate(
        &self,
        label: &str,
        kind: Option<Kind>,
        sensors: &[Sensor],
        states: &StateMachine,
    ) -> (Option<EntityId>, Vec<(Sensor, EntityId)>) {
        let slug = Some(ha_core::slugify(label))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "smartthings".to_owned());
        let mut taken: Vec<EntityId> = Vec::new();
        let mut free = |domain: &str, name: &str| -> Option<EntityId> {
            let id = (1..=MAX_DEVICES).find_map(|n| {
                let candidate = if n == 1 {
                    format!("{domain}.{name}")
                } else {
                    format!("{domain}.{name}_{n}")
                };
                EntityId::new(&candidate)
                    .ok()
                    .filter(|id| !self.by_entity.contains_key(id) && !taken.contains(id))
                    // Une entité d'une autre intégration (MQTT, Hue…) garde son nom.
                    .filter(|id| {
                        states.get(id).is_none_or(|state| {
                            state.owner().is_none_or(|owner| owner == crate::ID)
                        })
                    })
            })?;
            taken.push(id.clone());
            Some(id)
        };
        let primary = kind.and_then(|kind| free(kind.domain(), &slug));
        let mut ids = Vec::new();
        for (index, sensor) in sensors.iter().enumerate() {
            let domain = if sensor.binary() {
                "binary_sensor"
            } else {
                "sensor"
            };
            // Un capteur seul garde le nom de l'appareil.
            let name = if primary.is_none() && index == 0 {
                slug.clone()
            } else {
                format!("{slug}_{}", sensor.suffix().0)
            };
            if let Some(id) = free(domain, &name) {
                ids.push((*sensor, id));
            }
        }
        (primary, ids)
    }

    /// Le nouvel état d'un appareil (`components/main/status`). Rend le
    /// cycle d'une machine qui vient de se terminer.
    pub fn apply_status(
        &mut self,
        device_id: &str,
        reading: Value,
        states: &StateMachine,
    ) -> Option<Finished> {
        let tracked = self.by_device.get_mut(device_id)?;
        tracked.status = reading;
        tracked.available = true;
        let finished = match tracked.kind {
            Some(Kind::Appliance(machine)) => {
                follow_cycle(tracked, machine, std::time::SystemTime::now())
            }
            _ => None,
        };
        publish(tracked, states);
        finished
    }

    /// L'appareil ne répond plus (retiré du compte, hors ligne).
    pub fn set_unavailable(&mut self, device_id: &str, states: &StateMachine) {
        if let Some(tracked) = self.by_device.get_mut(device_id)
            && tracked.available
        {
            tracked.available = false;
            publish(tracked, states);
        }
    }

    /// Traduit un appel de service en commandes SmartThings.
    #[must_use]
    pub fn command(
        &self,
        domain: &str,
        service: &str,
        entity_id: &EntityId,
        data: &Attributes,
    ) -> Option<Order> {
        let device_id = self.by_entity.get(entity_id)?;
        let tracked = self.by_device.get(device_id)?;
        if tracked.primary.as_ref() != Some(entity_id) {
            warn!("{entity_id} est une mesure : rien à commander");
            return None;
        }
        let kind = tracked.kind?;
        if kind.domain() != domain {
            warn!("service {domain}.{service} impossible sur {entity_id}");
            return None;
        }
        let commands = match kind {
            Kind::Light | Kind::Switch => switch_commands(tracked, kind, service, data, entity_id)?,
            Kind::Shade => shade_commands(tracked, service, data, entity_id)?,
            Kind::Garage => match service {
                "open_cover" => vec![cmd("doorControl", "open", vec![])],
                "close_cover" => vec![cmd("doorControl", "close", vec![])],
                _ => return unsupported(domain, service),
            },
            Kind::Lock => match service {
                "lock" => vec![cmd("lock", "lock", vec![])],
                "unlock" => vec![cmd("lock", "unlock", vec![])],
                _ => return unsupported(domain, service),
            },
            Kind::Thermostat => thermostat_commands(service, data, entity_id)?,
            Kind::AirConditioner => air_conditioner_commands(service, data, entity_id)?,
            Kind::Appliance(_) => return unsupported(domain, service),
        };
        Some(Order {
            device_id: device_id.clone(),
            commands,
        })
    }
}

/// Suit le cycle d'une machine : son début, sa fin (avec sa durée et son
/// énergie). Un état inconnu ne change rien.
fn follow_cycle(
    tracked: &mut Tracked,
    machine: Machine,
    now: std::time::SystemTime,
) -> Option<Finished> {
    let (said, job_finished) = tracked.machine(machine);
    let said = said?;
    let energy = tracked.energy_kwh();
    let cycle = &mut tracked.cycle;
    let before = cycle.phase;
    let running = |phase: Phase| matches!(phase, Phase::Running | Phase::Paused);
    match said {
        Phase::Running | Phase::Paused => {
            if !running(before) {
                cycle.started = Some(now);
                cycle.energy_at_start = energy;
            }
            cycle.phase = said;
            None
        }
        _ if running(before) => {
            cycle.phase = Phase::Finished;
            let minutes = cycle
                .started
                .and_then(|start| now.duration_since(start).ok())
                .map_or(0, |d| d.as_secs().div_ceil(60));
            let used = match (energy, cycle.energy_at_start) {
                (Some(end), Some(start)) if end >= start => {
                    Some(((end - start) * 1000.0).round() / 1000.0)
                }
                _ => None,
            };
            cycle.last = Some((minutes, used));
            cycle.started = None;
            Some(Finished {
                entity_id: tracked.primary.clone()?,
                name: tracked.label.clone(),
                minutes,
                energy_kwh: used,
            })
        }
        // Déjà fini : on le reste jusqu'au prochain cycle.
        _ if before == Phase::Finished => None,
        _ => {
            cycle.phase = if job_finished {
                Phase::Finished
            } else {
                Phase::Off
            };
            None
        }
    }
}

fn unsupported<T>(domain: &str, service: &str) -> Option<T> {
    warn!("service {domain}.{service} non pris en charge par SmartThings");
    None
}

fn cmd(capability: &'static str, command: &'static str, arguments: Vec<Value>) -> Command {
    Command {
        capability,
        command,
        arguments,
    }
}

fn describe(kind: Option<Kind>, sensors: &[(Sensor, EntityId)]) -> String {
    let mut parts: Vec<String> = kind.map(|k| k.domain().to_owned()).into_iter().collect();
    parts.extend(sensors.iter().map(|(s, _)| s.suffix().0.to_owned()));
    parts.join(", ")
}

fn switch_commands(
    tracked: &Tracked,
    kind: Kind,
    service: &str,
    data: &Attributes,
    entity_id: &EntityId,
) -> Option<Vec<Command>> {
    let on = match service {
        "turn_on" => true,
        "turn_off" => false,
        "toggle" => tracked.text("switch", "switch") != Some("on"),
        _ => return unsupported(kind.domain(), service),
    };
    if !on {
        return Some(vec![cmd("switch", "off", vec![])]);
    }
    let mut commands = Vec::new();
    if kind == Kind::Light {
        let level = data.get("brightness_pct").and_then(as_f64).or_else(|| {
            data.get("brightness")
                .and_then(as_f64)
                .map(|b| b / 255.0 * 100.0)
        });
        if let Some(level) = level {
            if !(0.0..=100.0).contains(&level) {
                warn!("luminosité invalide pour {entity_id}");
                return None;
            }
            if level < 0.5 {
                return Some(vec![cmd("switch", "off", vec![])]);
            }
            if tracked.has("switchLevel") {
                commands.push(cmd("switchLevel", "setLevel", vec![json!(level.round())]));
            }
        }
        if let Some(kelvin) = data.get("color_temp_kelvin").and_then(as_f64)
            && tracked.has("colorTemperature")
        {
            if !(1000.0..=12000.0).contains(&kelvin) {
                warn!("température de couleur invalide pour {entity_id}");
                return None;
            }
            commands.push(cmd(
                "colorTemperature",
                "setColorTemperature",
                vec![json!(kelvin.round())],
            ));
        }
    }
    commands.insert(0, cmd("switch", "on", vec![]));
    Some(commands)
}

fn shade_commands(
    tracked: &Tracked,
    service: &str,
    data: &Attributes,
    entity_id: &EntityId,
) -> Option<Vec<Command>> {
    Some(match service {
        "open_cover" => vec![cmd("windowShade", "open", vec![])],
        "close_cover" => vec![cmd("windowShade", "close", vec![])],
        "stop_cover" => vec![cmd("windowShade", "pause", vec![])],
        "set_cover_position" => {
            let Some(position) = data
                .get("position")
                .and_then(as_f64)
                .filter(|p| (0.0..=100.0).contains(p))
            else {
                warn!("position invalide pour {entity_id} (0 à 100)");
                return None;
            };
            let position = json!(position.round());
            if tracked.has("windowShadeLevel") {
                vec![cmd("windowShadeLevel", "setShadeLevel", vec![position])]
            } else if tracked.has("switchLevel") {
                vec![cmd("switchLevel", "setLevel", vec![position])]
            } else {
                warn!("{entity_id} ne sait pas aller à une position");
                return None;
            }
        }
        _ => return unsupported("cover", service),
    })
}

fn temperature(data: &Attributes, entity_id: &EntityId) -> Option<Value> {
    let Some(t) = data
        .get("temperature")
        .and_then(as_f64)
        .filter(|t| (5.0..=35.0).contains(t))
    else {
        warn!("consigne invalide pour {entity_id} (5 à 35 °C)");
        return None;
    };
    Some(json!(t))
}

fn thermostat_commands(
    service: &str,
    data: &Attributes,
    entity_id: &EntityId,
) -> Option<Vec<Command>> {
    Some(match service {
        "set_temperature" => vec![cmd(
            "thermostatHeatingSetpoint",
            "setHeatingSetpoint",
            vec![temperature(data, entity_id)?],
        )],
        "set_hvac_mode" => {
            let mode = match data.get("hvac_mode").and_then(Value::as_str) {
                Some("heat") => "heat",
                Some("off") => "off",
                Some("auto") => "auto",
                Some("cool") => "cool",
                _ => {
                    warn!("mode invalide pour {entity_id}");
                    return None;
                }
            };
            vec![cmd(
                "thermostatMode",
                "setThermostatMode",
                vec![json!(mode)],
            )]
        }
        "turn_off" => vec![cmd(
            "thermostatMode",
            "setThermostatMode",
            vec![json!("off")],
        )],
        "turn_on" => vec![cmd(
            "thermostatMode",
            "setThermostatMode",
            vec![json!("heat")],
        )],
        _ => return unsupported("climate", service),
    })
}

fn air_conditioner_commands(
    service: &str,
    data: &Attributes,
    entity_id: &EntityId,
) -> Option<Vec<Command>> {
    Some(match service {
        "set_temperature" => vec![cmd(
            "thermostatCoolingSetpoint",
            "setCoolingSetpoint",
            vec![temperature(data, entity_id)?],
        )],
        "set_hvac_mode" => {
            let mode = match data.get("hvac_mode").and_then(Value::as_str) {
                Some("off") => return Some(vec![cmd("switch", "off", vec![])]),
                Some("cool") => "cool",
                Some("heat") => "heat",
                Some("dry") => "dry",
                Some("fan_only") => "wind",
                Some("auto") => "auto",
                _ => {
                    warn!("mode invalide pour {entity_id}");
                    return None;
                }
            };
            vec![
                cmd("switch", "on", vec![]),
                cmd(
                    "airConditionerMode",
                    "setAirConditionerMode",
                    vec![json!(mode)],
                ),
            ]
        }
        "turn_off" => vec![cmd("switch", "off", vec![])],
        "turn_on" => vec![cmd("switch", "on", vec![])],
        _ => return unsupported("climate", service),
    })
}

/// Calcule et publie l'état de chaque entité de l'appareil.
fn publish(tracked: &Tracked, states: &StateMachine) {
    let loaded = !tracked.status.is_null();
    if let (Some(kind), Some(entity_id)) = (tracked.kind, &tracked.primary) {
        let mut attributes = base_attributes(tracked, &tracked.label);
        let state = if !tracked.available {
            "unavailable".to_owned()
        } else if loaded {
            primary_state(tracked, kind, &mut attributes)
        } else {
            "unknown".to_owned()
        };
        set(states, entity_id, state, attributes);
    }
    for (sensor, entity_id) in &tracked.sensors {
        let name = if tracked.primary.is_none()
            && tracked.sensors.first().map(|(_, id)| id) == Some(entity_id)
        {
            tracked.label.clone()
        } else {
            format!("{} {}", tracked.label, sensor.suffix().1)
        };
        let mut attributes = base_attributes(tracked, &name);
        let state = if !tracked.available {
            "unavailable".to_owned()
        } else if loaded {
            sensor_state(tracked, *sensor, &mut attributes)
        } else {
            "unknown".to_owned()
        };
        set(states, entity_id, state, attributes);
    }
}

fn base_attributes(tracked: &Tracked, name: &str) -> Attributes {
    let mut attributes = Attributes::new();
    attributes.insert("friendly_name".into(), json!(name));
    if let Some(room) = &tracked.room {
        attributes.insert("piece".into(), json!(room));
    }
    attributes
}

fn set(states: &StateMachine, entity_id: &EntityId, state: String, attributes: Attributes) {
    if let Err(error) = states.set_by(crate::ID, entity_id.clone(), state, attributes) {
        warn!("état refusé pour {entity_id} : {error}");
    }
}

fn primary_state(tracked: &Tracked, kind: Kind, attributes: &mut Attributes) -> String {
    let unknown = || "unknown".to_owned();
    match kind {
        Kind::Light | Kind::Switch => {
            if kind == Kind::Switch {
                if tracked.categories.iter().any(|c| c == "Television") {
                    attributes.insert("device_class".into(), json!("tv"));
                } else if tracked.categories.iter().any(|c| c == "SmartPlug") {
                    attributes.insert("device_class".into(), json!("outlet"));
                }
            }
            let on = tracked.text("switch", "switch") == Some("on");
            if kind == Kind::Light {
                let mut modes = vec!["onoff"];
                if tracked.has("switchLevel") {
                    modes = vec!["brightness"];
                    if on && let Some(level) = tracked.number("switchLevel", "level") {
                        attributes.insert(
                            "brightness".into(),
                            json!((level.clamp(0.0, 100.0) / 100.0 * 255.0).round()),
                        );
                    }
                }
                if tracked.has("colorTemperature") {
                    modes = vec!["color_temp"];
                    if let Some(k) = tracked.number("colorTemperature", "colorTemperature") {
                        attributes.insert("color_temp_kelvin".into(), json!(k.round()));
                    }
                }
                attributes.insert("supported_color_modes".into(), json!(modes));
            }
            match tracked.text("switch", "switch") {
                Some("on") => "on".into(),
                Some("off") => "off".into(),
                _ => unknown(),
            }
        }
        Kind::Shade => {
            attributes.insert(
                "device_class".into(),
                json!(if tracked.categories.iter().any(|c| c == "Blind") {
                    "blind"
                } else {
                    "shutter"
                }),
            );
            let level = tracked
                .number("windowShadeLevel", "shadeLevel")
                .or_else(|| tracked.number("switchLevel", "level"));
            if let Some(level) = level {
                attributes.insert(
                    "current_position".into(),
                    json!(level.clamp(0.0, 100.0).round()),
                );
            }
            match tracked.text("windowShade", "windowShade") {
                Some("open" | "partially open") => "open".into(),
                Some("closed") => "closed".into(),
                Some("opening") => "opening".into(),
                Some("closing") => "closing".into(),
                _ => level.map_or_else(unknown, |l| if l > 0.0 { "open" } else { "closed" }.into()),
            }
        }
        Kind::Garage => {
            attributes.insert("device_class".into(), json!("garage"));
            match tracked.text("doorControl", "door") {
                Some(state @ ("open" | "closed" | "opening" | "closing")) => state.into(),
                _ => unknown(),
            }
        }
        Kind::Lock => match tracked.text("lock", "lock") {
            Some("locked") => "locked".into(),
            Some("unlocked" | "unlocked with timeout") => "unlocked".into(),
            _ => unknown(),
        },
        Kind::Thermostat => {
            let modes: Vec<String> = tracked
                .value("thermostatMode", "supportedThermostatModes")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(Value::as_str)
                        .filter(|m| matches!(*m, "heat" | "off" | "auto" | "cool"))
                        .map(str::to_owned)
                        .collect()
                })
                .filter(|list: &Vec<String>| !list.is_empty())
                .unwrap_or_else(|| vec!["heat".into()]);
            climate_attributes(
                tracked,
                attributes,
                "thermostatHeatingSetpoint",
                "heatingSetpoint",
                &modes,
            );
            match tracked.text("thermostatMode", "thermostatMode") {
                Some(mode @ ("heat" | "off" | "auto" | "cool")) => mode.into(),
                Some("eco" | "emergency heat") | None => "heat".into(),
                Some(_) => "auto".into(),
            }
        }
        Kind::AirConditioner => {
            let modes: Vec<String> = ["off", "cool", "heat", "dry", "fan_only", "auto"]
                .into_iter()
                .map(str::to_owned)
                .collect();
            climate_attributes(
                tracked,
                attributes,
                "thermostatCoolingSetpoint",
                "coolingSetpoint",
                &modes,
            );
            if tracked.text("switch", "switch") == Some("off") {
                return "off".into();
            }
            match tracked.text("airConditionerMode", "airConditionerMode") {
                Some("cool") => "cool".into(),
                Some("heat") => "heat".into(),
                Some("dry") => "dry".into(),
                Some("wind" | "fan") => "fan_only".into(),
                Some(_) => "auto".into(),
                None => unknown(),
            }
        }
        Kind::Appliance(machine) => {
            appliance_attributes(tracked, machine, attributes, std::time::SystemTime::now());
            tracked.cycle.phase.name().into()
        }
    }
}

/// Ce qu'on montre d'une machine : son programme, la fin prévue, le
/// dernier cycle.
fn appliance_attributes(
    tracked: &Tracked,
    machine: Machine,
    attributes: &mut Attributes,
    now: std::time::SystemTime,
) {
    attributes.insert("appareil".into(), json!(machine.name()));
    attributes.insert("device_class".into(), json!("enum"));
    attributes.insert(
        "options".into(),
        json!([
            Phase::Off.name(),
            Phase::Running.name(),
            Phase::Paused.name(),
            Phase::Finished.name()
        ]),
    );
    if let Some(job) = tracked.job(machine) {
        attributes.insert("cycle".into(), json!(job_name(job)));
    }
    if let Some((minutes, kwh)) = tracked.cycle.last {
        attributes.insert("dernier_cycle_minutes".into(), json!(minutes));
        if let Some(kwh) = kwh {
            attributes.insert("dernier_cycle_kwh".into(), json!(kwh));
        }
    }
    if !matches!(tracked.cycle.phase, Phase::Running | Phase::Paused) {
        return;
    }
    let samsung = machine.samsung_capability();
    if let Some(progress) = tracked
        .number(samsung, "progress")
        .filter(|p| (0.0..=100.0).contains(p))
    {
        attributes.insert("progression".into(), json!(progress.round()));
    }
    // La fin : l'heure annoncée, sinon le temps restant.
    let end = tracked
        .text(machine.capability(), "completionTime")
        .and_then(|text| text.parse::<jiff::Timestamp>().ok())
        .or_else(|| {
            let left = tracked.number(samsung, "remainingTime")?;
            let now = jiff::Timestamp::try_from(now).ok()?;
            #[allow(
                clippy::cast_possible_truncation,
                reason = "minutes bornées juste avant"
            )]
            let left = left.clamp(0.0, 24.0 * 60.0) as i64;
            now.checked_add(jiff::SignedDuration::from_mins(left)).ok()
        });
    let Some(end) = end else { return };
    let Ok(now) = jiff::Timestamp::try_from(now) else {
        return;
    };
    let left = end.duration_since(now).as_secs().max(0);
    #[allow(clippy::cast_sign_loss, reason = "positif juste avant")]
    let left_minutes = (left as u64).div_ceil(60);
    attributes.insert("reste_minutes".into(), json!(left_minutes));
    attributes.insert("fin".into(), json!(end.to_string()));
    attributes.insert(
        "fin_heure".into(),
        json!(
            end.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%H:%M")
                .to_string()
        ),
    );
}

/// Le programme en cours, en français.
fn job_name(job: &str) -> String {
    match job {
        "wash" | "ai_wash" | "air_wash" | "airwash" => "lavage",
        "pre_wash" => "prélavage",
        "rinse" | "ai_rinse" => "rinçage",
        "spin" | "ai_spin" => "essorage",
        "drying" | "ai_drying" => "séchage",
        "cooling" => "refroidissement",
        "weight_sensing" => "pesée",
        "delay_wash" => "départ différé",
        "wrinkle_prevent" => "anti-froissage",
        "refreshing" => "rafraîchissement",
        "sanitizing" => "désinfection",
        "pre_drain" => "vidange",
        "finish" | "finished" => "fini",
        other => return other.chars().take(32).collect(),
    }
    .to_owned()
}

fn climate_attributes(
    tracked: &Tracked,
    attributes: &mut Attributes,
    capability: &str,
    attribute: &str,
    modes: &[String],
) {
    if let Some(target) = tracked.number(capability, attribute) {
        attributes.insert("temperature".into(), json!(target));
    }
    if let Some(current) = tracked.number("temperatureMeasurement", "temperature") {
        attributes.insert("current_temperature".into(), json!(current));
    }
    attributes.insert("hvac_modes".into(), json!(modes));
    attributes.insert("min_temp".into(), json!(5));
    attributes.insert("max_temp".into(), json!(30));
    attributes.insert("unit_of_measurement".into(), json!("°C"));
}

fn sensor_state(tracked: &Tracked, sensor: Sensor, attributes: &mut Attributes) -> String {
    let (capability, attribute) = sensor.capability();
    let class = match sensor {
        Sensor::Contact => {
            if tracked.categories.iter().any(|c| c == "Window") {
                "window"
            } else {
                "door"
            }
        }
        Sensor::Motion => "motion",
        Sensor::Water => "moisture",
        Sensor::Smoke => "smoke",
        Sensor::Presence => "presence",
        Sensor::Temperature => "temperature",
        Sensor::Humidity => "humidity",
        Sensor::Illuminance => "illuminance",
        Sensor::Battery => "battery",
        Sensor::Power => "power",
        Sensor::Energy => "energy",
    };
    attributes.insert("device_class".into(), json!(class));
    if sensor.binary() {
        let on = match sensor {
            Sensor::Contact => "open",
            Sensor::Motion => "active",
            Sensor::Water => "wet",
            Sensor::Smoke => "detected",
            _ => "present",
        };
        return match tracked.text(capability, attribute) {
            Some(value) if value == on => "on".into(),
            Some(_) => "off".into(),
            None => "unknown".into(),
        };
    }
    if sensor == Sensor::Energy {
        attributes.insert("state_class".into(), json!("total_increasing"));
        attributes.insert("unit_of_measurement".into(), json!("kWh"));
        return tracked.energy_kwh().map_or_else(
            || "unknown".into(),
            |kwh| format!("{}", (kwh * 1000.0).round() / 1000.0),
        );
    }
    let Some(mut value) = tracked.number(capability, attribute) else {
        return "unknown".into();
    };
    let unit = match sensor {
        Sensor::Temperature => {
            if tracked.unit(capability, attribute) == Some("F") {
                value = (value - 32.0) * 5.0 / 9.0;
            }
            "°C"
        }
        Sensor::Humidity | Sensor::Battery => "%",
        Sensor::Illuminance => "lx",
        Sensor::Power => "W",
        _ => "",
    };
    attributes.insert("unit_of_measurement".into(), json!(unit));
    format!("{}", (value * 100.0).round() / 100.0)
}

fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|v: &f64| v.is_finite())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;
    use ha_core::Core;

    fn device(id: &str, label: &str, capabilities: &[&str], categories: &[&str]) -> Device {
        Device {
            id: id.into(),
            label: label.into(),
            location_id: None,
            room_id: Some("aa".into()),
            capabilities: capabilities.iter().map(|c| (*c).to_owned()).collect(),
            categories: categories.iter().map(|c| (*c).to_owned()).collect(),
        }
    }

    fn house() -> Vec<Device> {
        vec![
            device(
                "01",
                "Lampe salon",
                &["switch", "switchLevel", "colorTemperature"],
                &["Light"],
            ),
            device(
                "02",
                "Prise café",
                &["switch", "powerMeter", "energyMeter"],
                &["SmartPlug"],
            ),
            device(
                "03",
                "Volet chambre",
                &["windowShade", "windowShadeLevel", "battery"],
                &["Blind"],
            ),
            device(
                "04",
                "Porte entrée",
                &["contactSensor", "temperatureMeasurement", "battery"],
                &["MultiFunctionalSensor"],
            ),
            device(
                "05",
                "Clim salon",
                &[
                    "switch",
                    "airConditionerMode",
                    "thermostatCoolingSetpoint",
                    "temperatureMeasurement",
                ],
                &["AirConditioner"],
            ),
            device(
                "06",
                "Lave-linge",
                &["switch", "washerOperatingState"],
                &["Washer"],
            ),
            device("07", "Garage", &["doorControl"], &["GarageDoor"]),
            device("08", "Télé", &["switch", "audioVolume"], &["Television"]),
            device("09", "Hub", &["bridge"], &["Hub"]),
        ]
    }

    fn rooms() -> HashMap<String, String> {
        [("aa".to_owned(), "Salon".to_owned())].into()
    }

    fn id(text: &str) -> EntityId {
        text.parse().unwrap()
    }

    #[test]
    fn devices_become_entities() {
        let core = Core::new();
        let mut devices = Devices::default();
        devices.sync(house(), &rooms(), &core.states);
        assert_eq!(devices.len(), 8, "le hub n'a rien à montrer");
        let mut all: Vec<String> = devices
            .summary()
            .into_iter()
            .flat_map(|(_, entities)| entities)
            .collect();
        all.sort();
        assert_eq!(
            all,
            [
                "binary_sensor.porte_entree",
                "climate.clim_salon",
                "cover.garage",
                "cover.volet_chambre",
                "light.lampe_salon",
                "sensor.lave_linge",
                "sensor.porte_entree_batterie",
                "sensor.porte_entree_temperature",
                "sensor.prise_cafe_energie",
                "sensor.prise_cafe_puissance",
                "sensor.volet_chambre_batterie",
                "switch.prise_cafe",
                "switch.tele",
            ]
        );
        // Avant le premier relevé : inconnu, avec le nom et la pièce.
        let lamp = core.states.get(&id("light.lampe_salon")).unwrap();
        assert_eq!(lamp.state(), "unknown");
        assert_eq!(lamp.attributes()["piece"], "Salon");
        assert_eq!(lamp.owner(), Some(crate::ID));
    }

    #[test]
    fn status_becomes_states() {
        let core = Core::new();
        let mut devices = Devices::default();
        devices.sync(house(), &rooms(), &core.states);
        let state = |entity: &str| core.states.get(&id(entity)).unwrap();
        devices.apply_status(
            "01",
            json!({"switch": {"switch": {"value": "on"}}, "switchLevel": {"level": {"value": 50, "unit": "%"}},
                   "colorTemperature": {"colorTemperature": {"value": 2700}}}),
            &core.states,
        );
        assert_eq!(state("light.lampe_salon").state(), "on");
        assert_eq!(state("light.lampe_salon").attributes()["brightness"], 128.0);
        assert_eq!(
            state("light.lampe_salon").attributes()["color_temp_kelvin"],
            2700.0
        );

        devices.apply_status(
            "03",
            json!({"windowShade": {"windowShade": {"value": "partially open"}},
                   "windowShadeLevel": {"shadeLevel": {"value": 40}}, "battery": {"battery": {"value": 87}}}),
            &core.states,
        );
        assert_eq!(state("cover.volet_chambre").state(), "open");
        assert_eq!(
            state("cover.volet_chambre").attributes()["current_position"],
            40.0
        );
        assert_eq!(
            state("cover.volet_chambre").attributes()["device_class"],
            "blind"
        );
        assert_eq!(state("sensor.volet_chambre_batterie").state(), "87");

        devices.apply_status(
            "04",
            json!({"contactSensor": {"contact": {"value": "open"}},
                   "temperatureMeasurement": {"temperature": {"value": 68, "unit": "F"}}}),
            &core.states,
        );
        assert_eq!(state("binary_sensor.porte_entree").state(), "on");
        assert_eq!(
            state("binary_sensor.porte_entree").attributes()["friendly_name"],
            "Porte entrée"
        );
        assert_eq!(state("sensor.porte_entree_temperature").state(), "20");
        assert_eq!(
            state("sensor.porte_entree_temperature").attributes()["friendly_name"],
            "Porte entrée Température"
        );

        devices.apply_status(
            "05",
            json!({"switch": {"switch": {"value": "on"}}, "airConditionerMode": {"airConditionerMode": {"value": "wind"}},
                   "thermostatCoolingSetpoint": {"coolingSetpoint": {"value": 24}},
                   "temperatureMeasurement": {"temperature": {"value": 27.5}}}),
            &core.states,
        );
        let clim = state("climate.clim_salon");
        assert_eq!(clim.state(), "fan_only");
        assert_eq!(clim.attributes()["temperature"], 24.0);
        assert_eq!(clim.attributes()["current_temperature"], 27.5);

        devices.apply_status(
            "06",
            json!({"washerOperatingState": {"machineState": {"value": "run"}, "washerJobState": {"value": "rinse"},
                   "completionTime": {"value": "2026-10-22T10:30:00Z"}}}),
            &core.states,
        );
        assert_eq!(state("sensor.lave_linge").state(), "En marche");
        assert_eq!(state("sensor.lave_linge").attributes()["cycle"], "rinçage");
        assert_eq!(
            state("sensor.lave_linge").attributes()["appareil"],
            "lave-linge"
        );

        devices.apply_status(
            "02",
            json!({"energyMeter": {"energy": {"value": 12500, "unit": "Wh"}}}),
            &core.states,
        );
        assert_eq!(state("sensor.prise_cafe_energie").state(), "12.5");
        assert_eq!(
            state("switch.prise_cafe").attributes()["device_class"],
            "outlet"
        );

        devices.set_unavailable("01", &core.states);
        assert_eq!(state("light.lampe_salon").state(), "unavailable");
    }

    /// Un lave-linge Samsung : le cycle suivi, puis l'évènement de fin avec
    /// sa durée et son énergie.
    #[test]
    fn a_samsung_washer_cycle() {
        use std::time::{Duration, SystemTime};

        let core = Core::new();
        let mut devices = Devices::default();
        let washer = device(
            "w1",
            "Lave-linge",
            &[
                "switch",
                "samsungce.washerOperatingState",
                "powerConsumptionReport",
                "remoteControlStatus",
            ],
            &["Washer"],
        );
        devices.sync(vec![washer], &rooms(), &core.states);
        assert_eq!(
            devices.summary()[0].1,
            ["sensor.lave_linge", "sensor.lave_linge_energie"]
        );
        let state = |entity: &str| core.states.get(&id(entity)).unwrap();
        let reading = |operating: &str, job: &str, remaining: u32, wh: u32| {
            json!({
                "samsungce.washerOperatingState": {
                    "operatingState": {"value": operating}, "jobState": {"value": job},
                    "remainingTime": {"value": remaining, "unit": "min"}, "progress": {"value": 40}},
                "powerConsumptionReport": {"powerConsumption": {"value": {"energy": wh, "deltaEnergy": 0}}},
            })
        };
        assert!(
            devices
                .apply_status("w1", reading("ready", "none", 0, 100_000), &core.states)
                .is_none()
        );
        assert_eq!(state("sensor.lave_linge").state(), "Arrêt");
        assert!(
            devices
                .apply_status("w1", reading("running", "wash", 95, 100_000), &core.states)
                .is_none()
        );
        let running = state("sensor.lave_linge");
        assert_eq!(running.state(), "En marche");
        assert_eq!(running.attributes()["cycle"], "lavage");
        assert_eq!(running.attributes()["reste_minutes"], 95);
        assert_eq!(running.attributes()["progression"], 40.0);
        assert!(
            running.attributes()["fin_heure"]
                .as_str()
                .unwrap()
                .contains(':')
        );
        assert_eq!(state("sensor.lave_linge_energie").state(), "100");
        // Une pause ne termine pas le cycle.
        assert!(
            devices
                .apply_status("w1", reading("paused", "rinse", 30, 100_400), &core.states)
                .is_none()
        );
        assert_eq!(state("sensor.lave_linge").state(), "En pause");
        // Le cycle commencé il y a 90 minutes se termine.
        devices.by_device.get_mut("w1").unwrap().cycle.started =
            Some(SystemTime::now() - Duration::from_mins(90));
        let done = devices
            .apply_status("w1", reading("ready", "finish", 0, 100_850), &core.states)
            .unwrap();
        assert_eq!(done.entity_id.as_str(), "sensor.lave_linge");
        assert_eq!(done.name, "Lave-linge");
        assert_eq!(done.minutes, 90);
        assert_eq!(done.energy_kwh, Some(0.85));
        let finished = state("sensor.lave_linge");
        assert_eq!(finished.state(), "Terminé");
        assert_eq!(finished.attributes()["dernier_cycle_minutes"], 90);
        assert_eq!(finished.attributes()["dernier_cycle_kwh"], 0.85);
        assert!(finished.attributes().get("reste_minutes").is_none());
        // Relu à l'arrêt : toujours « Terminé », pas de deuxième évènement.
        assert!(
            devices
                .apply_status("w1", reading("ready", "none", 0, 100_850), &core.states)
                .is_none()
        );
        assert_eq!(state("sensor.lave_linge").state(), "Terminé");
        // Le cycle suivant repart.
        assert!(
            devices
                .apply_status("w1", reading("running", "wash", 60, 100_850), &core.states)
                .is_none()
        );
        assert_eq!(state("sensor.lave_linge").state(), "En marche");
    }

    #[test]
    fn services_become_commands() {
        let core = Core::new();
        let mut devices = Devices::default();
        devices.sync(house(), &rooms(), &core.states);
        let order = |domain: &str, service: &str, entity: &str, data: Value| {
            let Value::Object(data) = data else { panic!() };
            devices.command(domain, service, &id(entity), &data)
        };
        let names = |order: Option<Order>| -> Vec<(String, String, Vec<Value>)> {
            order
                .unwrap()
                .commands
                .into_iter()
                .map(|c| (c.capability.to_owned(), c.command.to_owned(), c.arguments))
                .collect()
        };
        assert_eq!(
            names(order(
                "light",
                "turn_on",
                "light.lampe_salon",
                json!({"brightness_pct": 40})
            )),
            [
                ("switch".to_owned(), "on".to_owned(), vec![]),
                (
                    "switchLevel".to_owned(),
                    "setLevel".to_owned(),
                    vec![json!(40.0)]
                )
            ]
        );
        assert_eq!(
            names(order(
                "light",
                "turn_on",
                "light.lampe_salon",
                json!({"brightness": 0})
            )),
            [("switch".to_owned(), "off".to_owned(), vec![])]
        );
        assert_eq!(
            names(order(
                "cover",
                "set_cover_position",
                "cover.volet_chambre",
                json!({"position": 30})
            )),
            [(
                "windowShadeLevel".to_owned(),
                "setShadeLevel".to_owned(),
                vec![json!(30.0)]
            )]
        );
        assert_eq!(
            names(order(
                "climate",
                "set_hvac_mode",
                "climate.clim_salon",
                json!({"hvac_mode": "cool"})
            )),
            [
                ("switch".to_owned(), "on".to_owned(), vec![]),
                (
                    "airConditionerMode".to_owned(),
                    "setAirConditionerMode".to_owned(),
                    vec![json!("cool")]
                )
            ]
        );
        assert_eq!(
            names(order(
                "climate",
                "set_temperature",
                "climate.clim_salon",
                json!({"temperature": 23})
            )),
            [(
                "thermostatCoolingSetpoint".to_owned(),
                "setCoolingSetpoint".to_owned(),
                vec![json!(23.0)]
            )]
        );
        assert_eq!(
            order("cover", "open_cover", "cover.garage", json!({}))
                .unwrap()
                .device_id,
            "07"
        );
        // Ce qui ne se commande pas.
        assert!(order("sensor", "turn_on", "sensor.lave_linge", json!({})).is_none());
        assert!(
            order(
                "switch",
                "turn_on",
                "sensor.prise_cafe_puissance",
                json!({})
            )
            .is_none()
        );
        assert!(
            order(
                "light",
                "turn_on",
                "light.lampe_salon",
                json!({"brightness_pct": 400})
            )
            .is_none()
        );
        assert!(
            order(
                "climate",
                "set_temperature",
                "climate.clim_salon",
                json!({"temperature": 80})
            )
            .is_none()
        );
        assert!(order("light", "turn_on", "light.ailleurs", json!({})).is_none());
    }

    #[test]
    fn names_of_other_integrations_are_kept() {
        let core = Core::new();
        core.states
            .set_by("mqtt", id("light.lampe_salon"), "on", Attributes::new())
            .unwrap();
        let mut devices = Devices::default();
        devices.sync(house(), &rooms(), &core.states);
        let entities: Vec<String> = devices
            .summary()
            .into_iter()
            .find(|(label, _)| label == "Lampe salon")
            .unwrap()
            .1;
        assert_eq!(entities, ["light.lampe_salon_2"]);
        assert_eq!(
            core.states.get(&id("light.lampe_salon")).unwrap().owner(),
            Some("mqtt")
        );
    }

    #[test]
    fn renamed_kept_and_removed_devices() {
        let core = Core::new();
        let mut devices = Devices::default();
        devices.sync(house(), &rooms(), &core.states);
        // Renommé : l'identifiant reste (les automatisations continuent).
        let mut renamed = house();
        renamed[0].label = "Plafonnier".into();
        renamed.truncate(3);
        devices.sync(renamed, &rooms(), &core.states);
        assert_eq!(devices.len(), 3);
        assert_eq!(
            core.states
                .get(&id("light.lampe_salon"))
                .unwrap()
                .attributes()["friendly_name"],
            "Plafonnier"
        );
        // Retirés : leurs entités disparaissent.
        assert!(core.states.get(&id("climate.clim_salon")).is_none());
        assert!(
            core.states
                .get(&id("sensor.porte_entree_batterie"))
                .is_none()
        );
    }
}
