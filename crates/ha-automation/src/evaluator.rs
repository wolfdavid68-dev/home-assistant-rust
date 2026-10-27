//! Décide **quelles** automatisations se déclenchent, et si leurs conditions
//! sont remplies. Aucune attente, aucun réseau : l'heure est passée en
//! paramètre, ce qui permet de tester « il est 18 h 59, puis 19 h 00 ».

use std::collections::{BTreeMap, HashMap};
use std::time::SystemTime;

use ha_core::{Attributes, EntityId, State, StateMachine};
use ha_sun::{Location, SunEvent};
use jiff::civil::{Date, DateTime, Time};
use jiff::tz::{AmbiguousOffset, TimeZone};
use jiff::{SignedDuration, Timestamp};
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
    /// Pour chaque entité, les déclencheurs `state` et `numeric_state` qui
    /// la surveillent (automatisation, déclencheur), dans l'ordre de la
    /// liste : un changement d'état ne parcourt qu'eux. Refait à chaque
    /// remplacement des automatisations.
    by_entity: HashMap<EntityId, Vec<(usize, usize)>>,
    pending: Vec<Pending>,
    sun: Option<Sun>,
    /// Le fuseau horaire de la maison : il dit à quel moment réel sonne
    /// « 7 h 30 », y compris les jours de changement d'heure.
    zone: TimeZone,
}

/// Une automatisation déclenchée, par lequel de ses déclencheurs, et le
/// changement d'état en cause (pour `trigger.to_state`… dans les modèles).
#[derive(Debug, Clone, PartialEq)]
pub struct Fired {
    pub automation: usize,
    pub trigger: usize,
    pub data: Option<TriggerData>,
}

/// Les déclencheurs `state` et `numeric_state` de chaque entité, dans
/// l'ordre (automatisation, puis déclencheur), chacun une seule fois même si
/// l'entité est citée deux fois.
fn index_by_entity(automations: &[Automation]) -> HashMap<EntityId, Vec<(usize, usize)>> {
    let mut index: HashMap<EntityId, Vec<(usize, usize)>> = HashMap::new();
    for (a, automation) in automations.iter().enumerate() {
        for (t, trigger) in automation.triggers.iter().enumerate() {
            if let Trigger::State { entity_ids, .. } | Trigger::NumericState { entity_ids, .. } =
                trigger
            {
                for entity_id in entity_ids {
                    let watching = index.entry(entity_id.clone()).or_default();
                    if watching.last() != Some(&(a, t)) {
                        watching.push((a, t));
                    }
                }
            }
        }
    }
    index
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
    /// Le moment de `event` (plus `offset`) le jour `date`. `None` si
    /// l'événement n'a pas lieu ce jour-là (jour ou nuit polaire).
    fn event_at(&self, date: Date, event: SunEvent, offset: SignedDuration) -> Option<Timestamp> {
        let time = event.time_in(&ha_sun::sun_times(date, self.location)?)?;
        time.checked_add(offset).ok()
    }
}

impl Evaluator {
    #[must_use]
    pub fn new(automations: Vec<Automation>) -> Self {
        Self {
            by_entity: index_by_entity(&automations),
            automations,
            pending: Vec::new(),
            sun: None,
            // Par défaut UTC : pas de changement d'heure, les heures civiles
            // passées à `on_tick` se comparent telles quelles (les tests).
            zone: TimeZone::UTC,
        }
    }

    /// Le fuseau horaire de la maison (le moteur prend celui de la machine).
    #[must_use]
    pub fn with_zone(mut self, zone: TimeZone) -> Self {
        self.zone = zone;
        self
    }

