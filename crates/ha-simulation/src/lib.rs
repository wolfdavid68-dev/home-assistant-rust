//! **Simulation de présence** (étape 58) : quand la maison est vide, les
//! lumières (et, si on veut, les volets) refont ce qu'elles ont fait il y a
//! une semaine, à la même heure. Vue de la rue, la maison a l'air habitée,
//! avec des horaires vraisemblables, puisque ce sont les vôtres.
//!
//! ```yaml
//! # simulation.yaml
//! appareils:
//!   - light.salon
//!   - light.cuisine
//!   - light.chambre
//!   - cover.salon
//! il_y_a: 7     # rejouer d'il y a 7 jours (1 à 9 ; l'historique est gardé 10 jours)
//! ```
//!
//! On l'**arme** avec `switch.simulation_presence` (tableau de bord,
//! automatisation « quand je pars en vacances »…). Armée, elle ne rejoue que
//! pendant que **personne n'est à la maison** (`zone.home` vaut 0) : au
//! retour du premier, elle s'arrête et ne touche plus à rien.
//!
//! Le journal de bord dit « par l'automatisation « Simulation de
//! présence » ». Les ordres passent par les vérifications du noyau.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ha_core::{Attributes, Background, CancellationToken, Core, EntityId, Event, Origin};
use ha_storage::Store;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};

/// L'interrupteur qui arme la simulation.
pub const SWITCH: &str = "switch.simulation_presence";
/// Le nom donné aux ordres dans le journal de bord.
pub const NAME: &str = "Simulation de présence";
/// Appareils au plus.
pub const MAX_DEVICES: usize = 30;
const DOMAINS: [&str; 5] = ["light", "switch", "fan", "input_boolean", "cover"];

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

/// Ce qui est rejoué.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub devices: Vec<EntityId>,
    /// Combien de jours en arrière.
    pub days_ago: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    appareils: Vec<String>,
    #[serde(default = "seven")]
    il_y_a: u8,
}

fn seven() -> u8 {
    7
}

/// Vérifie le contenu de `simulation.yaml`.
pub fn parse_config(text: &str) -> Result<Config, ConfigError> {
    let file: File =
        serde_saphyr::from_str(text).map_err(|error| ConfigError::Invalid(error.to_string()))?;
    if !(1..=9).contains(&file.il_y_a) {
        return Err(ConfigError::Invalid(
            "il_y_a : de 1 à 9 jours (l'historique est gardé 10 jours)".into(),
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
                "{entity_id} : seulement des lumières, prises, ventilateurs, interrupteurs virtuels et volets"
            )));
        }
        if !devices.contains(&entity_id) {
            devices.push(entity_id);
        }
    }
    Ok(Config {
        devices,
        days_ago: file.il_y_a,
    })
}

/// Lit `simulation.yaml` ; `None` si le fichier est absent.
pub fn load(path: &Path) -> Result<Option<Config>, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_config(&text).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ConfigError::Read {
            path: path.display().to_string(),
            source,
        }),
    }
}

/// Ce qu'un état passé veut dire : allumé / éteint, ouvert / fermé.
/// `None` : rien à rejouer (inconnu, injoignable…).
#[must_use]
pub fn wanted(domain: &str, past: &str) -> Option<&'static str> {
    match (domain, past) {
        ("cover", "open" | "opening") => Some("open"),
        ("cover", "closed" | "closing") => Some("closed"),
        ("cover", _) => None,
        (_, "on") => Some("on"),
        (_, "off") => Some("off"),
        _ => None,
    }
}

/// Le service qui amène l'appareil à `target`, s'il n'y est pas déjà.
#[must_use]
pub fn order(domain: &str, current: Option<&str>, target: &str) -> Option<&'static str> {
    let already = match (domain, target, current) {
        ("cover", "open", Some("open" | "opening"))
        | ("cover", "closed", Some("closed" | "closing")) => true,
        (_, _, Some(now)) => now == target,
        (_, _, None) => false,
    };
    if already {
        return None;
    }
    Some(match (domain, target) {
        ("cover", "open") => "open_cover",
        ("cover", _) => "close_cover",
        (_, "on") => "turn_on",
        _ => "turn_off",
    })
}

