//! **Journal de bord** : ce qui s'est passé dans la maison, en phrases, et
//! pourquoi.
//!
//! ```text
//! 22:30  Bonne nuit        automatisation lancée   déclencheur : il est 22:30
//! 22:30  Lumière du salon  s'est éteint            par l'automatisation « Bonne nuit »
//! 22:41  Porte d'entrée    s'est ouvert
//! ```
//!
//! La cause d'un changement vient de la **dernière commande** envoyée au même
//! appareil (par une personne, une automatisation, Alexa…) dans les 10
//! secondes. Une commande n'explique qu'**un** changement : une lumière
//! allumée par David qui s'éteint toute seule ensuite n'est pas « par David ».
//! Un appareil qui change tout seul, ou un minuteur qui arrive à zéro, n'a
//! pas de cause.

mod describe;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use ha_core::{Background, CancellationToken, Core, EntityId, Event, Origin, StateMachine};
use ha_storage::{NewLogbookEntry, Store};
use serde_json::Value;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::Instant;
use tracing::{error, info, warn};

pub use describe::{describe_change, describe_timer_event, name_of};

/// Une commande compte comme cause d'un changement pendant ce temps.
pub const CAUSE_WINDOW: Duration = Duration::from_secs(10);
/// L'événement d'une automatisation lancée (voir `ha-automation`).
const AUTOMATION_TRIGGERED: &str = "automation_triggered";
/// Commandes récentes gardées en mémoire, au plus.
const MAX_RECENT: usize = 256;

/// Une ligne du journal, avant d'être écrite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub entity_id: Option<EntityId>,
    pub name: String,
    pub message: String,
    pub cause: Option<String>,
}

/// Ce dont le journal se souvient entre deux événements : la dernière
/// commande envoyée à chaque appareil.
#[derive(Debug, Default)]
pub struct Recent(HashMap<EntityId, (Instant, Origin)>);

impl Recent {
    fn remember(&mut self, entity_id: &EntityId, origin: &Origin, now: Instant) {
        if self.0.len() >= MAX_RECENT {
            self.0
                .retain(|_, (at, _)| now.duration_since(*at) <= CAUSE_WINDOW);
        }
        self.0.insert(entity_id.clone(), (now, origin.clone()));
    }

    /// La cause d'un changement, qui est ensuite oubliée.
    fn take_cause(&mut self, entity_id: &EntityId, now: Instant) -> Option<String> {
        let (at, origin) = self.0.remove(entity_id)?;
        if now.duration_since(at) > CAUSE_WINDOW {
            return None;
        }
        origin.describe()
    }
}

/// La ligne à écrire pour un événement, s'il en mérite une.
pub fn line_for(
    event: &Event,
    states: &StateMachine,
    recent: &mut Recent,
    now: Instant,
) -> Option<Line> {
    match event {
        Event::CallService {
            entity_id, origin, ..
        } => {
            recent.remember(entity_id, origin, now);
            None
        }
        Event::StateChanged {
            entity_id,
            old_state,
            new_state,
        } => {
            let message = describe_change(old_state.as_ref(), new_state.as_ref())?;
            Some(Line {
                entity_id: Some(entity_id.clone()),
                name: name_of(entity_id, new_state.as_ref()),
                message,
                cause: recent.take_cause(entity_id, now),
            })
        }
        Event::Fired { event_type, data } if event_type == AUTOMATION_TRIGGERED => {
            let text = |key: &str| data.get(key).and_then(Value::as_str);
            Some(Line {
                entity_id: None,
                name: text("name")?.to_owned(),
                message: "automatisation lancée".into(),
                cause: text("trigger").map(|why| format!("déclencheur : {why}")),
            })
        }
        Event::Fired { event_type, data } => {
            let message = describe_timer_event(event_type)?;
            let entity_id: EntityId = data.get("entity_id")?.as_str()?.parse().ok()?;
            // Arriver à zéro n'est la faute de personne.
            let cause = if event_type == "timer.finished" {
                None
            } else {
                recent.take_cause(&entity_id, now)
            };
            Some(Line {
                name: name_of(&entity_id, states.get(&entity_id).as_ref()),
                message: message.into(),
                cause,
                entity_id: Some(entity_id),
            })
        }
    }
}

/// La tâche qui écrit le journal de bord.
#[derive(Debug)]
pub struct Logbook(Background);

