//! **Assistants calculés** : des entités qui se déduisent d'autres entités,
//! comme les « helpers » de Home Assistant.
//!
//! - **groupe** (`light.lumieres_du_bas`) : allumé si un membre l'est ; une
//!   commande sur le groupe est transmise à chaque membre ;
//! - **plage horaire** (`schedule.chauffage_matin`) : `on` pendant les plages
//!   de la semaine ;
//! - **min / max / moyenne** (`sensor.temperature_moyenne`) de plusieurs capteurs ;
//! - **seuil** (`binary_sensor.trop_humide`) : au-dessus ou au-dessous d'une
//!   limite, avec une marge (hystérésis) pour ne pas clignoter ;
//! - **consommation par période** (`sensor.electricite_du_jour`) : ce qu'un
//!   compteur a compté depuis minuit, le début de la semaine, du mois… ;
//! - **statistique d'historique** (`sensor.chauffage_du_jour`) : combien de
//!   temps, ou combien de fois, une entité a été dans un état depuis le
//!   début de la période (étape 59, voir [`history`]).

mod compute;
mod config;
pub mod history;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use ha_core::{Attributes, Background, CancellationToken, Core, EntityId, Event, Origin, State};
use jiff::civil::DateTime;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use tokio::time::MissedTickBehavior;
use tracing::{info, warn};

pub use compute::{
    Meter, Position, cover_group, on_off_group, period_start, rounded, schedule, statistic,
    threshold,
};
pub use config::{
    Block, Cycle, Derived, DerivedError, GROUP_TYPES, Kind, Measure, Statistic, load, parse,
};

/// La tâche des assistants calculés.
#[derive(Debug)]
pub struct Assistants(Background);

