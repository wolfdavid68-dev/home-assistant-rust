//! Décide **quelles** automatisations se déclenchent, et si leurs conditions
//! sont remplies. Aucune attente, aucun réseau : l'heure est passée en
//! paramètre, ce qui permet de tester « il est 18 h 59, puis 19 h 00 ».

use std::collections::BTreeMap;
use std::time::SystemTime;

use ha_core::{Attributes, EntityId, State, StateMachine};
use ha_sun::{Location, SunEvent};
use jiff::SignedDuration;
use jiff::civil::{Date, DateTime, Time};
use jiff::tz::TimeZone;
use serde_json::Value;
use tokio::time::Instant;

use crate::config::{Automation, CalendarEdge, Condition, Pattern, TimeAt, Trigger};
use crate::template::TriggerData;

/// Un déclencheur `state` avec `for:` en attente : « la porte est ouverte…
/// on vérifiera dans 5 minutes qu'elle l'est toujours ».
#[derive(Debug)]
struct Pending {
    automation: usize,
    trigger: usize,
    entity_id: EntityId,
    /// Attribut surveillé (sinon, l'état).
    attribute: Option<String>,
    /// Valeur attendue, et date du changement d'état (sans attribut) : si
    /// l'une ou l'autre a bougé entre-temps, le déclencheur est annulé.
    value: Option<String>,
    since: Option<SystemTime>,
    /// L'état d'avant le changement (pour `trigger.from_state`).
    from_state: Option<State>,
    deadline: Instant,
}

#[derive(Debug)]
pub struct Evaluator {
    automations: Vec<Automation>,
    pending: Vec<Pending>,
    sun: Option<Sun>,
}

/// Une automatisation déclenchée, par lequel de ses déclencheurs, et le
/// changement d'état en cause (pour `trigger.to_state`… dans les modèles).
#[derive(Debug, Clone, PartialEq)]
pub struct Fired {
    pub automation: usize,
    pub trigger: usize,
    pub data: Option<TriggerData>,
}

/// Les automatisations déclenchées : pour chacune, le premier déclencheur.
type FiredMap = BTreeMap<usize, (usize, Option<TriggerData>)>;

/// Le changement en cause, pour les modèles.
fn change(entity_id: &EntityId, old: Option<&State>, new: Option<&State>) -> TriggerData {
    TriggerData {
        id: None,
        entity_id: Some(entity_id.clone()),
        from_state: old.cloned(),
        to_state: new.cloned(),
        ..TriggerData::default()
    }
}

/// Ce qu'il faut pour vérifier une condition, à tout moment (aussi au milieu
/// des actions : `if`, `choose`, `condition`). Concept Rust : `Clone` permet
/// d'en donner une copie à chaque exécution, qui tourne dans sa propre tâche.
#[derive(Debug, Clone, Default)]
pub struct Checker {
    sun: Option<Sun>,
}

impl Checker {
    /// La condition est-elle vraie ? `trigger` : l'identifiant du
    /// déclencheur qui a lancé l'exécution (pour la condition `trigger`).
    #[must_use]
    pub fn passes(
        &self,
        condition: &Condition,
        states: &StateMachine,
        now: DateTime,
        trigger: Option<&TriggerData>,
    ) -> bool {
        condition_passes(condition, states, now, self.sun.as_ref(), trigger)
    }

    /// Toutes les conditions de la liste sont-elles vraies ?
    #[must_use]
    pub fn all(
        &self,
        conditions: &[Condition],
        states: &StateMachine,
        now: DateTime,
        trigger: Option<&TriggerData>,
    ) -> bool {
        conditions
            .iter()
            .all(|c| self.passes(c, states, now, trigger))
    }
}

/// De quoi calculer les heures du soleil, en heure locale.
#[derive(Debug, Clone)]
struct Sun {
    location: Location,
    zone: TimeZone,
}

impl Sun {
    /// L'heure locale de `event` (plus `offset`) le jour `date`. `None` si
    /// l'événement n'a pas lieu ce jour-là (jour ou nuit polaire).
    fn event_at(&self, date: Date, event: SunEvent, offset: SignedDuration) -> Option<DateTime> {
        let time = event.time_in(&ha_sun::sun_times(date, self.location)?)?;
        let time = time.checked_add(offset).ok()?;
        Some(time.to_zoned(self.zone.clone()).datetime())
    }
}

