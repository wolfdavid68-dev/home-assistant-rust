//! **Délestage** (étape 62) : quand la maison tire plus que l'abonnement ne
//! le permet, le serveur coupe d'abord les appareils les moins utiles (le
//! chauffe-eau, un radiateur d'appoint…), un par un, **avant** que le
//! disjoncteur du compteur ne saute. Il les rallume ensuite, dans l'ordre
//! inverse, quand la consommation est redescendue depuis un moment.
//!
//! ```yaml
//! # delestage.yaml
//! mesure: sensor.linky_intensite   # par défaut ; ou sensor.linky_puissance (VA)
//! limite: 30                       # facultatif : sinon, la puissance souscrite du Linky
//! appareils:                       # coupés dans cet ordre, rallumés dans l'ordre inverse
//!   - switch.chauffe_eau
//!   - climate.bureau
//!   - switch.radiateur_salle_de_bain
//! retour_apres: 5                  # minutes sous la limite avant de rallumer (1 à 60)
//! ```
//!
//! On l'**arme** avec `switch.delestage`. Le désarmer rallume tout de suite ce
//! qu'il avait coupé. Chaque coupure et chaque remise en marche est annoncée
//! sur le bus (événement `delestage`) et dite dans le journal de bord (« par
//! l'automatisation « Délestage » »).

use std::sync::Arc;
use std::time::Duration;

use ha_core::{
    Attributes, Background, CancellationToken, Core, EntityId, Event, Origin, StateMachine,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use tokio::time::Instant;
use tracing::{info, warn};

/// L'interrupteur qui arme le délestage.
pub const SWITCH: &str = "switch.delestage";
/// Le nom donné aux ordres dans le journal de bord.
pub const NAME: &str = "Délestage";
/// L'événement publié à chaque coupure ou remise en marche.
pub const EVENT: &str = "delestage";
/// Appareils au plus.
pub const MAX_DEVICES: usize = 20;
/// La mesure par défaut : l'intensité lue sur le Linky (étape 32).
pub const DEFAULT_MEASURE: &str = "sensor.linky_intensite";
/// Le capteur du Linky qui porte la puissance souscrite (« 30 A », « 6 kVA »).
const LINKY: &str = "sensor.linky_energie";
const DOMAINS: [&str; 5] = ["switch", "light", "fan", "input_boolean", "climate"];

/// Entre deux coupures : le Linky publie la puissance toutes les 10 s, la
/// coupure précédente doit avoir eu le temps de se voir.
pub const CUT_EVERY: Duration = Duration::from_secs(15);
/// On ne rallume que sous 85 % de la limite : rallumer à 99 % ferait
/// recouper aussitôt.
pub const RESTORE_BELOW: f64 = 0.85;
/// Entre deux remises en marche.
pub const RESTORE_EVERY: Duration = Duration::from_mins(1);
/// Le rythme de la vérification.
const TICK: Duration = Duration::from_secs(2);

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("lecture de {path} impossible : {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("{0}")]
    Invalid(String),
}

/// Les réglages du délestage.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Ce qui est mesuré : une intensité (A) ou une puissance (VA, W).
    pub measure: EntityId,
    /// La limite, dans l'unité de la mesure ; `None` : la puissance souscrite
    /// du Linky.
    pub limit: Option<f64>,
    /// Coupés dans cet ordre.
    pub devices: Vec<EntityId>,
    /// Le temps passé sous la limite avant de rallumer.
    pub restore_after: Duration,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    mesure: Option<String>,
    #[serde(default)]
    limite: Option<f64>,
    appareils: Vec<String>,
    #[serde(default = "five")]
    retour_apres: u8,
}

fn five() -> u8 {
    5
}