impl Assistants {
    /// Publie les assistants (les compteurs reprennent leur valeur d'avant
    /// l'arrêt), puis les tient à jour.
    #[must_use]
    pub fn start(derived: Vec<Derived>, core: Arc<Core>) -> Self {
        // Les statistiques d'historique lisent la base : leur propre tâche
        // (`history::HistoryStats`).
        let derived: Vec<Derived> = derived
            .into_iter()
            .filter(|d| !matches!(d.kind, Kind::HistoryStats { .. }))
            .collect();
        let events = core.bus.subscribe();
        let mut live = Live::new(derived, &core);
        live.refresh_all(&core, local_now());
        info!("{} assistant(s) calculé(s)", live.items.len());
        Self(Background::spawn("les assistants calculés", move |token| {
            run(live, core, events, token)
        }))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

/// Un assistant et ce dont il se souvient.
struct Item {
    derived: Derived,
    /// Le seuil : son dernier état (pour l'hystérésis).
    threshold: Option<(bool, Position)>,
    /// Le compteur de consommation.
    meter: Option<Meter>,
    /// Ce qui a été publié la dernière fois (pour ne republier qu'en cas de changement).
    published: Option<(String, Attributes)>,
}

struct Live {
    items: Vec<Item>,
    /// Entité source → assistants qui en dépendent.
    watchers: BTreeMap<EntityId, Vec<usize>>,
}

impl Live {
    fn new(derived: Vec<Derived>, core: &Core) -> Self {
        let now = local_now();
        let mut watchers: BTreeMap<EntityId, Vec<usize>> = BTreeMap::new();
        let items = derived
            .into_iter()
            .enumerate()
            .map(|(index, derived)| {
                for source in derived.sources() {
                    watchers.entry(source.clone()).or_default().push(index);
                }
                let meter = match &derived.kind {
                    Kind::Meter { cycle, .. } => Some(restore_meter(
                        core.states.get(&derived.entity_id).as_ref(),
                        *cycle,
                        now,
                    )),
                    _ => None,
                };
                Item {
                    derived,
                    threshold: None,
                    meter,
                    published: None,
                }
            })
            .collect();
        Self { items, watchers }
    }

    fn refresh_all(&mut self, core: &Core, now: DateTime) {
        for index in 0..self.items.len() {
            self.refresh(index, core, now, None);
        }
    }

    /// Recalcule un assistant et le publie s'il a changé. `fed` : la
    /// nouvelle valeur de la source d'un compteur de consommation.
    fn refresh(&mut self, index: usize, core: &Core, now: DateTime, fed: Option<&State>) {
        let Some(item) = self.items.get_mut(index) else {
            return;
        };
        let read = |ids: &[EntityId]| -> Vec<Option<State>> {
            ids.iter().map(|id| core.states.get(id)).collect()
        };
        let mut attributes = Attributes::new();
        attributes.insert("friendly_name".into(), json!(item.derived.name));
        let state: String = match &item.derived.kind {
            Kind::HistoryStats { .. } => return,
            Kind::Group { members, all } => {
                attributes.insert(
                    "entity_id".into(),
                    json!(members.iter().map(EntityId::as_str).collect::<Vec<_>>()),
                );
                let states = read(members);
                if item.derived.entity_id.domain() == "cover" {
                    match cover_group(&states) {
                        Some((state, position)) => {
                            if let Some(position) = position {
                                attributes.insert("current_position".into(), json!(position));
                            }
                            state.to_owned()
                        }
                        None => "unavailable".to_owned(),
                    }
                } else {
                    match on_off_group(&states, *all) {
                        Some(on) => if on { "on" } else { "off" }.to_owned(),
                        None => "unavailable".to_owned(),
                    }
                }
            }
            Kind::Schedule { week } => {
                let (on, next) = schedule(week, now);
                if let Some(next) = next {
                    attributes.insert(
                        "next_event".into(),
                        json!(next.strftime("%Y-%m-%dT%H:%M:%S").to_string()),
                    );
                }
                if on { "on" } else { "off" }.to_owned()
            }
            Kind::MinMax {
                statistic: kind,
                sources,
                round_digits,
            } => {
                let states = read(sources);
                copy_unit(&states, &mut attributes);
                attributes.insert("type".into(), json!(statistic_name(*kind)));
                attributes.insert("count_sensors".into(), json!(sources.len()));
                statistic(*kind, &states).map_or_else(
                    || "unknown".to_owned(),
                    |value| rounded(value, *round_digits),
                )
            }
            Kind::Threshold {
                source,
                lower,
                upper,
                hysteresis,
            } => {
                attributes.insert("entity_id".into(), json!(source.as_str()));
                for (key, value) in [("lower", lower), ("upper", upper)] {
                    if let Some(value) = value {
                        attributes.insert(key.into(), json!(value));
                    }
                }
                attributes.insert("hysteresis".into(), json!(hysteresis));
                let value = core
                    .states
                    .get(source)
                    .and_then(|s| s.state().trim().parse::<f64>().ok())
                    .filter(|v| v.is_finite());
                match value {
                    Some(value) => {
                        let result = threshold(item.threshold, value, *lower, *upper, *hysteresis);
                        item.threshold = Some(result);
                        attributes.insert("sensor_value".into(), json!(value));
                        attributes.insert("position".into(), json!(result.1.as_str()));
                        if result.0 { "on" } else { "off" }.to_owned()
                    }
                    None => "unknown".to_owned(),
                }
            }
            Kind::Meter { source, cycle } => {
                let Some(meter) = item.meter.as_mut() else {
                    return;
                };
                if let Some(value) = fed
                    .filter(|s| !matches!(s.state(), "unavailable" | "unknown"))
                    .and_then(|s| s.state().trim().parse::<f64>().ok())
                {
                    meter.feed(value);
                }
                meter.roll(*cycle, now);
                let source_state = core.states.get(source);
                copy_unit(std::slice::from_ref(&source_state), &mut attributes);
                if let Some(class) = source_state
                    .as_ref()
                    .and_then(|s| s.attributes().get("device_class").cloned())
                {
                    attributes.insert("device_class".into(), class);
                }
                attributes.insert("source".into(), json!(source.as_str()));
                attributes.insert("cycle".into(), json!(cycle.as_str()));
                attributes.insert(
                    "last_reset".into(),
                    json!(meter.period.strftime("%Y-%m-%dT%H:%M:%S").to_string()),
                );
                if let Some(last) = meter.last_period {
                    attributes.insert("last_period".into(), json!(rounded(last, 3)));
                }
                if let Some(last) = meter.last_source {
                    attributes.insert("last_valid_state".into(), json!(last));
                }
                rounded(meter.value, 3)
            }
        };
        let published = (state, attributes);
        if item.published.as_ref() == Some(&published) {
            return;
        }
        if let Err(error) = core.states.set(
            item.derived.entity_id.clone(),
            published.0.clone(),
            published.1.clone(),
        ) {
            warn!("{} : état refusé ({error})", item.derived.entity_id);
        }
        item.published = Some(published);
    }

    /// Une commande sur un groupe : elle est transmise à chaque membre, au
    /// nom de la même personne (le journal de bord et les droits suivent).
    fn forward(
        &self,
        core: &Core,
        entity_id: &EntityId,
        service: &str,
        data: &Attributes,
        origin: &Origin,
    ) {
        let Some(item) = self
            .items
            .iter()
            .find(|i| &i.derived.entity_id == entity_id)
        else {
            return;
        };
        let Kind::Group { members, .. } = &item.derived.kind else {
            return;
        };
        let domain = entity_id.domain();
        // « Inverser » un groupe : tout allumer s'il est éteint, tout éteindre sinon.
        let service = match (
            service,
            item.published.as_ref().map(|(state, _)| state.as_str()),
        ) {
            ("toggle", Some("on" | "open")) => {
                if domain == "cover" {
                    "close_cover"
                } else {
                    "turn_off"
                }
            }
            ("toggle", _) => {
                if domain == "cover" {
                    "open_cover"
                } else {
                    "turn_on"
                }
            }
            (service, _) => service,
        };
        let allowed: &[&str] = match domain {
            "cover" => &[
                "open_cover",
                "close_cover",
                "stop_cover",
                "set_cover_position",
            ],
            "binary_sensor" => &[],
            _ => &["turn_on", "turn_off", "set_percentage", "set_preset_mode"],
        };
        if !allowed.contains(&service) {
            warn!("{entity_id} : {service} non transmis aux membres");
            return;
        }
        for member in members {
            if let Err(error) = core.call_service_from(
                origin.clone(),
                member.domain(),
                service,
                member.clone(),
                data.clone(),
            ) {
                warn!("{entity_id} : {service} vers {member} refusé ({error})");
            }
        }
    }
}

async fn run(
    mut live: Live,
    core: Arc<Core>,
    mut events: tokio::sync::broadcast::Receiver<Arc<Event>>,
    token: CancellationToken,
) {
    // Les plages horaires et les débuts de période : vérifiés toutes les 5 s.
    let mut ticker = tokio::time::interval(Duration::from_secs(5));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            () = token.cancelled() => break,
            _ = ticker.tick() => {
                let now = local_now();
                for index in 0..live.items.len() {
                    if matches!(live.items[index].derived.kind, Kind::Schedule { .. } | Kind::Meter { .. }) {
                        live.refresh(index, &core, now, None);
                    }
                }
            }
            received = events.recv() => match received {
                Ok(event) => match event.as_ref() {
                    Event::StateChanged { entity_id, new_state, .. } => {
                        let indexes = live.watchers.get(entity_id).cloned().unwrap_or_default();
                        let now = local_now();
                        for index in indexes {
                            live.refresh(index, &core, now, new_state.as_ref());
                        }
                    }
                    Event::CallService { domain, service, entity_id, data, origin, .. } if domain == entity_id.domain() => {
                        live.forward(&core, entity_id, service, data, origin);
                    }
                    _ => {}
                },
                Err(RecvError::Lagged(missed)) => warn!("assistants : {missed} événements manqués"),
                Err(RecvError::Closed) => break,
            },
        }
    }
}