    /// Active les déclencheurs et conditions `sun`, pour la maison située à
    /// `location`, dans le fuseau horaire `zone` (qui devient aussi celui de
    /// l'évaluateur, voir [`Evaluator::with_zone`]).
    #[must_use]
    pub fn with_sun(mut self, location: Location, zone: TimeZone) -> Self {
        self.zone = zone.clone();
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
        self.by_entity = index_by_entity(&automations);
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
        // Seuls les déclencheurs qui surveillent cette entité sont parcourus,
        // dans l'ordre de la liste (comme un parcours complet).
        let watching = self.by_entity.get(entity_id).map_or(&[][..], Vec::as_slice);
        for &(a, t) in watching {
            let Some(trigger) = self
                .automations
                .get(a)
                .and_then(|automation| automation.triggers.get(t))
            else {
                continue;
            };
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
    /// Les heures sont locales, lues dans le fuseau de l'évaluateur ; le
    /// moteur, lui, passe des moments réels à [`Evaluator::on_tick_at`].
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
        let previous = instant_of(&self.zone, previous).unwrap_or_default();
        let now = instant_of(&self.zone, now).unwrap_or_default();
        self.fire_on_tick_at(previous, now, instant, states)
    }

    /// Comme [`Evaluator::on_tick`], avec des moments réels.
    pub fn on_tick_at(
        &mut self,
        previous: Timestamp,
        now: Timestamp,
        instant: Instant,
        states: &StateMachine,
    ) -> Vec<usize> {
        automations(self.fire_on_tick_at(previous, now, instant, states))
    }

    /// Le cœur du tic. On compare des moments réels (`Timestamp`), pas des
    /// heures affichées : quand on recule d'une heure, l'horloge repasse par
    /// 2 h 30, mais le temps, lui, ne revient pas en arrière. Chaque heure
    /// locale sonne donc une fois par jour (voir `instant_of`).
    pub fn fire_on_tick_at(
        &mut self,
        previous: Timestamp,
        now: Timestamp,
        instant: Instant,
        states: &StateMachine,
    ) -> Vec<Fired> {
        let span = Span {
            zone: &self.zone,
            previous,
            now,
        };
        let mut fired = FiredMap::new();
        for (a, automation) in self.automations.iter().enumerate() {
            for (t, trigger) in automation.triggers.iter().enumerate() {
                let crossed_now = match trigger {
                    Trigger::Time { at } => at.iter().any(|at| match at {
                        TimeAt::Fixed(time) => span.crossed(*time),
                        TimeAt::Entity(id) => entity_time_crossed(states, id, &span),
                    }),
                    Trigger::TimePattern {
                        hours,
                        minutes,
                        second,
                    } => span.pattern_crossed(*hours, *minutes, *second),
                    Trigger::Sun { event, offset } => self.sun.as_ref().is_some_and(|sun| {
                        span.dates().any(|date| {
                            sun.event_at(date, *event, *offset)
                                .is_some_and(|moment| span.contains(moment))
                        })
                    }),
                    Trigger::Calendar {
                        entity_id,
                        edge,
                        offset,
                    } => {
                        // Le rendez-vous en cause, pour les modèles
                        // (`trigger.calendar_event.summary`).
                        if let Some(event) =
                            calendar_crossed(states, entity_id, *edge, *offset, &span)
                        {
                            fired.entry(a).or_insert((
                                t,
                                Some(TriggerData {
                                    entity_id: Some(entity_id.clone()),
                                    calendar_event: Some(event),
                                    ..TriggerData::default()
                                }),
                            ));
                        }
                        false
                    }
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
                    current.as_deref(),
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
    span: &Span,
) -> Option<Attributes> {
    let calendar = states.get(entity_id)?;
    let events = calendar
        .attributes()
        .get("events")
        .and_then(Value::as_array)?;
    events
        .iter()
        .find(|event| {
            event
                .get(edge.field())
                .and_then(Value::as_str)
                .and_then(|text| text.parse::<DateTime>().ok())
                .and_then(|moment| moment.checked_add(offset).ok())
                .is_some_and(|moment| span.contains_local(moment))
        })
        .and_then(Value::as_object)
        .cloned()
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

/// Le moment réel où sonne l'heure locale `moment` dans `zone`.
///
/// Deux jours par an, la réponse n'est pas évidente :
/// - on recule d'une heure (fin octobre) : 2 h 30 a lieu deux fois. On
///   sonne à la première ; la seconde fois, ce moment est déjà passé.
/// - on avance d'une heure (fin mars) : 2 h 30 n'existe pas. On sonne une
///   fois, à la fin du trou (3 h 00), plutôt que de sauter le jour.
///
/// Concept : `to_ambiguous_timestamp` ne choisit pas à notre place, il dit
/// si l'heure est normale, dans un trou (`Gap`) ou en double (`Fold`).
fn instant_of(zone: &TimeZone, moment: DateTime) -> Option<Timestamp> {
    let ambiguous = zone.to_ambiguous_timestamp(moment);
    if let AmbiguousOffset::Gap { .. } = ambiguous.offset() {
        // `earlier` lit l'heure avec le décalage d'après le trou : un moment
        // juste avant le changement. Le changement suivant est la fin du trou.
        let before = ambiguous.earlier().ok()?;
        return zone.following(before).next().map(|t| t.timestamp());
    }
    ambiguous.earlier().ok()
}

/// L'intervalle `]previous, now]` d'un tic, et le fuseau où lire les heures.
struct Span<'a> {
    zone: &'a TimeZone,
    previous: Timestamp,
    now: Timestamp,
}

impl Span<'_> {
    fn contains(&self, moment: Timestamp) -> bool {
        self.previous < moment && moment <= self.now
    }

    /// L'heure locale `moment` sonne-t-elle dans l'intervalle ?
    fn contains_local(&self, moment: DateTime) -> bool {
        instant_of(self.zone, moment).is_some_and(|at| self.contains(at))
    }

    /// Les jours (locaux) où tombent `previous` et `now`.
    fn dates(&self) -> impl Iterator<Item = Date> {
        [self.previous, self.now]
            .map(|moment| self.zone.to_datetime(moment).date())
            .into_iter()
    }

    /// L'heure `time` (aujourd'hui ou hier) tombe-t-elle dans l'intervalle ?
    fn crossed(&self, time: Time) -> bool {
        self.dates()
            .any(|date| self.contains_local(date.to_datetime(time)))
    }

    /// Un moment du motif est-il passé dans l'intervalle ? On regarde les
    /// minutes locales à partir de `previous` (deux heures au plus : après
    /// une mise en veille plus longue, on ne rattrape pas tout).
    fn pattern_crossed(&self, hours: Pattern, minutes: Pattern, second: i8) -> bool {
        let start = self.zone.to_datetime(self.previous);
        let Ok(mut minute) = start.with().second(0).subsec_nanosecond(0).build() else {
            return false;
        };
        for _ in 0..=120 {
            // Fin octobre, les minutes de l'heure rejouée ont sonné la
            // première fois : elles tombent avant `previous`, on passe.
            if instant_of(self.zone, minute).is_none_or(|at| at > self.now) {
                break;
            }
            if hours.matches(minute.hour())
                && minutes.matches(minute.minute())
                && let Ok(moment) = minute.with().second(second).build()
                && self.contains_local(moment)
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
}

/// L'heure réglée dans une entrée `input_datetime` est-elle passée dans
/// l'intervalle ? Une heure seule (`07:30:00`) sonne chaque jour ; une date
/// (`2026-10-20`) à minuit ce jour-là ; une date et une heure une seule fois.
fn entity_time_crossed(states: &StateMachine, id: &EntityId, span: &Span) -> bool {
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
        return span.crossed(time);
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
    moment.is_some_and(|moment| span.contains_local(moment))
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
            watched(states.get(id).as_deref(), attribute.as_deref())
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
                limit.map(|(event, offset)| {
                    sun.event_at(now.date(), event, offset)
                        .map(|moment| sun.zone.to_datetime(moment))
                })
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