/// Vérifie le contenu de `delestage.yaml`.
pub fn parse_config(text: &str) -> Result<Config, ConfigError> {
    let file: File =
        serde_saphyr::from_str(text).map_err(|error| ConfigError::Invalid(error.to_string()))?;
    let raw = file.mesure.as_deref().unwrap_or(DEFAULT_MEASURE);
    let measure: EntityId = raw
        .parse()
        .map_err(|_| ConfigError::Invalid(format!("mesure « {raw} » invalide")))?;
    if measure.domain() != "sensor" {
        return Err(ConfigError::Invalid(format!(
            "mesure : un capteur (sensor.…) attendu, pas {measure}"
        )));
    }
    if let Some(limit) = file.limite
        && !(limit.is_finite() && limit > 0.0 && limit <= 100_000.0)
    {
        return Err(ConfigError::Invalid(
            "limite : un nombre plus grand que 0 (en A, ou en VA)".into(),
        ));
    }
    if !(1..=60).contains(&file.retour_apres) {
        return Err(ConfigError::Invalid(
            "retour_apres : de 1 à 60 minutes".into(),
        ));
    }
    if file.appareils.is_empty() || file.appareils.len() > MAX_DEVICES {
        return Err(ConfigError::Invalid(format!(
            "appareils : de 1 à {MAX_DEVICES}"
        )));
    }
    let mut devices: Vec<EntityId> = Vec::new();
    for raw in &file.appareils {
        let entity_id: EntityId = raw
            .parse()
            .map_err(|_| ConfigError::Invalid(format!("appareil « {raw} » invalide")))?;
        if !DOMAINS.contains(&entity_id.domain()) {
            return Err(ConfigError::Invalid(format!(
                "{entity_id} : seulement des prises, lumières, ventilateurs, interrupteurs virtuels \
                 et thermostats"
            )));
        }
        if !devices.contains(&entity_id) {
            devices.push(entity_id);
        }
    }
    Ok(Config {
        measure,
        limit: file.limite,
        devices,
        restore_after: Duration::from_mins(u64::from(file.retour_apres)),
    })
}

/// Lit `delestage.yaml` ; `None` si le fichier est absent.
pub fn load(path: &std::path::Path) -> Result<Option<Config>, ConfigError> {
    match ha_core::config_file::read(path, ha_core::config_file::DEFAULT_MAX) {
        Ok(Some(text)) => parse_config(&text).map(Some),
        Ok(None) => Ok(None),
        Err(error) => Err(ConfigError::Read {
            path: path.display().to_string(),
            // Concept Rust : `.into()` range « fichier trop gros » dans une
            // erreur d'entrée-sortie, avec son message.
            source: error.into(),
        }),
    }
}

/// La limite tirée de la puissance souscrite du Linky : « 30 A » pour une
/// intensité en A, « 6 kVA » pour une puissance en VA (ou en W, à peu
/// près). `None` si les unités ne vont pas ensemble.
#[must_use]
pub fn subscribed_limit(subscribed: &str, unit: &str) -> Option<f64> {
    let (number, suffix) = subscribed.trim().split_once(' ')?;
    let number: f64 = number
        .parse()
        .ok()
        .filter(|n: &f64| n.is_finite() && *n > 0.0)?;
    match (suffix.trim(), unit.trim()) {
        ("A", "A") | ("kVA", "kVA" | "kW") => Some(number),
        ("kVA", "VA" | "W") => Some(number * 1000.0),
        _ => None,
    }
}

/// Un appareil coupé, et ce qu'il faisait avant (« on », ou le mode du
/// thermostat : « heat »…).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cut {
    pub entity_id: String,
    #[serde(rename = "avant")]
    pub before: String,
}

/// Un ordre à donner.
#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    pub entity_id: EntityId,
    pub service: &'static str,
    pub data: Attributes,
    /// `true` pour une coupure, `false` pour une remise en marche.
    pub cut: bool,
}