impl Logbook {
    pub fn start(store: Store, core: Arc<Core>) -> Self {
        let events = core.bus.subscribe();
        Self(Background::spawn("le journal de bord", move |token| {
            run(store, core, events, token)
        }))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

async fn run(
    store: Store,
    core: Arc<Core>,
    mut events: tokio::sync::broadcast::Receiver<Arc<Event>>,
    token: CancellationToken,
) {
    info!("journal de bord en marche");
    let mut recent = Recent::default();
    loop {
        let event = tokio::select! {
            () = token.cancelled() => break,
            received = events.recv() => match received {
                Ok(event) => event,
                Err(RecvError::Lagged(missed)) => {
                    warn!("journal de bord : {missed} événements manqués");
                    continue;
                }
                Err(RecvError::Closed) => break,
            },
        };
        let Some(line) = line_for(&event, &core.states, &mut recent, Instant::now()) else {
            continue;
        };
        let entry = NewLogbookEntry {
            entity_id: line.entity_id.as_ref(),
            name: &line.name,
            message: &line.message,
            cause: line.cause.as_deref(),
        };
        // Un disque plein ne doit pas arrêter le journal : on le signale.
        if let Err(failure) = store.add_logbook(entry).await {
            error!("journal de bord : écriture impossible ({failure})");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ha_core::{Attributes, State};
    use serde_json::json;

    fn id(value: &str) -> EntityId {
        value.parse().unwrap()
    }

    fn state(core: &Core, entity: &str, value: &str) -> State {
        let mut attributes = Attributes::new();
        attributes.insert("friendly_name".into(), "Salon".into());
        core.states.set(id(entity), value, attributes).unwrap();
        core.states.get(&id(entity)).unwrap()
    }

    fn changed(core: &Core, from: &str, to: &str) -> Event {
        Event::StateChanged {
            entity_id: id("light.salon"),
            old_state: Some(state(core, "light.salon", from)),
            new_state: Some(state(core, "light.salon", to)),
        }
    }

    fn call(origin: Origin) -> Event {
        Event::CallService {
            domain: "light".into(),
            service: "turn_on".into(),
            entity_id: id("light.salon"),
            data: Attributes::new(),
            origin,
        }
    }

    #[test]
    fn a_change_gets_the_cause_of_the_last_command() {
        let core = Core::new();
        let mut recent = Recent::default();
        let start = Instant::now();
        let line = |recent: &mut Recent, event: &Event, at: Instant| {
            line_for(event, &core.states, recent, at)
        };

        // Sans commande : pas de cause.
        let spontaneous = line(&mut recent, &changed(&core, "off", "on"), start).unwrap();
        assert_eq!(
            (
                spontaneous.name.as_str(),
                spontaneous.message.as_str(),
                spontaneous.cause
            ),
            ("Salon", "s'est allumé", None)
        );

        assert_eq!(
            line(&mut recent, &call(Origin::User("david".into())), start),
            None
        );
        let by_david = line(
            &mut recent,
            &changed(&core, "on", "off"),
            start + Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(by_david.cause.as_deref(), Some("par david"));
        // Une commande n'explique qu'un changement.
        let after = line(&mut recent, &changed(&core, "off", "on"), start).unwrap();
        assert_eq!(after.cause, None);

        // Une commande plus récente remplace la précédente…
        line(
            &mut recent,
            &call(Origin::Automation("Bonne nuit".into())),
            start,
        );
        let by_automation = line(&mut recent, &changed(&core, "off", "on"), start).unwrap();
        assert_eq!(
            by_automation.cause.as_deref(),
            Some("par l'automatisation « Bonne nuit »")
        );
        // … et elle ne compte plus après 10 secondes.
        let later = start + CAUSE_WINDOW + Duration::from_secs(1);
        assert_eq!(
            line(&mut recent, &changed(&core, "on", "off"), later)
                .unwrap()
                .cause,
            None
        );
        // Le serveur lui-même n'est pas une cause affichée.
        line(&mut recent, &call(Origin::System), later);
        assert_eq!(
            line(&mut recent, &changed(&core, "off", "on"), later)
                .unwrap()
                .cause,
            None
        );
    }

    #[test]
    fn automations_and_timers_get_a_line() {
        let core = Core::new();
        let mut recent = Recent::default();
        let now = Instant::now();
        let fired = |event_type: &str, data: Value| {
            let Value::Object(data) = data else {
                panic!("un objet est attendu")
            };
            Event::Fired {
                event_type: event_type.into(),
                data,
            }
        };

        let automation = fired(
            AUTOMATION_TRIGGERED,
            json!({"id": "bonne_nuit", "name": "Bonne nuit", "trigger": "il est 22:30"}),
        );
        assert_eq!(
            line_for(&automation, &core.states, &mut recent, now),
            Some(Line {
                entity_id: None,
                name: "Bonne nuit".into(),
                message: "automatisation lancée".into(),
                cause: Some("déclencheur : il est 22:30".into()),
            })
        );

        // David démarre le minuteur : « démarré par david », mais « terminé »
        // tout seul, même dans les 10 secondes.
        let timer: EntityId = id("timer.lessive");
        recent.remember(&timer, &Origin::User("david".into()), now);
        let started = fired("timer.started", json!({"entity_id": "timer.lessive"}));
        assert_eq!(
            line_for(&started, &core.states, &mut recent, now)
                .unwrap()
                .cause
                .as_deref(),
            Some("par david")
        );
        recent.remember(&timer, &Origin::User("david".into()), now);
        let finished = fired("timer.finished", json!({"entity_id": "timer.lessive"}));
        let line = line_for(&finished, &core.states, &mut recent, now).unwrap();
        assert_eq!(
            (line.name.as_str(), line.message.as_str(), line.cause),
            ("timer.lessive", "terminé", None)
        );
        // Un événement inconnu, ou sans appareil : rien.
        assert_eq!(
            line_for(
                &fired("autre.chose", json!({})),
                &core.states,
                &mut recent,
                now
            ),
            None
        );
        assert_eq!(
            line_for(
                &fired("timer.finished", json!({})),
                &core.states,
                &mut recent,
                now
            ),
            None
        );
    }

    #[test]
    fn old_commands_are_forgotten() {
        let mut recent = Recent::default();
        let start = Instant::now();
        for i in 0..MAX_RECENT {
            recent.remember(&id(&format!("light.l{i}")), &Origin::Voice, start);
        }
        let later = start + CAUSE_WINDOW * 2;
        recent.remember(&id("light.nouvelle"), &Origin::Voice, later);
        assert_eq!(
            recent.0.len(),
            1,
            "les commandes trop anciennes sont oubliées"
        );
    }
}