impl Evaluator {
    #[must_use]
    pub fn new(automations: Vec<Automation>) -> Self {
        Self {
            automations,
            pending: Vec::new(),
            sun: None,
        }
    }

    /// Active les déclencheurs et conditions `sun`, pour la maison située à
    /// `location`, dans le fuseau horaire `zone`.
    #[must_use]
    pub fn with_sun(mut self, location: Location, zone: TimeZone) -> Self {
        self.sun = Some(Sun { location, zone });
        self
    }

    #[must_use]
    pub fn automations(&self) -> &[Automation] {
        &self.automations
    }

    /// Remplace les automatisations (après une modification dans l'éditeur).
    ///
    /// Renvoie, pour chaque nouvelle automatisation, sa position dans
    /// l'ancienne liste si elle **n'a pas changé**. Ses attentes `for` en
    /// cours sont gardées ; celles des automatisations modifiées ou
    /// supprimées sont abandonnées.
    pub fn replace(&mut self, automations: Vec<Automation>) -> Vec<Option<usize>> {
        let previous: Vec<Option<usize>> = automations
            .iter()
            .map(|new| self.automations.iter().position(|old| old == new))
            .collect();
        self.pending.retain_mut(|pending| {
            match previous
                .iter()
                .position(|old| *old == Some(pending.automation))
            {
                Some(new) => {
                    pending.automation = new;
                    true
                }
                None => false,
            }
        });
        self.automations = automations;
        previous
    }

    /// De quoi vérifier les conditions pendant les actions.
    #[must_use]
    pub fn checker(&self) -> Checker {
        Checker {
            sun: self.sun.clone(),
        }
    }

    /// Un changement d'état : renvoie les automatisations déclenchées
    /// (leur position dans la liste).
    pub fn on_state_changed(
        &mut self,
        entity_id: &EntityId,
        old: Option<&State>,
        new: Option<&State>,
        now: Instant,
    ) -> Vec<usize> {
        automations(self.fire_on_state_changed(entity_id, old, new, now))
    }

    /// Comme [`Evaluator::on_state_changed`], avec le déclencheur de chacune.
    pub fn fire_on_state_changed(
        &mut self,
        entity_id: &EntityId,
        old: Option<&State>,
        new: Option<&State>,
        now: Instant,
    ) -> Vec<Fired> {
        // Concept Rust : `entry(a).or_insert(t)` garde le **premier**
        // déclencheur qui a fonctionné pour chaque automatisation.
        let mut fired = FiredMap::new();
        for (a, automation) in self.automations.iter().enumerate() {
            for (t, trigger) in automation.triggers.iter().enumerate() {
                match trigger {
                    Trigger::State {
                        entity_ids,
                        attribute,
                        from,
                        to,
                        not_from,
                        not_to,
                        for_,
                    } if entity_ids.contains(entity_id) => {
                        let attribute = attribute.as_deref();
                        let before = watched(old, attribute);
                        let after = watched(new, attribute);
                        // La valeur surveillée n'a pas bougé (un autre attribut
                        // a changé) : rien ne se passe, une attente continue.
                        if new.is_some() && before == after {
                            continue;
                        }
                        // Sinon, un changement annule toujours l'attente en cours.
                        self.pending.retain(|p| {
                            !(p.automation == a && p.trigger == t && &p.entity_id == entity_id)
                        });
                        let Some(new) = new else { continue };
                        let Some(after) = after else { continue };
                        let matches = |list: &[String], value: Option<&String>| {
                            value.is_some_and(|value| list.contains(value))
                        };
                        if (!from.is_empty() && !matches(from, before.as_ref()))
                            || (!to.is_empty() && !to.contains(&after))
                            || matches(not_from, before.as_ref())
                            || not_to.contains(&after)
                        {
                            continue;
                        }
                        match for_ {
                            None => {
                                fired
                                    .entry(a)
                                    .or_insert((t, Some(change(entity_id, old, Some(new)))));
                            }
                            Some(duration) => self.pending.push(Pending {
                                automation: a,
                                trigger: t,
                                entity_id: entity_id.clone(),
                                attribute: attribute.map(str::to_owned),
                                value: Some(after),
                                since: attribute.is_none().then(|| new.last_changed()),
                                from_state: old.cloned(),
                                deadline: now + *duration,
                            }),
                        }
                    }
                    Trigger::NumericState {
                        entity_ids,
                        above,
                        below,
                    } if entity_ids.contains(entity_id) => {
                        let inside = |state: Option<&State>| {
                            state
                                .and_then(|s| s.state().parse::<f64>().ok())
                                .is_some_and(|value| in_range(value, *above, *below))
                        };
                        // On déclenche en **entrant** dans l'intervalle, pas à
                        // chaque nouvelle mesure tant qu'on y reste.
                        if inside(new) && !inside(old) {
                            fired
                                .entry(a)
                                .or_insert((t, Some(change(entity_id, old, new))));
                        }
                    }
                    _ => {}
                }
            }
        }
        to_fired(fired)
    }