/// Ce que fait l'appareil, s'il est en marche (ce qu'il faudra lui rendre).
fn running(states: &StateMachine, entity_id: &EntityId) -> Option<String> {
    let state = states.get(entity_id)?;
    let value = state.state();
    if matches!(value, "unavailable" | "unknown" | "off") {
        return None;
    }
    match entity_id.domain() {
        "climate" => Some(value.to_owned()),
        _ => (value == "on").then(|| "on".to_owned()),
    }
}

fn cut_order(entity_id: &EntityId) -> Order {
    let (service, data) = if entity_id.domain() == "climate" {
        ("set_hvac_mode", hvac("off"))
    } else {
        ("turn_off", Attributes::new())
    };
    Order {
        entity_id: entity_id.clone(),
        service,
        data,
        cut: true,
    }
}

fn hvac(mode: &str) -> Attributes {
    let mut data = Attributes::new();
    data.insert("hvac_mode".into(), mode.into());
    data
}

/// Remet l'appareil comme avant, s'il est toujours éteint : rallumé à la
/// main entre-temps, il est laissé tel quel.
fn restore_order(states: &StateMachine, cut: &Cut) -> Option<Order> {
    let entity_id: EntityId = cut.entity_id.parse().ok()?;
    if states.get(&entity_id)?.state() != "off" {
        return None;
    }
    let (service, data) = if entity_id.domain() == "climate" {
        ("set_hvac_mode", hvac(&cut.before))
    } else {
        ("turn_on", Attributes::new())
    };
    Some(Order {
        entity_id,
        service,
        data,
        cut: false,
    })
}

/// La décision, sans horloge ni bus : testable telle quelle.
#[derive(Debug, Default)]
pub struct Shedder {
    /// Ce qui est coupé, dans l'ordre des coupures.
    pub cut: Vec<Cut>,
    last_cut: Option<Instant>,
    calm_since: Option<Instant>,
    last_restore: Option<Instant>,
}

impl Shedder {
    /// Reprend les coupures d'avant un redémarrage.
    #[must_use]
    pub fn resume(cut: Vec<Cut>) -> Self {
        Self {
            cut,
            ..Self::default()
        }
    }

    /// Une vérification. `reading` : la mesure (`None` : inconnue, compteur
    /// muet). Coupe si elle dépasse la limite ; rallume si elle est restée
    /// sous 85 % de la limite (ou inconnue) assez longtemps.
    pub fn step(
        &mut self,
        config: &Config,
        states: &StateMachine,
        now: Instant,
        reading: Option<f64>,
        limit: f64,
    ) -> Option<Order> {
        if reading.is_some_and(|value| value > limit) {
            self.calm_since = None;
            if self
                .last_cut
                .is_some_and(|at| now.duration_since(at) < CUT_EVERY)
            {
                return None;
            }
            let (device, before) = config.devices.iter().find_map(|device| {
                if self.cut.iter().any(|cut| cut.entity_id == device.as_str()) {
                    return None;
                }
                running(states, device).map(|before| (device, before))
            })?;
            self.cut.push(Cut {
                entity_id: device.to_string(),
                before,
            });
            self.last_cut = Some(now);
            return Some(cut_order(device));
        }
        if reading.is_some_and(|value| value > limit * RESTORE_BELOW) {
            self.calm_since = None;
            return None;
        }
        let since = *self.calm_since.get_or_insert(now);
        if now.duration_since(since) < config.restore_after
            || self
                .last_restore
                .is_some_and(|at| now.duration_since(at) < RESTORE_EVERY)
        {
            return None;
        }
        while let Some(cut) = self.cut.pop() {
            self.last_restore = Some(now);
            if let Some(order) = restore_order(states, &cut) {
                return Some(order);
            }
        }
        None
    }

    /// Désarmé : tout ce qui a été coupé est rallumé tout de suite.
    pub fn release(&mut self, states: &StateMachine) -> Vec<Order> {
        let cut = std::mem::take(&mut self.cut);
        self.calm_since = None;
        cut.iter()
            .rev()
            .filter_map(|cut| restore_order(states, cut))
            .collect()
    }
}

