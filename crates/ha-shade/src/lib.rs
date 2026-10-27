//! **Protection solaire** (étape 70) : quand le soleil tape sur une façade,
//! qu'il fait chaud et qu'il y a du soleil, ses volets descendent à la
//! position choisie ; quand le soleil est passé, ils remontent là où ils
//! étaient. Un volet bougé à la main est laissé tranquille un moment.
//!
//! ```yaml
//! # protection-solaire.yaml
//! temperature: sensor.meteo_temperature  # facultatif : sans mesure, la chaleur ne compte pas
//! seuil: 24                              # °C : protéger au-dessus (24 par défaut)
//! ecart: 2                               # °C : arrêter sous seuil - écart (2 par défaut)
//! meteo: weather.maison                  # facultatif : seulement par temps ensoleillé
//! pause_manuelle: 2                      # heures sans toucher un volet bougé à la main
//! volets:
//!   - entity_id: cover.salon
//!     orientation: sud                   # ou un azimut en degrés (180)
//!     position: 20                       # où le baisser (0 fermé, 100 ouvert ; 20 par défaut)
//!   - entity_id: cover.chambre
//!     orientation: ouest
//!     ouverture: 70                      # le soleil compte à ±70° de la façade (90 par défaut)
//!     hauteur_min: 15                    # degrés au-dessus de l'horizon (10 par défaut)
//! ```
//!
//! On l'arme et le désarme avec `switch.protection_solaire` (armée au premier
//! démarrage). La désarmer remonte tout de suite les volets qu'elle avait
//! baissés.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use ha_core::{Attributes, Background, CancellationToken, Core, EntityId, Event, Origin, State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use tokio::time::Instant;
use tracing::{info, warn};

/// L'interrupteur qui arme la protection.
pub const SWITCH: &str = "switch.protection_solaire";
/// Le nom donné aux ordres dans le journal de bord.
pub const NAME: &str = "Protection solaire";
/// L'événement publié à chaque mouvement.
pub const EVENT: &str = "protection_solaire";
/// Volets au plus.
pub const MAX_COVERS: usize = 30;
/// Entre deux mouvements d'un même volet : un nuage qui passe ne le fait pas
/// monter et descendre sans cesse.
pub const MIN_INTERVAL: Duration = Duration::from_mins(15);
/// Après un de nos ordres, le volet bouge pendant ce temps : ce n'est pas
/// une main.
const OWN_MOVE: Duration = Duration::from_mins(3);
/// Le temps qui protège : soleil, ou soleil voilé.
const SUNNY: [&str; 2] = ["sunny", "partlycloudy"];
/// Le rythme de la vérification.
const TICK: Duration = Duration::from_secs(30);

/// Les réglages, vérifiés.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub temperature: Option<EntityId>,
    pub threshold: f64,
    pub hysteresis: f64,
    pub weather: Option<EntityId>,
    pub manual_pause: Duration,
    pub covers: Vec<CoverConfig>,
}

/// Un volet et sa façade.
#[derive(Debug, Clone, PartialEq)]
pub struct CoverConfig {
    pub entity_id: EntityId,
    /// La direction vers laquelle regarde la façade (azimut : 0 nord, 90 est,
    /// 180 sud, 270 ouest).
    pub orientation: f64,
    /// Le soleil compte s'il est à moins de tant de degrés de cette direction.
    pub half_angle: f64,
    /// Et au moins à cette hauteur au-dessus de l'horizon.
    pub min_elevation: f64,
    /// La position où le baisser (0 fermé, 100 ouvert).
    pub position: u8,
}