    /// Un événement nommé (`timer.finished`…) : renvoie les automatisations
    /// déclenchées. Les détails de l'événement doivent contenir toutes les
    /// valeurs de `event_data` (et peuvent en contenir d'autres).
    #[must_use]
    pub fn fire_on_event(&self, event_type: &str, data: &Attributes) -> Vec<Fired> {
        let mut fired = FiredMap::new();
        for (a, automation) in self.automations.iter().enumerate() {
            for (t, trigger) in automation.triggers.iter().enumerate() {
                if let Trigger::Event {
                    event_type: wanted,
                    event_data,
                } = trigger
                    && wanted == event_type
                    && event_data
                        .iter()
                        .all(|(key, value)| data.get(key) == Some(value))
                {
                    // Pour les modèles : `trigger.event.data.…`.
                    fired.entry(a).or_insert((
                        t,
                        Some(TriggerData {
                            event_type: Some(event_type.to_owned()),
                            event_data: Some(data.clone()),
                            ..TriggerData::default()
                        }),
                    ));
                }
            }
        }
        to_fired(fired)
    }

    /// Appelé régulièrement (chaque seconde) : déclencheurs `time` passés
    /// entre `previous` et `now`, et attentes `for:` arrivées à échéance.
    pub fn on_tick(
        &mut self,
        previous: DateTime,
        now: DateTime,
        instant: Instant,
        states: &StateMachine,
    ) -> Vec<usize> {
        automations(self.fire_on_tick(previous, now, instant, states))
    }

    /// Comme [`Evaluator::on_tick`], avec le déclencheur de chacune.
    pub fn fire_on_tick(
        &mut self,
        previous: DateTime,
        now: DateTime,
        instant: Instant,
        states: &StateMachine,
    ) -> Vec<Fired> {
        let mut fired = FiredMap::new();
        for (a, automation) in self.automations.iter().enumerate() {
            for (t, trigger) in automation.triggers.iter().enumerate() {
                let crossed_now = match trigger {
                    Trigger::Time { at } => at.iter().any(|at| match at {
                        TimeAt::Fixed(time) => crossed(previous, now, *time),
                        TimeAt::Entity(id) => entity_time_crossed(states, id, previous, now),
                    }),
                    Trigger::TimePattern {
                        hours,
                        minutes,
                        second,
                    } => pattern_crossed(previous, now, *hours, *minutes, *second),
                    Trigger::Sun { event, offset } => self.sun.as_ref().is_some_and(|sun| {
                        [previous.date(), now.date()].into_iter().any(|date| {
                            sun.event_at(date, *event, *offset)
                                .is_some_and(|moment| previous < moment && moment <= now)
                        })
                    }),
                    Trigger::Calendar {
                        entity_id,
                        edge,
                        offset,
                    } => calendar_crossed(states, entity_id, *edge, *offset, previous, now),
                    _ => false,
                };
                if crossed_now {
                    fired.entry(a).or_insert((t, None));
                }
            }
        }

        let (due, waiting): (Vec<Pending>, Vec<Pending>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|p| p.deadline <= instant);
        self.pending = waiting;
        for pending in due {
            let current = states.get(&pending.entity_id);
            let still_true = current.as_ref().is_some_and(|state| {
                watched(Some(state), pending.attribute.as_deref()) == pending.value
                    && pending
                        .since
                        .is_none_or(|since| state.last_changed() == since)
            });
            if still_true {
                let data = change(
                    &pending.entity_id,
                    pending.from_state.as_ref(),
                    current.as_ref(),
                );
                fired
                    .entry(pending.automation)
                    .or_insert((pending.trigger, Some(data)));
            }
        }
        to_fired(fired)
    }

