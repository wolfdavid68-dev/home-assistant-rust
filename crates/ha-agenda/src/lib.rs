//! **Listes et agenda** de la maison, gardés chez toi (dans la base SQLite) :
//!
//! - `todo.courses` (« Liste de courses ») et `todo.taches` (« Tâches ») :
//!   leur état est le nombre d'éléments qui restent à faire ;
//! - `calendar.maison` (« Agenda de la maison ») : `on` pendant un
//!   rendez-vous, `off` sinon, comme dans Home Assistant.
//!
//! On les modifie avec les services de Home Assistant : `todo.add_item`,
//! `todo.update_item`, `todo.remove_item`, `todo.remove_completed_items`,
//! `calendar.create_event`, et `calendar.delete_event` (en plus).
//!
//! Les rendez-vous de la semaine passée et du mois à venir sont publiés dans
//! l'attribut `events` de l'agenda : le tableau de bord les affiche, et le
//! déclencheur `calendar` des automatisations s'en sert (« 30 minutes avant
//! un rendez-vous »).

mod check;

use std::sync::Arc;
use std::time::Duration;

use ha_core::{Attributes, Background, CancellationToken, Core, EntityId, Event, Origin};
use ha_storage::{CalendarEvent, NewCalendarEvent, Store, TodoItem};
use jiff::ToSpan;
use jiff::civil::DateTime;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use tokio::time::{Instant, MissedTickBehavior};
use tracing::{info, warn};

pub use check::{CheckError, MAX_DESCRIPTION, MAX_SUMMARY};

/// Les listes : identifiant et nom affiché.
pub const LISTS: [(&str, &str); 2] = [
    ("todo.courses", "Liste de courses"),
    ("todo.taches", "Tâches"),
];
/// L'agenda de la maison.
pub const CALENDAR: (&str, &str) = ("calendar.maison", "Agenda de la maison");
/// Nombre maximal d'éléments par liste.
pub const MAX_ITEMS: i64 = 500;
/// Nombre maximal de rendez-vous à venir.
pub const MAX_FUTURE_EVENTS: i64 = 2000;
/// Les rendez-vous publiés : depuis 7 jours, jusque dans 31 jours, 200 au plus.
/// (Un déclencheur `calendar` peut donc avoir un décalage d'une semaine.)
const PAST_DAYS: i64 = 7;
const FUTURE_DAYS: i64 = 31;
const PUBLISHED_EVENTS: u32 = 200;
/// Relecture de l'agenda dans la base (un rendez-vous entre dans la fenêtre).
const RELOAD_EVERY: Duration = Duration::from_mins(10);

/// La tâche qui fait vivre les listes et l'agenda.
#[derive(Debug)]
pub struct Agenda(Background);

impl Agenda {
    /// Publie les listes et l'agenda, puis répond aux appels de service.
    pub async fn start(store: Store, core: Arc<Core>) -> Self {
        Self::start_without(store, core, &[]).await
    }

    /// Comme [`Agenda::start`], sans les listes `without` : une autre
    /// intégration les tient (la liste de courses d'Iglou, étape 81).
    pub async fn start_without(store: Store, core: Arc<Core>, without: &[&str]) -> Self {
        let events = core.bus.subscribe();
        let mut live = Live {
            store,
            core,
            lists: LISTS
                .into_iter()
                .filter(|(list, _)| !without.contains(list))
                .collect(),
            window: Vec::new(),
            loaded_at: None,
            published: None,
        };
        for (list, _) in live.lists.clone() {
            live.publish_list(list).await;
        }
        live.refresh_calendar(true).await;
        info!("listes et agenda prêts");
        Self(Background::spawn("l'agenda", move |token| {
            run(live, events, token)
        }))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

/// Les domaines gérés ici.
pub const DOMAINS: [&str; 2] = ["todo", "calendar"];

struct Live {
    store: Store,
    core: Arc<Core>,
    /// Les listes tenues ici.
    lists: Vec<(&'static str, &'static str)>,
    /// Les rendez-vous de la fenêtre publiée, relus de temps en temps.
    window: Vec<CalendarEvent>,
    loaded_at: Option<Instant>,
    /// Ce qui a été publié pour l'agenda (pour ne republier qu'en cas de changement).
    published: Option<(String, Attributes)>,
}

async fn run(
    mut live: Live,
    mut events: tokio::sync::broadcast::Receiver<Arc<Event>>,
    token: CancellationToken,
) {
    // Toutes les 15 secondes : un rendez-vous commence ou se termine.
    let mut ticker = tokio::time::interval(Duration::from_secs(15));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            () = token.cancelled() => break,
            _ = ticker.tick() => live.refresh_calendar(false).await,
            received = events.recv() => match received {
                Ok(event) => {
                    if let Event::CallService { domain, service, entity_id, data, origin, .. } = event.as_ref()
                        && domain == entity_id.domain()
                        && DOMAINS.contains(&domain.as_str())
                    {
                        live.call(service, entity_id, data, origin).await;
                    }
                }
                Err(RecvError::Lagged(missed)) => warn!("agenda : {missed} événements manqués"),
                Err(RecvError::Closed) => break,
            },
        }
    }
}