impl CoverConfig {
    /// Le soleil tape-t-il sur cette façade ?
    #[must_use]
    pub fn sun_on_facade(&self, elevation: f64, azimuth: f64) -> bool {
        let diff = ((azimuth - self.orientation + 540.0).rem_euclid(360.0)) - 180.0;
        elevation >= self.min_elevation && diff.abs() <= self.half_angle
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Orientation {
    Degrees(f64),
    Name(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    temperature: Option<String>,
    #[serde(default)]
    seuil: Option<f64>,
    #[serde(default)]
    ecart: Option<f64>,
    #[serde(default)]
    meteo: Option<String>,
    #[serde(default)]
    pause_manuelle: Option<f64>,
    volets: Vec<CoverFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CoverFile {
    entity_id: String,
    orientation: Orientation,
    #[serde(default)]
    ouverture: Option<f64>,
    #[serde(default)]
    hauteur_min: Option<f64>,
    #[serde(default)]
    position: Option<u8>,
}

/// « sud-ouest » → 225.
fn cardinal(name: &str) -> Option<f64> {
    let normalized = name.trim().to_lowercase().replace([' ', '_'], "-");
    Some(match normalized.as_str() {
        "nord" | "n" => 0.0,
        "nord-est" | "ne" => 45.0,
        "est" | "e" => 90.0,
        "sud-est" | "se" => 135.0,
        "sud" | "s" => 180.0,
        "sud-ouest" | "so" => 225.0,
        "ouest" | "o" => 270.0,
        "nord-ouest" | "no" => 315.0,
        _ => return None,
    })
}

fn entity(raw: &str, what: &str, domain: &str) -> Result<EntityId, String> {
    let id: EntityId = raw
        .trim()
        .parse()
        .map_err(|_| format!("{what} : « {raw} » n'est pas un identifiant"))?;
    if id.domain() != domain {
        return Err(format!("{what} : un {domain}.… est attendu, pas {id}"));
    }
    Ok(id)
}

fn in_range(value: f64, what: &str, min: f64, max: f64) -> Result<f64, String> {
    if value.is_finite() && (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(format!("{what} : de {min} à {max}"))
    }
}

/// Vérifie le contenu de `protection-solaire.yaml`.
///
/// # Errors
/// Le fichier est mal écrit ou une valeur est hors limites (le message dit
/// laquelle).
pub fn parse_config(text: &str) -> Result<Config, String> {
    let file: File = serde_saphyr::from_str(text).map_err(|error| error.to_string())?;
    let temperature = file
        .temperature
        .as_deref()
        .map(|raw| entity(raw, "temperature", "sensor"))
        .transpose()?;
    let weather = file
        .meteo
        .as_deref()
        .map(|raw| entity(raw, "meteo", "weather"))
        .transpose()?;
    let threshold = in_range(file.seuil.unwrap_or(24.0), "seuil", -20.0, 50.0)?;
    let hysteresis = in_range(file.ecart.unwrap_or(2.0), "ecart", 0.0, 10.0)?;
    let pause = in_range(
        file.pause_manuelle.unwrap_or(2.0),
        "pause_manuelle",
        0.0,
        24.0,
    )?;
    if file.volets.is_empty() || file.volets.len() > MAX_COVERS {
        return Err(format!("volets : de 1 à {MAX_COVERS}"));
    }
    let mut covers: Vec<CoverConfig> = Vec::new();
    for cover in file.volets {
        let entity_id = entity(&cover.entity_id, "volet", "cover")?;
        if covers.iter().any(|other| other.entity_id == entity_id) {
            return Err(format!("{entity_id} : en double"));
        }
        let orientation = match cover.orientation {
            Orientation::Degrees(degrees) => {
                in_range(degrees, &format!("{entity_id} : orientation"), 0.0, 360.0)?
            }
            Orientation::Name(name) => cardinal(&name).ok_or_else(|| {
                format!(
                    "{entity_id} : orientation « {name} » inconnue (nord, nord-est, est, sud-est, \
                     sud, sud-ouest, ouest, nord-ouest, ou des degrés)"
                )
            })?,
        };
        let position = cover.position.unwrap_or(20);
        if position > 90 {
            return Err(format!(
                "{entity_id} : position de 0 (fermé) à 90 ; 100 serait grand ouvert"
            ));
        }
        covers.push(CoverConfig {
            orientation,
            half_angle: in_range(
                cover.ouverture.unwrap_or(90.0),
                &format!("{entity_id} : ouverture"),
                10.0,
                90.0,
            )?,
            min_elevation: in_range(
                cover.hauteur_min.unwrap_or(10.0),
                &format!("{entity_id} : hauteur_min"),
                0.0,
                60.0,
            )?,
            position,
            entity_id,
        });
    }
    Ok(Config {
        temperature,
        threshold,
        hysteresis,
        weather,
        manual_pause: Duration::from_secs_f64(pause * 3600.0),
        covers,
    })
}

/// Lit `protection-solaire.yaml` ; `None` si le fichier est absent.
///
/// # Errors
/// Le fichier existe mais ne peut pas être lu, ou il est faux.
pub fn load(path: &std::path::Path) -> Result<Option<Config>, String> {
    match ha_core::config_file::read(path, ha_core::config_file::DEFAULT_MAX) {
        Ok(Some(text)) => parse_config(&text)
            .map(Some)
            .map_err(|error| format!("{} : {error}", path.display())),
        Ok(None) => Ok(None),
        Err(error) => Err(format!(
            "lecture de {} impossible : {error}",
            path.display()
        )),
    }
}

/// Ce qui décide, à un instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sky {
    pub elevation: f64,
    pub azimuth: f64,
    /// Assez chaud (avec l'écart, voir [`Heat`]).
    pub hot: bool,
    /// Ensoleillé, ou sans météo configurée.
    pub sunny: bool,
    pub armed: bool,
}

/// La chaleur, avec son écart : on protège au-dessus du seuil, on arrête en
/// dessous de seuil - écart (pas à chaque dixième de degré).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Heat(bool);

impl Heat {
    pub fn update(&mut self, temperature: Option<f64>, config: &Config) -> bool {
        self.0 = match temperature {
            None => config.temperature.is_none() || self.0,
            Some(t) if t >= config.threshold => true,
            Some(t) if t < config.threshold - config.hysteresis => false,
            Some(_) => self.0,
        };
        self.0
    }
}

/// Une position, lue sur l'état d'un volet : `current_position`, sinon
/// ouvert = 100, fermé = 0.
#[must_use]
pub fn position_of(state: &State) -> Option<u8> {
    // MQTT la donne souvent en décimal (« 80.0 »).
    if let Some(position) = state
        .attributes()
        .get("current_position")
        .and_then(Value::as_f64)
        .filter(|p| p.is_finite())
    {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "borné à 0..=100 juste avant"
        )]
        return Some(position.round().clamp(0.0, 100.0) as u8);
    }
    match state.state() {
        "open" => Some(100),
        "closed" => Some(0),
        _ => None,
    }
}

/// Un ordre à donner à un volet.
#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    pub entity_id: EntityId,
    pub service: &'static str,
    pub data: Attributes,
    /// `true` : on baisse ; `false` : on remonte.
    pub lower: bool,
}