    /// Toutes les conditions de l'automatisation sont-elles remplies ?
    pub fn conditions_pass(&self, automation: usize, states: &StateMachine, now: DateTime) -> bool {
        self.conditions_pass_for(automation, states, now, None)
    }

    /// Comme [`Evaluator::conditions_pass`], en sachant quel déclencheur
    /// (son identifiant) a lancé l'automatisation.
    pub fn conditions_pass_for(
        &self,
        automation: usize,
        states: &StateMachine,
        now: DateTime,
        trigger: Option<&str>,
    ) -> bool {
        let Some(automation) = self.automations.get(automation) else {
            return false;
        };
        let trigger = TriggerData {
            id: trigger.map(str::to_owned),
            ..TriggerData::default()
        };
        automation.conditions.iter().all(|condition| {
            condition_passes(condition, states, now, self.sun.as_ref(), Some(&trigger))
        })
    }
}

/// Un rendez-vous de l'agenda commence (ou se termine), décalage compris,
/// entre `previous` et `now` ? Les rendez-vous viennent de l'attribut
/// `events` de l'agenda (`{"start": "2026-10-15T18:30:00", …}`) : le moteur
/// n'a pas besoin de lire la base.
fn calendar_crossed(
    states: &StateMachine,
    entity_id: &EntityId,
    edge: CalendarEdge,
    offset: SignedDuration,
    previous: DateTime,
    now: DateTime,
) -> bool {
    let Some(calendar) = states.get(entity_id) else {
        return false;
    };
    let Some(events) = calendar
        .attributes()
        .get("events")
        .and_then(Value::as_array)
    else {
        return false;
    };
    events.iter().any(|event| {
        event
            .get(edge.field())
            .and_then(Value::as_str)
            .and_then(|text| text.parse::<DateTime>().ok())
            .and_then(|moment| moment.checked_add(offset).ok())
            .is_some_and(|moment| previous < moment && moment <= now)
    })
}

fn to_fired(map: FiredMap) -> Vec<Fired> {
    map.into_iter()
        .map(|(automation, (trigger, data))| Fired {
            automation,
            trigger,
            data,
        })
        .collect()
}

fn automations(fired: Vec<Fired>) -> Vec<usize> {
    fired.into_iter().map(|f| f.automation).collect()
}

/// La valeur surveillée : l'état, ou un attribut écrit en texte (`128`,
/// `Orange`, `true`). `None` : pas d'entité, ou pas cet attribut.
pub(crate) fn watched(state: Option<&State>, attribute: Option<&str>) -> Option<String> {
    let state = state?;
    let Some(attribute) = attribute else {
        return Some(state.state().to_owned());
    };
    match state.attributes().get(attribute)? {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    }
}

/// `above` et `below` sont exclus, comme dans Home Assistant.
fn in_range(value: f64, above: Option<f64>, below: Option<f64>) -> bool {
    above.is_none_or(|a| value > a) && below.is_none_or(|b| value < b)
}

/// L'heure `time` (aujourd'hui ou hier) tombe-t-elle dans `]previous, now]` ?
fn crossed(previous: DateTime, now: DateTime, time: Time) -> bool {
    [previous.date(), now.date()]
        .into_iter()
        .map(|date| date.to_datetime(time))
        .any(|moment| previous < moment && moment <= now)
}

/// L'heure réglée dans une entrée `input_datetime` est-elle passée entre
/// `previous` (exclu) et `now` ? Une heure seule (`07:30:00`) sonne chaque
/// jour ; une date (`2026-10-20`) à minuit ce jour-là ; une date et une heure
/// une seule fois.
fn entity_time_crossed(
    states: &StateMachine,
    id: &EntityId,
    previous: DateTime,
    now: DateTime,
) -> bool {
    let Some(state) = states.get(id) else {
        return false;
    };
    let text = state.state();
    // Une heure seule (`07:30:00`). Attention : `jiff` accepte aussi de lire
    // une heure dans « 2026-10-16 07:30:00 » (il garde l'heure) ; on vérifie
    // donc d'abord qu'il n'y a pas de date.
    if !text.contains('-')
        && let Ok(time) = text.parse::<Time>()
    {
        return crossed(previous, now, time);
    }
    let moment = text
        .replacen(' ', "T", 1)
        .parse::<DateTime>()
        .ok()
        .or_else(|| {
            text.parse::<Date>()
                .ok()
                .map(|d| d.to_datetime(Time::midnight()))
        });
    moment.is_some_and(|moment| previous < moment && moment <= now)
}