/// Personne n'est à la maison ? Sans `zone.home` (pas de personnes
/// configurées), la maison est considérée vide.
#[must_use]
pub fn house_empty(core: &Core) -> bool {
    let Ok(home) = "zone.home".parse::<EntityId>() else {
        return true;
    };
    core.states
        .get(&home)
        .is_none_or(|zone| zone.state().trim() == "0")
}

/// Le service.
#[derive(Debug)]
pub struct Simulation(Background);

impl Simulation {
    /// Démarre la simulation (désarmée, sauf si elle était armée avant
    /// l'arrêt du serveur).
    #[must_use]
    pub fn start(config: Config, store: Store, core: Arc<Core>) -> Self {
        let events = core.bus.subscribe();
        Self(Background::spawn(
            "la simulation de présence",
            move |token| run(config, store, core, events, token),
        ))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

fn publish(core: &Core, config: &Config, armed: bool, replaying: bool) {
    let Ok(entity_id) = SWITCH.parse::<EntityId>() else {
        return;
    };
    let attributes: Attributes = match json!({
        "friendly_name": NAME,
        "icon": "mdi:home-account",
        "en_cours": replaying,
        "appareils": config.devices.iter().map(EntityId::as_str).collect::<Vec<_>>(),
        "il_y_a": config.days_ago,
    }) {
        Value::Object(map) => map,
        _ => Attributes::new(),
    };
    if let Err(error) = core
        .states
        .set(entity_id, if armed { "on" } else { "off" }, attributes)
    {
        warn!("simulation de présence : état refusé ({error})");
    }
}

async fn run(
    config: Config,
    store: Store,
    core: Arc<Core>,
    mut events: tokio::sync::broadcast::Receiver<Arc<Event>>,
    token: CancellationToken,
) {
    let switch: Option<EntityId> = SWITCH.parse().ok();
    // Armée avant l'arrêt du serveur ? (l'état est restauré au démarrage)
    let mut armed = switch
        .as_ref()
        .and_then(|id| core.states.get(id))
        .is_some_and(|state| state.state() == "on");
    let mut replaying = false;
    // Le dernier état rejoué de chaque appareil : on n'agit que quand le
    // passé change, pour ne pas contrarier un geste fait entre-temps.
    let mut last: HashMap<EntityId, &'static str> = HashMap::new();
    publish(&core, &config, armed, replaying);
    info!(
        "simulation de présence : {} appareil(s), rejoue il y a {} jours{}",
        config.devices.len(),
        config.days_ago,
        if armed { " (armée)" } else { "" }
    );
    let mut ticker = tokio::time::interval(Duration::from_mins(1));
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
                        info!("simulation de présence {}", if armed { "armée" } else { "désarmée" });
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            },
        }
        let should = armed && house_empty(&core);
        if should != replaying {
            replaying = should;
            last.clear();
            info!(
                "simulation de présence : {}",
                if replaying {
                    "la maison est vide, je rejoue la semaine passée"
                } else {
                    "arrêtée"
                }
            );
        }
        publish(&core, &config, armed, replaying);
        if !replaying {
            continue;
        }
        let past = SystemTime::now() - Duration::from_hours(24 * u64::from(config.days_ago));
        for device in &config.devices {
            let then = match store.state_at(device, past).await {
                Ok(Some(state)) => state,
                Ok(None) => continue,
                Err(error) => {
                    warn!("simulation de présence : historique illisible ({error})");
                    continue;
                }
            };
            let Some(target) = wanted(device.domain(), &then) else {
                continue;
            };
            if last.get(device) == Some(&target) {
                continue;
            }
            last.insert(device.clone(), target);
            let current = core.states.get(device);
            let Some(service) = order(
                device.domain(),
                current.as_ref().map(ha_core::State::state),
                target,
            ) else {
                continue;
            };
            if let Err(error) = core.call_service_from(
                Origin::Automation(NAME.into()),
                device.domain(),
                service,
                device.clone(),
                Attributes::new(),
            ) {
                warn!("simulation de présence : {device} refusé ({error})");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn the_config_is_checked() {
        let config = parse_config(
            "appareils:\n  - light.salon\n  - cover.salon\n  - light.salon\nil_y_a: 3\n",
        )
        .unwrap();
        assert_eq!(config.days_ago, 3);
        assert_eq!(config.devices.len(), 2, "sans doublon");
        assert_eq!(
            parse_config("appareils: [light.salon]\n").unwrap().days_ago,
            7
        );
        for bad in [
            "appareils: []\n",
            "appareils: [lock.entree]\n",
            "appareils: [light.salon]\nil_y_a: 10\n",
            "appareils: [pas un id]\n",
            "appareils: [light.salon]\ncouleur: bleu\n",
        ] {
            assert!(parse_config(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_past_is_turned_into_orders() {
        assert_eq!(wanted("light", "on"), Some("on"));
        assert_eq!(wanted("cover", "closing"), Some("closed"));
        assert_eq!(wanted("light", "unavailable"), None);
        assert_eq!(wanted("cover", "on"), None);
        assert_eq!(order("light", Some("off"), "on"), Some("turn_on"));
        assert_eq!(order("light", Some("on"), "on"), None);
        assert_eq!(order("cover", Some("open"), "closed"), Some("close_cover"));
        assert_eq!(order("cover", Some("opening"), "open"), None);
        assert_eq!(order("switch", None, "off"), Some("turn_off"));
    }

    #[tokio::test]
    async fn it_replays_only_while_the_house_is_empty() {
        let store = Store::open_in_memory().await.unwrap();
        let core = Arc::new(Core::new());
        let lamp: EntityId = "light.salon".parse().unwrap();
        core.states
            .set(lamp.clone(), "off", Attributes::new())
            .unwrap();
        let home: EntityId = "zone.home".parse().unwrap();
        core.states
            .set(home.clone(), "1", Attributes::new())
            .unwrap();
        // Il y a 7 jours, à cette heure-ci, la lampe était allumée.
        let a_week_ago = SystemTime::now() - Duration::from_hours(24 * 7);
        let at = ha_storage::to_micros(a_week_ago - Duration::from_mins(5)).unwrap();
        sqlx_insert(&store, "light.salon", "on", at).await;

        let mut events = core.bus.subscribe();
        let simulation = Simulation::start(
            Config {
                devices: vec![lamp.clone()],
                days_ago: 7,
            },
            store,
            Arc::clone(&core),
        );
        // Armée, mais quelqu'un est là : rien.
        tokio::time::sleep(Duration::from_millis(50)).await;
        core.call_service(
            "switch",
            "turn_on",
            SWITCH.parse().unwrap(),
            Attributes::new(),
        )
        .unwrap();
        let order = wait_for_order(&mut events, Duration::from_millis(300)).await;
        assert_eq!(order, None, "quelqu'un est à la maison");
        assert_eq!(
            core.states.get(&SWITCH.parse().unwrap()).unwrap().state(),
            "on"
        );

        // Tout le monde part : la lampe s'allume, comme il y a une semaine.
        core.states.set(home, "0", Attributes::new()).unwrap();
        core.call_service(
            "switch",
            "turn_on",
            SWITCH.parse().unwrap(),
            Attributes::new(),
        )
        .unwrap();
        let order = wait_for_order(&mut events, Duration::from_secs(2)).await;
        assert_eq!(
            order,
            Some((
                "turn_on".to_owned(),
                "light.salon".to_owned(),
                Origin::Automation(NAME.into())
            ))
        );
        simulation.stop().await;
    }

    async fn sqlx_insert(store: &Store, entity: &str, state: &str, at: i64) {
        sqlx::query(
            "INSERT INTO state_history (entity_id, state, attributes, last_changed, last_updated, recorded_at)
             VALUES (?1, ?2, '{}', ?3, ?3, ?3)",
        )
        .bind(entity)
        .bind(state)
        .bind(at)
        .execute(store.pool())
        .await
        .unwrap();
    }

    async fn wait_for_order(
        events: &mut tokio::sync::broadcast::Receiver<Arc<Event>>,
        within: Duration,
    ) -> Option<(String, String, Origin)> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let event = tokio::time::timeout_at(deadline, events.recv())
                .await
                .ok()?
                .ok()?;
            if let Event::CallService {
                domain,
                service,
                entity_id,
                origin,
                ..
            } = event.as_ref()
                && domain == "light"
            {
                return Some((service.clone(), entity_id.to_string(), origin.clone()));
            }
        }
    }
}