/// Où était un volet avant qu'on le baisse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lowered {
    pub entity_id: String,
    /// La position d'avant (100 : grand ouvert).
    #[serde(rename = "avant")]
    pub before: u8,
}

/// Le suivi d'un volet, sans horloge ni réseau.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tracker {
    /// Baissé par nous, et sa position d'avant.
    pub lowered: Option<u8>,
    last_command: Option<Instant>,
    /// La position demandée par notre dernier ordre.
    heading: Option<u8>,
    manual_until: Option<Instant>,
}

impl Tracker {
    #[must_use]
    pub fn paused(&self, now: Instant) -> bool {
        self.manual_until.is_some_and(|until| until > now)
    }

    fn may_move(&self, now: Instant) -> bool {
        self.last_command
            .is_none_or(|last| now.duration_since(last) >= MIN_INTERVAL)
    }

    /// Décide pour ce volet.
    pub fn step(
        &mut self,
        cover: &CoverConfig,
        sky: Sky,
        state: Option<&State>,
        now: Instant,
    ) -> Option<Order> {
        if self.paused(now) {
            return None;
        }
        let want =
            sky.armed && sky.hot && sky.sunny && cover.sun_on_facade(sky.elevation, sky.azimuth);
        match self.lowered {
            None if want && self.may_move(now) => {
                let state = state?;
                let current = position_of(state)?;
                // Déjà assez baissé : on n'y touche pas (et on ne le
                // remontera pas).
                if current <= cover.position.saturating_add(2) {
                    return None;
                }
                self.lowered = Some(current);
                self.last_command = Some(now);
                let order = lower_order(cover, state);
                self.heading = Some(order_target(&order));
                Some(order)
            }
            Some(before) if !want && (self.may_move(now) || !sky.armed) => {
                self.lowered = None;
                self.last_command = Some(now);
                let order = raise_order(cover, before, state);
                self.heading = Some(order_target(&order));
                Some(order)
            }
            _ => None,
        }
    }