/// Un moment du motif est-il passé entre `previous` (exclu) et `now` ?
/// On regarde les minutes de l'intervalle (deux heures au plus : après une
/// mise en veille plus longue, on ne rattrape pas tout).
fn pattern_crossed(
    previous: DateTime,
    now: DateTime,
    hours: Pattern,
    minutes: Pattern,
    second: i8,
) -> bool {
    let Ok(mut minute) = previous.with().second(0).subsec_nanosecond(0).build() else {
        return false;
    };
    for _ in 0..=120 {
        if minute > now {
            break;
        }
        if hours.matches(minute.hour())
            && minutes.matches(minute.minute())
            && let Ok(moment) = minute.with().second(second).build()
            && previous < moment
            && moment <= now
        {
            return true;
        }
        let Ok(next) = minute.checked_add(SignedDuration::from_mins(1)) else {
            break;
        };
        minute = next;
    }
    false
}

fn condition_passes(
    condition: &Condition,
    states: &StateMachine,
    now: DateTime,
    sun: Option<&Sun>,
    trigger: Option<&TriggerData>,
) -> bool {
    let check = |c: &Condition| condition_passes(c, states, now, sun, trigger);
    match condition {
        Condition::And(list) => list.iter().all(check),
        Condition::Or(list) => list.iter().any(check),
        // Comme dans Home Assistant : vrai si **aucune** n'est vraie.
        Condition::Not(list) => !list.iter().any(check),
        Condition::Trigger(ids) => trigger
            .and_then(|t| t.id.as_deref())
            .is_some_and(|t| ids.iter().any(|id| id == t)),
        // Comme Home Assistant : vrai si le modèle donne `true`, un nombre
        // non nul, ou « on », « yes »… ; un modèle qui n'aboutit pas est faux.
        Condition::Template { template, .. } => {
            crate::template::is_true(&template.evaluate(states, trigger))
        }
        Condition::State {
            entity_ids,
            attribute,
            states: expected,
        } => entity_ids.iter().all(|id| {
            watched(states.get(id).as_ref(), attribute.as_deref())
                .is_some_and(|value| expected.contains(&value))
        }),
        Condition::NumericState {
            entity_ids,
            above,
            below,
        } => entity_ids.iter().all(|id| {
            states
                .get(id)
                .and_then(|state| state.state().parse::<f64>().ok())
                .is_some_and(|value| in_range(value, *above, *below))
        }),
        Condition::Time {
            after,
            before,
            weekdays,
        } => {
            let time = now.time();
            let in_hours = match (after, before) {
                // Plage qui passe minuit : « après 22 h ou avant 6 h ».
                (Some(after), Some(before)) if after > before => time >= *after || time < *before,
                (after, before) => {
                    after.is_none_or(|a| time >= a) && before.is_none_or(|b| time < b)
                }
            };
            in_hours && (weekdays.is_empty() || weekdays.contains(&now.weekday()))
        }
        Condition::Sun { after, before } => {
            let Some(sun) = sun else { return false };
            // `Some(None)` : l'événement n'a pas lieu aujourd'hui (pays
            // polaires) ; la condition est alors fausse.
            let at = |limit: &Option<(SunEvent, SignedDuration)>| {
                limit.map(|(event, offset)| sun.event_at(now.date(), event, offset))
            };
            match (at(after), at(before)) {
                // « Après le coucher et avant le lever » : la plage passe
                // minuit, comme pour la condition `time`.
                (Some(Some(after)), Some(Some(before))) if after > before => {
                    now >= after || now < before
                }
                (after, before) => {
                    after.is_none_or(|after| after.is_some_and(|after| now >= after))
                        && before.is_none_or(|before| before.is_some_and(|before| now < before))
                }
            }
        }
    }
}