/// Le compteur de consommation, tel qu'il était avant l'arrêt du serveur.
fn restore_meter(restored: Option<&State>, cycle: Cycle, now: DateTime) -> Meter {
    let number =
        |value: Option<&Value>| value.and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()));
    let current = period_start(cycle, now);
    let Some(restored) = restored else {
        return Meter {
            value: 0.0,
            last_period: None,
            period: current,
            last_source: None,
        };
    };
    let attributes = restored.attributes();
    let mut meter = Meter {
        value: restored
            .state()
            .parse()
            .ok()
            .filter(|v: &f64| v.is_finite() && *v >= 0.0)
            .unwrap_or(0.0),
        last_period: number(attributes.get("last_period")),
        period: attributes
            .get("last_reset")
            .and_then(Value::as_str)
            .and_then(|text| text.parse::<DateTime>().ok())
            .unwrap_or(current),
        last_source: number(attributes.get("last_valid_state")),
    };
    // Une période a commencé pendant l'arrêt.
    meter.roll(cycle, now);
    meter
}

fn copy_unit(states: &[Option<State>], attributes: &mut Attributes) {
    if let Some(unit) = states
        .iter()
        .flatten()
        .find_map(|s| s.attributes().get("unit_of_measurement").cloned())
    {
        attributes.insert("unit_of_measurement".into(), unit);
    }
}

fn statistic_name(kind: Statistic) -> &'static str {
    match kind {
        Statistic::Min => "min",
        Statistic::Max => "max",
        Statistic::Mean => "mean",
        Statistic::Median => "median",
        Statistic::Last => "last",
        Statistic::Sum => "sum",
        Statistic::Range => "range",
    }
}

fn local_now() -> DateTime {
    jiff::Zoned::now().datetime()
}