impl Live {
    async fn call(
        &mut self,
        service: &str,
        entity_id: &EntityId,
        data: &Attributes,
        origin: &Origin,
    ) {
        let id = entity_id.as_str();
        let result = if self.lists.iter().any(|(list, _)| *list == id) {
            self.call_list(id, service, data).await
        } else if id == CALENDAR.0 {
            self.call_calendar(service, data, origin).await
        } else {
            return; // Une autre intégration s'en occupe.
        };
        match result {
            Ok(()) => {}
            Err(Refused::Check(error)) => warn!("{id} : {service} refusé ({error})"),
            Err(Refused::Other(reason)) => warn!("{id} : {service} refusé ({reason})"),
            Err(Refused::Storage(error)) => warn!("{id} : {service} impossible ({error})"),
        }
    }

    async fn call_list(&self, list: &str, service: &str, data: &Attributes) -> Result<(), Refused> {
        match service {
            "add_item" => {
                let summary = check::required_text(data, "item", MAX_SUMMARY)?;
                let due = check::due(data)?;
                if self.store.count_todo_items(list).await? >= MAX_ITEMS {
                    return Err(Refused::Other(format!(
                        "la liste a déjà {MAX_ITEMS} éléments"
                    )));
                }
                self.store.add_todo_item(list, &summary, due.date()).await?;
            }
            "update_item" => {
                let item = self.find_item(list, data).await?;
                let rename = check::optional_text(data, "rename", MAX_SUMMARY)?;
                let done = check::status(data)?;
                let due = check::due(data)?;
                if rename.is_none() && done.is_none() && due == check::Due::Keep {
                    return Err(Refused::Check(CheckError::Missing("status")));
                }
                self.store
                    .update_todo_item(list, item.id, rename.as_deref(), done, due.as_change())
                    .await?;
            }
            "remove_item" => {
                let item = self.find_item(list, data).await?;
                self.store.remove_todo_item(list, item.id).await?;
            }
            "remove_completed_items" => {
                self.store.remove_done_todo_items(list).await?;
            }
            _ => return Err(Refused::Other("service inconnu".into())),
        }
        self.publish_list(list).await;
        Ok(())
    }

    /// L'élément désigné par `item` : son numéro (`uid`), sinon son texte
    /// (le premier qui reste à faire, comme dans Home Assistant).
    async fn find_item(&self, list: &str, data: &Attributes) -> Result<TodoItem, Refused> {
        let items = self.store.todo_items(list).await?;
        let wanted = data.get("item").ok_or(CheckError::Missing("item"))?;
        let by_id = wanted
            .as_i64()
            .or_else(|| wanted.as_str().and_then(|text| text.trim().parse().ok()));
        let found = by_id
            .and_then(|id| items.iter().find(|item| item.id == id))
            .or_else(|| {
                let text = wanted.as_str()?.trim();
                items.iter().find(|item| item.summary == text)
            });
        found
            .cloned()
            .ok_or_else(|| Refused::Other("élément introuvable".into()))
    }