/// La mesure, et la limite qui va avec (celle du fichier, ou celle du Linky).
fn reading_and_limit(core: &Core, config: &Config) -> (Option<f64>, Option<f64>) {
    let measure = core.states.get(&config.measure);
    let reading = measure
        .as_ref()
        .and_then(|state| state.state().trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0);
    let limit = config.limit.or_else(|| {
        let unit = measure
            .as_ref()?
            .attributes()
            .get("unit_of_measurement")?
            .as_str()?
            .to_owned();
        let linky = core.states.get(&LINKY.parse().ok()?)?;
        let subscribed = linky.attributes().get("puissance_souscrite")?.as_str()?;
        subscribed_limit(subscribed, &unit)
    });
    (reading, limit)
}

/// Le service.
#[derive(Debug)]
pub struct Shedding(Background);

impl Shedding {
    /// Démarre le délestage (désarmé, sauf s'il était armé avant l'arrêt du
    /// serveur ; ses coupures en cours sont reprises).
    #[must_use]
    pub fn start(config: Config, core: Arc<Core>) -> Self {
        let events = core.bus.subscribe();
        Self(Background::spawn("le délestage", move |token| {
            run(config, core, events, token)
        }))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

fn publish(core: &Core, config: &Config, armed: bool, shedder: &Shedder, limit: Option<f64>) {
    let Ok(entity_id) = SWITCH.parse::<EntityId>() else {
        return;
    };
    let mut attributes: Attributes = match json!({
        "friendly_name": NAME,
        "icon": "mdi:transmission-tower-off",
        "mesure": config.measure.as_str(),
        "appareils": config.devices.iter().map(EntityId::as_str).collect::<Vec<_>>(),
        "coupes": shedder.cut,
    }) {
        Value::Object(map) => map,
        _ => Attributes::new(),
    };
    match limit {
        Some(limit) => {
            attributes.insert("limite".into(), limit.into());
        }
        None => {
            attributes.insert(
                "erreur".into(),
                "limite inconnue : indique « limite » dans delestage.yaml".into(),
            );
        }
    }
    let previous = core.states.get(&entity_id);
    let value = if armed { "on" } else { "off" };
    // Rien de changé : pas de nouvel état toutes les 2 secondes.
    if previous
        .as_ref()
        .is_some_and(|state| state.state() == value && state.attributes() == &attributes)
    {
        return;
    }
    if let Err(error) = core.states.set(entity_id, value, attributes) {
        warn!("délestage : état refusé ({error})");
    }
}

fn send(core: &Core, order: Order, reading: Option<f64>, limit: Option<f64>) {
    let action = if order.cut { "coupe" } else { "rallume" };
    info!(
        "délestage : {action} {} ({} sur {})",
        order.entity_id,
        reading.map_or_else(|| "?".to_owned(), |v| format!("{v}")),
        limit.map_or_else(|| "?".to_owned(), |v| format!("{v}"))
    );
    let entity_id = order.entity_id.to_string();
    if let Err(error) = core.call_service_from(
        Origin::Automation(NAME.into()),
        order.entity_id.domain(),
        order.service,
        order.entity_id.clone(),
        order.data,
    ) {
        warn!("délestage : {entity_id} refusé ({error})");
        return;
    }
    let mut data = Attributes::new();
    data.insert("action".into(), action.into());
    data.insert("entity_id".into(), entity_id.into());
    if let Some(reading) = reading {
        data.insert("mesure".into(), reading.into());
    }
    if let Some(limit) = limit {
        data.insert("limite".into(), limit.into());
    }
    if let Err(error) = core.fire(EVENT, data) {
        warn!("délestage : événement refusé ({error})");
    }
}

async fn run(
    config: Config,
    core: Arc<Core>,
    mut events: tokio::sync::broadcast::Receiver<Arc<Event>>,
    token: CancellationToken,
) {
    let switch: Option<EntityId> = SWITCH.parse().ok();
    // Armé avant l'arrêt du serveur ? Ses coupures en cours aussi (l'état
    // est restauré au démarrage) : elles seront rallumées le moment venu.
    let previous = switch.as_ref().and_then(|id| core.states.get(id));
    let mut armed = previous.as_ref().is_some_and(|state| state.state() == "on");
    let resumed: Vec<Cut> = previous
        .as_ref()
        .and_then(|state| state.attributes().get("coupes").cloned())
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let mut shedder = Shedder::resume(resumed);
    let (_, limit) = reading_and_limit(&core, &config);
    publish(&core, &config, armed, &shedder, limit);
    info!(
        "délestage : {} appareil(s), mesure {}{}",
        config.devices.len(),
        config.measure,
        if armed { " (armé)" } else { "" }
    );
    let mut ticker = tokio::time::interval(TICK);
    loop {
        tokio::select! {
            () = token.cancelled() => break,
            _ = ticker.tick() => {}
            received = events.recv() => match received {
                Ok(event) => {
                    let Event::CallService { domain, service, entity_id, .. } = event.as_ref() else {
                        continue;
                    };
                    if domain != "switch" || Some(entity_id) != switch.as_ref() {
                        continue;
                    }
                    let wanted = match service.as_str() {
                        "turn_on" => true,
                        "turn_off" => false,
                        "toggle" => !armed,
                        _ => continue,
                    };
                    if wanted != armed {
                        armed = wanted;
                        info!("délestage {}", if armed { "armé" } else { "désarmé" });
                        if !armed {
                            let (reading, limit) = reading_and_limit(&core, &config);
                            for order in shedder.release(&core.states) {
                                send(&core, order, reading, limit);
                            }
                        }
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            },
        }
        let (reading, limit) = reading_and_limit(&core, &config);
        if armed
            && let Some(limit) = limit
            && let Some(order) = shedder.step(&config, &core.states, Instant::now(), reading, limit)
        {
            send(&core, order, reading, Some(limit));
        }
        publish(&core, &config, armed, &shedder, limit);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    fn id(text: &str) -> EntityId {
        text.parse().unwrap()
    }

    fn config() -> Config {
        parse_config(
            "appareils:\n  - switch.chauffe_eau\n  - climate.bureau\n  - switch.radiateur\n",
        )
        .unwrap()
    }

    #[test]
    fn the_config_is_checked() {
        let config = config();
        assert_eq!(config.measure, id(DEFAULT_MEASURE));
        assert_eq!(config.limit, None);
        assert_eq!(config.restore_after, Duration::from_mins(5));
        assert_eq!(config.devices.len(), 3);
        let full = parse_config(
            "mesure: sensor.linky_puissance\nlimite: 5800\nappareils: [switch.a, switch.a]\nretour_apres: 10\n",
        )
        .unwrap();
        assert_eq!(full.limit, Some(5800.0));
        assert_eq!(full.devices.len(), 1, "sans doublon");
        for bad in [
            "appareils: []\n",
            "appareils: [lock.entree]\n",
            "appareils: [cover.garage]\n",
            "appareils: [switch.a]\nlimite: 0\n",
            "appareils: [switch.a]\nlimite: -3\n",
            "appareils: [switch.a]\nmesure: switch.b\n",
            "appareils: [switch.a]\nretour_apres: 0\n",
            "appareils: [switch.a]\ncouleur: bleu\n",
        ] {
            assert!(parse_config(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_example_file_is_understood() {
        let config = parse_config(include_str!("../../../delestage.example.yaml")).unwrap();
        assert_eq!(config.devices.len(), 3);
    }

    #[test]
    fn the_limit_comes_from_the_subscription() {
        assert_eq!(subscribed_limit("30 A", "A"), Some(30.0));
        assert_eq!(subscribed_limit("6 kVA", "VA"), Some(6000.0));
        assert_eq!(subscribed_limit("6 kVA", "W"), Some(6000.0));
        assert_eq!(subscribed_limit("30 A", "VA"), None, "unités différentes");
        assert_eq!(subscribed_limit("beaucoup", "A"), None);
        assert_eq!(subscribed_limit("0 A", "A"), None);
    }

    #[test]
    fn devices_are_cut_one_by_one_then_restored_in_reverse() {
        let core = Core::new();
        let states = &core.states;
        let set = |entity: &str, value: &str| {
            states.set(id(entity), value, Attributes::new()).unwrap();
        };
        set("switch.chauffe_eau", "on");
        set("climate.bureau", "heat");
        // Déjà éteint : rien à couper.
        set("switch.radiateur", "off");
        let config = config();
        let mut shedder = Shedder::default();
        let start = Instant::now();
        let at = |secs: u64| start + Duration::from_secs(secs);

        // 32 A sur 30 : le chauffe-eau d'abord…
        let order = shedder
            .step(&config, states, at(0), Some(32.0), 30.0)
            .unwrap();
        assert_eq!(
            (order.entity_id.as_str(), order.service),
            ("switch.chauffe_eau", "turn_off")
        );
        set("switch.chauffe_eau", "off");
        // … puis on attend que la mesure suive.
        assert_eq!(shedder.step(&config, states, at(5), Some(31.0), 30.0), None);
        let order = shedder
            .step(&config, states, at(16), Some(31.0), 30.0)
            .unwrap();
        assert_eq!(order.entity_id.as_str(), "climate.bureau");
        assert_eq!(order.service, "set_hvac_mode");
        assert_eq!(order.data["hvac_mode"], "off");
        set("climate.bureau", "off");
        // Plus rien d'allumé à couper.
        assert_eq!(
            shedder.step(&config, states, at(40), Some(31.0), 30.0),
            None
        );
        assert_eq!(shedder.cut.len(), 2);

        // 27 A : sous la limite, mais au-dessus de 85 % : on attend.
        assert_eq!(
            shedder.step(&config, states, at(50), Some(27.0), 30.0),
            None
        );
        // 20 A pendant 5 minutes : le thermostat revient d'abord, à son mode
        // d'avant…
        assert_eq!(
            shedder.step(&config, states, at(60), Some(20.0), 30.0),
            None
        );
        assert_eq!(
            shedder.step(&config, states, at(300), Some(20.0), 30.0),
            None
        );
        let order = shedder
            .step(&config, states, at(361), Some(20.0), 30.0)
            .unwrap();
        assert_eq!(order.entity_id.as_str(), "climate.bureau");
        assert_eq!(order.data["hvac_mode"], "heat");
        // … le chauffe-eau une minute après.
        assert_eq!(
            shedder.step(&config, states, at(400), Some(20.0), 30.0),
            None
        );
        let order = shedder
            .step(&config, states, at(422), Some(20.0), 30.0)
            .unwrap();
        assert_eq!(
            (order.entity_id.as_str(), order.service),
            ("switch.chauffe_eau", "turn_on")
        );
        assert_eq!(shedder.cut, Vec::<Cut>::new());
    }

    #[test]
    fn a_device_turned_back_on_by_hand_is_left_alone() {
        let core = Core::new();
        let states = &core.states;
        states
            .set(id("switch.chauffe_eau"), "on", Attributes::new())
            .unwrap();
        let config = config();
        let mut shedder = Shedder::default();
        let start = Instant::now();
        assert!(
            shedder
                .step(&config, states, start, Some(35.0), 30.0)
                .is_some()
        );
        // Quelqu'un le rallume : il n'est plus recoupé…
        states
            .set(id("switch.chauffe_eau"), "on", Attributes::new())
            .unwrap();
        assert_eq!(
            shedder.step(
                &config,
                states,
                start + Duration::from_secs(20),
                Some(35.0),
                30.0
            ),
            None
        );
        // … ni rallumé une seconde fois : il est simplement oublié.
        for minutes in [10, 16] {
            assert_eq!(
                shedder.step(
                    &config,
                    states,
                    start + Duration::from_mins(minutes),
                    Some(10.0),
                    30.0
                ),
                None
            );
        }
        assert_eq!(shedder.cut, Vec::<Cut>::new());
    }

    #[test]
    fn disarming_restores_everything_at_once() {
        let core = Core::new();
        let states = &core.states;
        for entity in ["switch.chauffe_eau", "climate.bureau"] {
            states.set(id(entity), "off", Attributes::new()).unwrap();
        }
        let mut shedder = Shedder::resume(vec![
            Cut {
                entity_id: "switch.chauffe_eau".into(),
                before: "on".into(),
            },
            Cut {
                entity_id: "climate.bureau".into(),
                before: "heat".into(),
            },
        ]);
        let orders: Vec<&str> = shedder
            .release(states)
            .iter()
            .map(|order| order.service)
            .collect();
        assert_eq!(orders, ["set_hvac_mode", "turn_on"]);
        assert_eq!(shedder.cut, Vec::<Cut>::new());
    }

    #[test]
    fn a_silent_meter_cuts_nothing_but_lets_devices_come_back() {
        let core = Core::new();
        let states = &core.states;
        states
            .set(id("switch.chauffe_eau"), "off", Attributes::new())
            .unwrap();
        let config = config();
        let mut shedder = Shedder::resume(vec![Cut {
            entity_id: "switch.chauffe_eau".into(),
            before: "on".into(),
        }]);
        let start = Instant::now();
        assert_eq!(shedder.step(&config, states, start, None, 30.0), None);
        let order = shedder
            .step(&config, states, start + Duration::from_mins(5), None, 30.0)
            .unwrap();
        assert_eq!(order.service, "turn_on");
    }

    #[tokio::test]
    async fn the_service_cuts_and_announces_it() {
        let core = Arc::new(Core::new());
        let mut energy = Attributes::new();
        energy.insert("puissance_souscrite".into(), "30 A".into());
        core.states
            .set(id("sensor.linky_energie"), "1000", energy)
            .unwrap();
        let mut amps = Attributes::new();
        amps.insert("unit_of_measurement".into(), "A".into());
        core.states
            .set(id("sensor.linky_intensite"), "33", amps)
            .unwrap();
        core.states
            .set(id("switch.chauffe_eau"), "on", Attributes::new())
            .unwrap();
        let mut events = core.bus.subscribe();
        let shedding = Shedding::start(config(), Arc::clone(&core));
        tokio::time::sleep(Duration::from_millis(50)).await;
        core.call_service("switch", "turn_on", id(SWITCH), Attributes::new())
            .unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut cut = None;
        let mut announced = None;
        while cut.is_none() || announced.is_none() {
            let event = tokio::time::timeout_at(deadline, events.recv())
                .await
                .unwrap()
                .unwrap();
            match event.as_ref() {
                Event::CallService {
                    domain,
                    service,
                    origin,
                    ..
                } if domain == "switch" && service == "turn_off" => {
                    cut = Some(origin.clone());
                }
                Event::Fired { event_type, data } if event_type == EVENT => {
                    announced = Some(data.clone());
                }
                _ => {}
            }
        }
        assert_eq!(cut, Some(Origin::Automation(NAME.into())));
        let announced = announced.unwrap();
        assert_eq!(announced["action"], "coupe");
        assert_eq!(announced["entity_id"], "switch.chauffe_eau");
        assert_eq!(announced["limite"], 30.0);
        let switch = core.states.get(&id(SWITCH)).unwrap();
        assert_eq!(switch.state(), "on");
        assert_eq!(
            switch.attributes()["coupes"][0]["entity_id"],
            "switch.chauffe_eau"
        );
        shedding.stop().await;
    }
}