    /// Un changement d'état du volet : si ce n'est pas notre ordre, c'est
    /// une main. On le laisse tranquille, et on oublie de le remonter.
    pub fn observe(&mut self, old: &State, new: &State, now: Instant, pause: Duration) -> bool {
        if ["unavailable", "unknown"].contains(&old.state())
            || ["unavailable", "unknown"].contains(&new.state())
        {
            return false;
        }
        let (before, after) = (position_of(old), position_of(new));
        let moved = before != after || old.state() != new.state();
        // Notre ordre : peu après, et vers la position demandée. Un volet
        // remonté à la main juste après qu'on l'a baissé est bien une main.
        let toward = match (self.heading, before, after) {
            (Some(target), Some(b), Some(a)) => a.abs_diff(target) <= b.abs_diff(target),
            _ => true,
        };
        let ours = toward
            && self
                .last_command
                .is_some_and(|last| now.duration_since(last) < OWN_MOVE);
        if !moved || ours || pause.is_zero() {
            return false;
        }
        // Vrai seulement au premier mouvement : un volet qui descend par
        // pas ne fait pas dix lignes de journal.
        let first = !self.paused(now);
        self.manual_until = Some(now + pause);
        self.lowered = None;
        first
    }
}

/// La position que vise un ordre.
fn order_target(order: &Order) -> u8 {
    match order.service {
        "close_cover" => 0,
        "open_cover" => 100,
        _ => order
            .data
            .get("position")
            .and_then(Value::as_u64)
            .and_then(|p| u8::try_from(p).ok())
            .unwrap_or(100),
    }
}

fn supports_position(state: &State) -> bool {
    state.attributes().contains_key("current_position")
}

fn lower_order(cover: &CoverConfig, state: &State) -> Order {
    let mut data = Attributes::new();
    let service = if supports_position(state) && cover.position > 0 {
        data.insert("position".into(), cover.position.into());
        "set_cover_position"
    } else {
        "close_cover"
    };
    Order {
        entity_id: cover.entity_id.clone(),
        service,
        data,
        lower: true,
    }
}

fn raise_order(cover: &CoverConfig, before: u8, state: Option<&State>) -> Order {
    let mut data = Attributes::new();
    let service = if before < 100 && state.is_some_and(supports_position) {
        data.insert("position".into(), before.into());
        "set_cover_position"
    } else {
        "open_cover"
    };
    Order {
        entity_id: cover.entity_id.clone(),
        service,
        data,
        lower: false,
    }
}

/// D'où vient la position du soleil (l'horloge, ou une valeur pour les
/// essais).
pub type SunSource = Box<dyn Fn() -> Option<(f64, f64)> + Send + Sync>;

/// La position réelle du soleil, maintenant, à cet endroit.
pub fn real_sun(location: ha_sun::Location) -> SunSource {
    Box::new(move || ha_sun::position(jiff::Timestamp::now(), location))
}

/// Le service.
#[derive(Debug)]
pub struct Shades(Background);