    async fn call_calendar(
        &mut self,
        service: &str,
        data: &Attributes,
        origin: &Origin,
    ) -> Result<(), Refused> {
        let calendar = CALENDAR.0;
        match service {
            "create_event" => {
                let summary = check::required_text(data, "summary", MAX_SUMMARY)?;
                let description = check::description(data)?;
                let (start, end, all_day) = check::event_period(data)?;
                let now = check::stored(local_now());
                if self
                    .store
                    .count_future_calendar_events(calendar, &now)
                    .await?
                    >= MAX_FUTURE_EVENTS
                {
                    return Err(Refused::Other(format!(
                        "l'agenda a déjà {MAX_FUTURE_EVENTS} rendez-vous à venir"
                    )));
                }
                let created_by = match origin {
                    Origin::User(name) => Some(name.as_str()),
                    _ => None,
                };
                self.store
                    .add_calendar_event(
                        calendar,
                        NewCalendarEvent {
                            summary: &summary,
                            description: &description,
                            start: &check::stored(start),
                            end: &check::stored(end),
                            all_day,
                            created_by,
                        },
                    )
                    .await?;
            }
            "delete_event" => {
                let uid = data
                    .get("uid")
                    .and_then(|uid| uid.as_i64().or_else(|| uid.as_str()?.trim().parse().ok()))
                    .ok_or(CheckError::Missing("uid"))?;
                if !self.store.delete_calendar_event(calendar, uid).await? {
                    return Err(Refused::Other("rendez-vous introuvable".into()));
                }
            }
            _ => return Err(Refused::Other("service inconnu".into())),
        }
        self.refresh_calendar(true).await;
        Ok(())
    }

    async fn publish_list(&self, list: &str) {
        let items = match self.store.todo_items(list).await {
            Ok(items) => items,
            Err(error) => {
                warn!("{list} : lecture impossible ({error})");
                return;
            }
        };
        let name = LISTS
            .iter()
            .find(|(id, _)| *id == list)
            .map_or(list, |(_, name)| *name);
        let (state, attributes) = list_state(name, &items);
        self.set(list, state, attributes);
    }

    /// Recalcule l'état de l'agenda (relu dans la base si `reload`, ou de
    /// temps en temps), et le publie s'il a changé.
    async fn refresh_calendar(&mut self, reload: bool) {
        let now = local_now();
        let stale = self.loaded_at.is_none_or(|at| at.elapsed() >= RELOAD_EVERY);
        if reload || stale {
            let (Ok(from), Ok(until)) = (
                now.checked_sub(PAST_DAYS.days()),
                now.checked_add(FUTURE_DAYS.days()),
            ) else {
                return;
            };
            match self
                .store
                .calendar_events(
                    CALENDAR.0,
                    &check::stored(from),
                    &check::stored(until),
                    PUBLISHED_EVENTS,
                )
                .await
            {
                Ok(window) => {
                    self.window = window;
                    self.loaded_at = Some(Instant::now());
                }
                Err(error) => warn!("{} : lecture impossible ({error})", CALENDAR.0),
            }
        }
        let published = calendar_state(CALENDAR.1, &self.window, now);
        if self.published.as_ref() != Some(&published) {
            self.set(CALENDAR.0, published.0.clone(), published.1.clone());
            self.published = Some(published);
        }
    }

    fn set(&self, entity_id: &str, state: String, attributes: Attributes) {
        let Ok(id) = EntityId::new(entity_id) else {
            return;
        };
        if let Err(error) = self.core.states.set(id, state, attributes) {
            warn!("{entity_id} : état refusé ({error})");
        }
    }
}

/// Pourquoi un appel n'a rien changé.
#[derive(Debug)]
enum Refused {
    Check(CheckError),
    Other(String),
    Storage(ha_storage::StorageError),
}

impl From<CheckError> for Refused {
    fn from(error: CheckError) -> Self {
        Self::Check(error)
    }
}

impl From<ha_storage::StorageError> for Refused {
    fn from(error: ha_storage::StorageError) -> Self {
        Self::Storage(error)
    }
}

fn local_now() -> DateTime {
    jiff::Zoned::now().datetime()
}

/// L'état d'une liste : le nombre d'éléments à faire, et tous les éléments.
#[must_use]
pub fn list_state(name: &str, items: &[TodoItem]) -> (String, Attributes) {
    let left = items.iter().filter(|item| !item.done).count();
    let items: Vec<Value> = items
        .iter()
        .map(|item| {
            json!({
                "uid": item.id.to_string(),
                "summary": item.summary,
                "status": if item.done { "completed" } else { "needs_action" },
                "due": item.due,
            })
        })
        .collect();
    let mut attributes = Attributes::new();
    attributes.insert("friendly_name".into(), json!(name));
    attributes.insert("items".into(), Value::Array(items));
    (left.to_string(), attributes)
}

