//! **Entrées** (« helpers » dans Home Assistant) : des appareils virtuels,
//! réglés depuis le tableau de bord, et que les automatisations lisent.
//!
//! - `input_boolean.mode_vacances` : un interrupteur (`on` / `off`) ;
//! - `input_number.consigne_chambre` : un nombre, avec un curseur ;
//! - `input_select.mode_maison` : un choix dans une liste ;
//! - `input_button.test_sirene` : un bouton (son état : la date du dernier appui) ;
//! - `timer.lessive` : un minuteur, qui publie l'événement `timer.finished`
//!   quand il arrive à zéro.
//!
//! Leurs valeurs sont gardées après un redémarrage. Un minuteur qui aurait
//! dû sonner pendant l'arrêt sonne au démarrage.

mod config;
mod helper;

use std::collections::BTreeMap;
use std::sync::Arc;

use ha_core::{Attributes, Background, CancellationToken, Core, EntityId, Event};
use jiff::Timestamp;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::Instant;
use tracing::{info, warn};

pub use config::{
    Config, Helper, InputError, Kind, Number, NumberMode, load, parse, parse_duration,
};
pub use helper::{Entry, hms};

/// Les domaines des entrées.
pub const DOMAINS: [&str; 5] = [
    "input_boolean",
    "input_number",
    "input_select",
    "input_button",
    "timer",
];

/// La tâche qui fait vivre les entrées.
#[derive(Debug)]
pub struct Inputs(Background);

/// Les entrées créées, pas encore en marche (voir [`Inputs::prepare`]).
#[derive(Debug)]
pub struct Prepared {
    entries: BTreeMap<EntityId, Entry>,
    /// Les minuteurs qui ont sonné pendant l'arrêt du serveur.
    finished: Vec<EntityId>,
}

impl Inputs {
    /// Publie les entrées, avec leur valeur d'avant l'arrêt.
    ///
    /// À appeler **avant** de démarrer les automatisations : la création
    /// d'une entrée n'est pas un changement (sinon, au tout premier
    /// démarrage, un bouton qui passe de « rien » à « jamais appuyé »
    /// lancerait ses automatisations). Puis [`Prepared::start`] **après**,
    /// pour qu'elles reçoivent un `timer.finished` arrivé pendant l'arrêt.
    #[must_use]
    pub fn prepare(config: Config, core: &Core) -> Prepared {
        let (now, wall) = (Instant::now(), Timestamp::now());
        let mut entries = BTreeMap::new();
        let mut finished = Vec::new();
        for helper in config.helpers {
            let restored = core.states.get(&helper.entity_id);
            let (entry, finished_while_down) = Entry::new(helper, restored.as_ref(), now, wall);
            let entity_id = entry.helper.entity_id.clone();
            publish(core, &entry, now);
            if finished_while_down {
                finished.push(entity_id.clone());
            }
            entries.insert(entity_id, entry);
        }
        Prepared { entries, finished }
    }

    /// [`Inputs::prepare`] puis [`Prepared::start`], d'un coup.
    pub fn start(config: Config, core: Arc<Core>) -> Self {
        Self::prepare(config, &core).start(core)
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

impl Prepared {
    /// Fait sonner les minuteurs en retard, puis répond aux appels de service.
    pub fn start(self, core: Arc<Core>) -> Inputs {
        let events = core.bus.subscribe();
        for entity_id in &self.finished {
            info!("{entity_id} a sonné pendant l'arrêt du serveur");
            fire(&core, "timer.finished", entity_id);
        }
        let entries = self.entries;
        Inputs(Background::spawn("les entrées", move |token| {
            run(entries, core, events, token)
        }))
    }
}

async fn run(
    mut entries: BTreeMap<EntityId, Entry>,
    core: Arc<Core>,
    mut events: tokio::sync::broadcast::Receiver<Arc<Event>>,
    token: CancellationToken,
) {
    info!("{} entrée(s)", entries.len());
    loop {
        // Le prochain minuteur qui doit sonner, s'il y en a un en marche.
        let next = entries.values().filter_map(Entry::deadline).min();
        tokio::select! {
            () = token.cancelled() => break,
            () = sleep_until(next) => {
                let now = Instant::now();
                for entry in entries.values_mut() {
                    if entry.expire(now) {
                        publish(&core, entry, now);
                        fire(&core, "timer.finished", &entry.helper.entity_id);
                    }
                }
            }
            received = events.recv() => match received {
                Ok(event) => {
                    if let Event::CallService { domain, service, entity_id, data } = event.as_ref()
                        && domain == entity_id.domain()
                        && let Some(entry) = entries.get_mut(entity_id)
                    {
                        call(&core, entry, service, data);
                    }
                }
                Err(RecvError::Lagged(missed)) => warn!("entrées : {missed} événements manqués"),
                Err(RecvError::Closed) => break,
            },
        }
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

fn call(core: &Core, entry: &mut Entry, service: &str, data: &Attributes) {
    let now = Instant::now();
    match entry.call(service, data, now, Timestamp::now()) {
        Ok(fired) => {
            publish(core, entry, now);
            if let Some(event_type) = fired {
                fire(core, event_type, &entry.helper.entity_id);
            }
        }
        Err(reason) => warn!("{} : {service} refusé ({reason})", entry.helper.entity_id),
    }
}

fn publish(core: &Core, entry: &Entry, now: Instant) {
    let (state, attributes) = entry.render(now);
    if let Err(error) = core
        .states
        .set(entry.helper.entity_id.clone(), state, attributes)
    {
        warn!("entrées : état refusé ({error})");
    }
}

/// Publie `timer.finished` (etc.) avec `{"entity_id": "timer.lessive"}`.
fn fire(core: &Core, event_type: &str, entity_id: &EntityId) {
    let mut data = Attributes::new();
    data.insert("entity_id".into(), entity_id.as_str().into());
    if let Err(error) = core.fire(event_type, data) {
        warn!("entrées : événement refusé ({error})");
    }
}