impl Shades {
    /// Démarre la protection (armée, sauf si elle était désarmée avant
    /// l'arrêt du serveur ; ses volets baissés sont repris).
    #[must_use]
    pub fn start(config: Config, sun: SunSource, core: Arc<Core>) -> Self {
        let events = core.bus.subscribe();
        Self(Background::spawn("la protection solaire", move |token| {
            run(config, sun, core, events, token)
        }))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

fn number(core: &Core, id: Option<&EntityId>) -> Option<f64> {
    core.states
        .get(id?)?
        .state()
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

fn sunny(core: &Core, config: &Config) -> bool {
    let Some(weather) = &config.weather else {
        return true;
    };
    core.states
        .get(weather)
        .is_some_and(|state| SUNNY.contains(&state.state()))
}

fn publish(core: &Core, config: &Config, sky: Sky, trackers: &[Tracker], now: Instant) {
    let Ok(entity_id) = SWITCH.parse::<EntityId>() else {
        return;
    };
    let lowered: Vec<Lowered> = config
        .covers
        .iter()
        .zip(trackers)
        .filter_map(|(cover, tracker)| {
            tracker.lowered.map(|before| Lowered {
                entity_id: cover.entity_id.to_string(),
                before,
            })
        })
        .collect();
    let paused: Vec<&str> = config
        .covers
        .iter()
        .zip(trackers)
        .filter(|(_, tracker)| tracker.paused(now))
        .map(|(cover, _)| cover.entity_id.as_str())
        .collect();
    let facing: Vec<&str> = config
        .covers
        .iter()
        .filter(|cover| cover.sun_on_facade(sky.elevation, sky.azimuth))
        .map(|cover| cover.entity_id.as_str())
        .collect();
    let attributes: Attributes = match json!({
        "friendly_name": NAME,
        "icon": "mdi:sun-angle",
        "soleil_hauteur": (sky.elevation * 10.0).round() / 10.0,
        "soleil_azimut": (sky.azimuth * 10.0).round() / 10.0,
        "chaleur": sky.hot,
        "ensoleille": sky.sunny,
        "soleil_sur": facing,
        "baisses": lowered,
        "en_pause": paused,
        "volets": config.covers.iter().map(|c| c.entity_id.as_str()).collect::<Vec<_>>(),
    }) {
        Value::Object(map) => map,
        _ => Attributes::new(),
    };
    let value = if sky.armed { "on" } else { "off" };
    let unchanged = core
        .states
        .get(&entity_id)
        .is_some_and(|state| state.state() == value && state.attributes() == &attributes);
    if unchanged {
        return;
    }
    if let Err(error) = core.states.set(entity_id, value, attributes) {
        warn!("protection solaire : état refusé ({error})");
    }
}

fn send(core: &Core, order: Order) {
    let action = if order.lower { "baisse" } else { "remonte" };
    info!("protection solaire : {action} {}", order.entity_id);
    let mut event = Attributes::new();
    event.insert("action".into(), action.into());
    event.insert("entity_id".into(), order.entity_id.as_str().into());
    if let Some(position) = order.data.get("position") {
        event.insert("position".into(), position.clone());
    }
    if let Err(error) = core.call_service_from(
        Origin::Automation(NAME.into()),
        "cover",
        order.service,
        order.entity_id.clone(),
        order.data,
    ) {
        warn!("protection solaire : {} refusé ({error})", order.entity_id);
        return;
    }
    if let Err(error) = core.fire(EVENT, event) {
        warn!("protection solaire : événement refusé ({error})");
    }
}

async fn run(
    config: Config,
    sun: SunSource,
    core: Arc<Core>,
    mut events: tokio::sync::broadcast::Receiver<Arc<Event>>,
    token: CancellationToken,
) {
    let switch: Option<EntityId> = SWITCH.parse().ok();
    let previous = switch.as_ref().and_then(|id| core.states.get(id));
    // Armée au premier démarrage ; sinon, comme avant l'arrêt.
    let mut armed = previous.as_ref().is_none_or(|state| state.state() != "off");
    let resumed: Vec<Lowered> = previous
        .as_ref()
        .and_then(|state| state.attributes().get("baisses").cloned())
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let mut trackers: Vec<Tracker> = config
        .covers
        .iter()
        .map(|cover| Tracker {
            lowered: resumed
                .iter()
                .find(|item| item.entity_id == cover.entity_id.as_str())
                .map(|item| item.before),
            ..Tracker::default()
        })
        .collect();
    let index: HashMap<EntityId, usize> = config
        .covers
        .iter()
        .enumerate()
        .map(|(i, cover)| (cover.entity_id.clone(), i))
        .collect();
    let mut heat = Heat::default();
    info!(
        "protection solaire : {} volet(s){}",
        config.covers.len(),
        if armed { "" } else { " (désarmée)" }
    );
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = token.cancelled() => break,
            _ = ticker.tick() => {}
            received = events.recv() => match received {
                Ok(event) => match event.as_ref() {
                    Event::CallService { domain, service, entity_id, .. }
                        if domain == "switch" && Some(entity_id) == switch.as_ref() =>
                    {
                        let wanted = match service.as_str() {
                            "turn_on" => true,
                            "turn_off" => false,
                            "toggle" => !armed,
                            _ => continue,
                        };
                        if wanted != armed {
                            armed = wanted;
                            info!("protection solaire {}", if armed { "armée" } else { "désarmée" });
                        }
                    }
                    Event::StateChanged { entity_id, old_state: Some(old), new_state: Some(new) } => {
                        if let Some(&i) = index.get(entity_id)
                            && let Some(tracker) = trackers.get_mut(i)
                            && tracker.observe(old, new, Instant::now(), config.manual_pause)
                        {
                            info!("protection solaire : {entity_id} bougé à la main, laissé tranquille");
                        }
                        let decides = config.temperature.as_ref() == Some(entity_id)
                            || config.weather.as_ref() == Some(entity_id);
                        if !decides && !index.contains_key(entity_id) {
                            continue;
                        }
                    }
                    _ => continue,
                },
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            },
        }
        let now = Instant::now();
        let (elevation, azimuth) = sun().unwrap_or((-90.0, 0.0));
        let sky = Sky {
            elevation,
            azimuth,
            hot: heat.update(number(&core, config.temperature.as_ref()), &config),
            sunny: sunny(&core, &config),
            armed,
        };
        for (cover, tracker) in config.covers.iter().zip(trackers.iter_mut()) {
            let state = core.states.get(&cover.entity_id);
            if let Some(order) = tracker.step(cover, sky, state.as_ref(), now) {
                send(&core, order);
            }
        }
        publish(&core, &config, sky, &trackers, now);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

    use super::*;

    const FILE: &str = "
temperature: sensor.dehors
seuil: 25
meteo: weather.maison
volets:
  - entity_id: cover.salon
    orientation: sud
  - entity_id: cover.chambre
    orientation: 270
    ouverture: 60
    hauteur_min: 15
    position: 0
";

    fn config() -> Config {
        parse_config(FILE).unwrap()
    }

    fn cover_state(state: &str, position: Option<u8>) -> State {
        let core = Core::new();
        let id: EntityId = "cover.salon".parse().unwrap();
        let mut attributes = Attributes::new();
        if let Some(position) = position {
            attributes.insert("current_position".into(), position.into());
        }
        core.states.set(id.clone(), state, attributes).unwrap();
        core.states.get(&id).unwrap()
    }

    fn sky(elevation: f64, azimuth: f64) -> Sky {
        Sky {
            elevation,
            azimuth,
            hot: true,
            sunny: true,
            armed: true,
        }
    }

    #[test]
    fn the_file_is_checked() {
        let config = config();
        assert_eq!(config.threshold, 25.0);
        assert_eq!(config.hysteresis, 2.0);
        assert_eq!(config.manual_pause, Duration::from_hours(2));
        assert_eq!(config.covers[0].orientation, 180.0);
        assert_eq!(config.covers[0].position, 20);
        assert_eq!(config.covers[1].half_angle, 60.0);
        assert_eq!(cardinal("Sud-Ouest"), Some(225.0));
        let base = "volets:\n  - {entity_id: cover.a, orientation: sud}\n";
        for (bad, hint) in [
            ("volets: []\n".to_owned(), "volets"),
            ("volets:\n  - {entity_id: light.a, orientation: sud}\n".to_owned(), "cover"),
            ("volets:\n  - {entity_id: cover.a, orientation: sudouest}\n".to_owned(), "inconnue"),
            ("volets:\n  - {entity_id: cover.a, orientation: 400}\n".to_owned(), "orientation"),
            ("volets:\n  - {entity_id: cover.a, orientation: sud, position: 100}\n".to_owned(), "position"),
            ("volets:\n  - {entity_id: cover.a, orientation: sud}\n  - {entity_id: cover.a, orientation: est}\n".to_owned(), "double"),
            (format!("{base}seuil: 80\n"), "seuil"),
            (format!("{base}temperature: weather.maison\n"), "sensor"),
            (format!("{base}couleur: bleu\n"), "couleur"),
        ] {
            let error = parse_config(&bad).unwrap_err();
            assert!(error.contains(hint), "{bad} : {error}");
        }
        let example =
            parse_config(include_str!("../../../protection-solaire.example.yaml")).unwrap();
        assert_eq!(example.covers.len(), 3);
    }

    #[test]
    fn the_sun_must_face_the_facade() {
        let south = &config().covers[0];
        assert!(south.sun_on_facade(40.0, 180.0));
        assert!(south.sun_on_facade(40.0, 95.0), "à 85° de la façade");
        assert!(!south.sun_on_facade(40.0, 85.0), "derrière la façade");
        assert!(!south.sun_on_facade(5.0, 180.0), "trop bas");
        let west = &config().covers[1];
        assert!(west.sun_on_facade(20.0, 320.0), "azimut qui passe par 360");
        assert!(!west.sun_on_facade(20.0, 200.0), "hors des ±60°");
    }

    #[test]
    fn heat_has_a_margin() {
        let config = config();
        let mut heat = Heat::default();
        assert!(!heat.update(Some(24.0), &config));
        assert!(heat.update(Some(25.0), &config));
        assert!(heat.update(Some(23.5), &config), "dans l'écart");
        assert!(!heat.update(Some(22.9), &config));
        // Sans mesure configurée, la chaleur ne compte pas.
        let mut free = Heat::default();
        let mut no_sensor = config.clone();
        no_sensor.temperature = None;
        assert!(free.update(None, &no_sensor));
    }

    #[test]
    fn a_shutter_goes_down_then_back_where_it_was() {
        let config = config();
        let cover = &config.covers[0];
        let start = Instant::now();
        let at = |minutes: u64| start + Duration::from_mins(minutes);
        let mut tracker = Tracker::default();
        let open = cover_state("open", Some(80));
        // Le soleil n'est pas encore sur la façade.
        assert_eq!(
            tracker.step(cover, sky(30.0, 80.0), Some(&open), at(0)),
            None
        );
        let order = tracker
            .step(cover, sky(40.0, 150.0), Some(&open), at(1))
            .unwrap();
        assert_eq!(order.service, "set_cover_position");
        assert_eq!(order.data["position"], 20);
        assert!(order.lower);
        assert_eq!(tracker.lowered, Some(80));
        // Le soleil passe à l'ouest : il remonte à 80, pas à 100.
        let down = cover_state("open", Some(20));
        assert_eq!(
            tracker.step(cover, sky(40.0, 280.0), Some(&down), at(5)),
            None,
            "15 min d'abord"
        );
        let order = tracker
            .step(cover, sky(40.0, 280.0), Some(&down), at(16))
            .unwrap();
        assert_eq!(order.data["position"], 80);
        assert!(!order.lower);
        assert_eq!(tracker.lowered, None);
        // Déjà baissé à la main plus bas : on n'y touche pas.
        let mut tracker = Tracker::default();
        let low = cover_state("open", Some(10));
        assert_eq!(
            tracker.step(cover, sky(40.0, 180.0), Some(&low), at(30)),
            None
        );
        // Sans position : fermer, puis ouvrir.
        let mut tracker = Tracker::default();
        let plain = cover_state("open", None);
        let order = tracker
            .step(cover, sky(40.0, 180.0), Some(&plain), at(40))
            .unwrap();
        assert_eq!(order.service, "close_cover");
        let order = tracker
            .step(cover, sky(-5.0, 280.0), Some(&plain), at(60))
            .unwrap();
        assert_eq!(order.service, "open_cover");
    }

    #[test]
    fn a_hand_wins() {
        let config = config();
        let cover = &config.covers[0];
        let start = Instant::now();
        let at = |minutes: u64| start + Duration::from_mins(minutes);
        let mut tracker = Tracker::default();
        let open = cover_state("open", Some(100));
        tracker
            .step(cover, sky(40.0, 180.0), Some(&open), at(0))
            .unwrap();
        let down = cover_state("open", Some(20));
        // Notre propre mouvement n'est pas une main.
        assert!(!tracker.observe(&open, &down, at(1), config.manual_pause));
        // Remonté à la main juste après : c'est une main, même dans les 3 min.
        let mut quick = tracker.clone();
        let decimal = {
            let core = Core::new();
            let mut attributes = Attributes::new();
            attributes.insert("current_position".into(), json!(79.6));
            core.states
                .set("cover.x".parse().unwrap(), "open", attributes)
                .unwrap();
            core.states.get(&"cover.x".parse().unwrap()).unwrap()
        };
        assert_eq!(position_of(&decimal), Some(80), "position décimale de MQTT");
        assert!(quick.observe(&down, &open, at(2), config.manual_pause));
        // Quelqu'un le remonte : on le laisse tranquille 2 h, et on oublie
        // de le remonter.
        assert!(tracker.observe(&down, &open, at(20), config.manual_pause));
        assert_eq!(tracker.lowered, None);
        assert_eq!(
            tracker.step(cover, sky(40.0, 180.0), Some(&open), at(60)),
            None
        );
        assert!(
            tracker
                .step(cover, sky(40.0, 180.0), Some(&open), at(141))
                .is_some()
        );
        // Un volet qui revient d'« indisponible » n'est pas une main.
        let gone = cover_state("unavailable", None);
        assert!(!tracker.observe(&gone, &open, at(200), config.manual_pause));
    }

    #[test]
    fn nothing_moves_when_disarmed_cloudy_or_cool() {
        let config = config();
        let cover = &config.covers[0];
        let open = cover_state("open", Some(100));
        let now = Instant::now();
        for sky in [
            Sky {
                armed: false,
                ..sky(40.0, 180.0)
            },
            Sky {
                sunny: false,
                ..sky(40.0, 180.0)
            },
            Sky {
                hot: false,
                ..sky(40.0, 180.0)
            },
        ] {
            assert_eq!(
                Tracker::default().step(cover, sky, Some(&open), now),
                None,
                "{sky:?}"
            );
        }
        // Désarmée : remonte tout de suite, même avant 15 min.
        let mut tracker = Tracker::default();
        tracker
            .step(cover, sky(40.0, 180.0), Some(&open), now)
            .unwrap();
        let order = tracker
            .step(
                cover,
                Sky {
                    armed: false,
                    ..sky(40.0, 180.0)
                },
                Some(&open),
                now + Duration::from_mins(1),
            )
            .unwrap();
        assert!(!order.lower);
    }

    #[tokio::test(start_paused = true)]
    async fn the_service_lowers_raises_and_remembers() {
        let core = Arc::new(Core::new());
        let set = |id: &str, state: &str, attrs: Value| {
            let Value::Object(attrs) = attrs else {
                panic!()
            };
            core.states.set(id.parse().unwrap(), state, attrs).unwrap();
        };
        set("cover.salon", "open", json!({"current_position": 100}));
        set("cover.chambre", "open", json!({"current_position": 100}));
        set("sensor.dehors", "28", json!({}));
        set("weather.maison", "sunny", json!({}));
        let azimuth = Arc::new(std::sync::Mutex::new(180.0_f64));
        let source = Arc::clone(&azimuth);
        let sun: SunSource = Box::new(move || Some((40.0, *source.lock().unwrap())));
        let mut rx = core.bus.subscribe();
        let service = Shades::start(config(), sun, Arc::clone(&core));
        tokio::time::sleep(Duration::from_secs(1)).await;
        let mut orders = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let Event::CallService {
                domain,
                service,
                entity_id,
                data,
                ..
            } = event.as_ref()
                && domain == "cover"
            {
                orders.push(format!(
                    "{entity_id} {service} {}",
                    data.get("position").map_or(String::new(), Value::to_string)
                ));
            }
        }
        assert_eq!(orders, ["cover.salon set_cover_position 20"]);
        let switch = core.states.get(&SWITCH.parse().unwrap()).unwrap();
        assert_eq!(switch.state(), "on");
        assert_eq!(
            switch.attributes()["baisses"],
            json!([{"entity_id": "cover.salon", "avant": 100}])
        );
        // Désarmée : il remonte aussitôt.
        core.call_service(
            "switch",
            "turn_off",
            SWITCH.parse().unwrap(),
            Attributes::new(),
        )
        .unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
        let mut raised = false;
        while let Ok(event) = rx.try_recv() {
            if let Event::CallService {
                domain, service, ..
            } = event.as_ref()
                && domain == "cover"
                && service == "open_cover"
            {
                raised = true;
            }
        }
        assert!(raised);
        let switch = core.states.get(&SWITCH.parse().unwrap()).unwrap();
        assert_eq!(switch.state(), "off");
        assert_eq!(switch.attributes()["baisses"], json!([]));
        service.stop().await;
    }
}