/// L'état de l'agenda à l'instant `now`, comme dans Home Assistant : `on`
/// pendant un rendez-vous ; les attributs `message`, `start_time`… décrivent
/// le rendez-vous en cours, sinon le prochain. Plus la liste `events`.
#[must_use]
pub fn calendar_state(name: &str, window: &[CalendarEvent], now: DateTime) -> (String, Attributes) {
    let now_text = check::stored(now);
    let current = window
        .iter()
        .find(|e| e.start <= now_text && now_text < e.end);
    let shown = current.or_else(|| window.iter().find(|e| e.start > now_text));
    let mut attributes = Attributes::new();
    attributes.insert("friendly_name".into(), json!(name));
    if let Some(event) = shown {
        attributes.insert("message".into(), json!(event.summary));
        attributes.insert("description".into(), json!(event.description));
        attributes.insert("all_day".into(), json!(event.all_day));
        attributes.insert(
            "start_time".into(),
            json!(event.start.replacen('T', " ", 1)),
        );
        attributes.insert("end_time".into(), json!(event.end.replacen('T', " ", 1)));
    }
    let events: Vec<Value> = window
        .iter()
        .map(|e| {
            let mut event = json!({
                "uid": e.id.to_string(),
                "summary": e.summary,
                "description": e.description,
                "start": e.start,
                "end": e.end,
                "all_day": e.all_day,
                "created_by": e.created_by,
            });
            // Les détails d'un agenda en ligne (TimeTree), seulement s'il y en a.
            if let Value::Object(map) = &mut event {
                let extra = &e.extra;
                for (key, value) in [
                    ("location", &extra.location),
                    ("label", &extra.label),
                    ("color", &extra.color),
                ] {
                    if !value.is_empty() {
                        map.insert(key.into(), json!(value));
                    }
                }
                if !extra.attendees.is_empty() {
                    map.insert("attendees".into(), json!(extra.attendees));
                }
            }
            event
        })
        .collect();
    attributes.insert("events".into(), Value::Array(events));
    let state = if current.is_some() { "on" } else { "off" };
    (state.to_owned(), attributes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: i64, summary: &str, start: &str, end: &str) -> CalendarEvent {
        CalendarEvent {
            id,
            summary: summary.into(),
            description: String::new(),
            start: start.into(),
            end: end.into(),
            all_day: false,
            created_by: None,
            extra: ha_storage::EventExtra::default(),
        }
    }

    #[test]
    fn calendar_is_on_during_an_event() {
        let window = [
            event(1, "Dentiste", "2026-10-15T18:30:00", "2026-10-15T19:00:00"),
            event(2, "Vacances", "2026-10-20T00:00:00", "2026-10-27T00:00:00"),
        ];
        let at = |text: &str| -> DateTime { text.parse().unwrap() };

        let (state, attributes) = calendar_state("Agenda", &window, at("2026-10-15T18:00:00"));
        assert_eq!(state, "off");
        // Pas encore commencé : on montre le prochain.
        assert_eq!(attributes["message"], "Dentiste");
        assert_eq!(attributes["start_time"], "2026-10-15 18:30:00");

        let (state, attributes) = calendar_state("Agenda", &window, at("2026-10-15T18:45:00"));
        assert_eq!(state, "on");
        assert_eq!(attributes["message"], "Dentiste");

        // À la fin exacte, c'est fini (la fin est exclue).
        let (state, attributes) = calendar_state("Agenda", &window, at("2026-10-15T19:00:00"));
        assert_eq!(state, "off");
        assert_eq!(attributes["message"], "Vacances");
        assert_eq!(attributes["events"].as_array().unwrap().len(), 2);

        let (state, attributes) = calendar_state("Agenda", &[], at("2026-10-15T19:00:00"));
        assert_eq!(state, "off");
        assert!(!attributes.contains_key("message"));
    }

    #[test]
    fn list_counts_what_is_left() {
        let items = [
            TodoItem {
                id: 1,
                summary: "Lait".into(),
                done: false,
                due: None,
            },
            TodoItem {
                id: 2,
                summary: "Pain".into(),
                done: true,
                due: Some("2026-10-20".into()),
            },
        ];
        let (state, attributes) = list_state("Courses", &items);
        assert_eq!(state, "1");
        assert_eq!(attributes["items"][1]["status"], "completed");
        assert_eq!(attributes["items"][0]["uid"], "1");
    }
}
