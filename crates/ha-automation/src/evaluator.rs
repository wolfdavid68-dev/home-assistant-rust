//! Décide **quelles** automatisations se déclenchent, et si leurs conditions
//! sont remplies. Aucune attente, aucun réseau : l'heure est passée en
//! paramètre, ce qui permet de tester « il est 18 h 59, puis 19 h 00 ».

use std::collections::BTreeSet;
use std::time::SystemTime;

use ha_core::{EntityId, State, StateMachine};
use ha_sun::{Location, SunEvent};
use jiff::SignedDuration;
use jiff::civil::{Date, DateTime, Time};
use jiff::tz::TimeZone;
use tokio::time::Instant;

use crate::config::{Automation, Condition, Trigger};

/// Un déclencheur `state` avec `for:` en attente : « la porte est ouverte…
/// on vérifiera dans 5 minutes qu'elle l'est toujours ».
#[derive(Debug)]
struct Pending {
    automation: usize,
    trigger: usize,
    entity_id: EntityId,
    /// Valeur attendue et date du changement : si l'une ou l'autre a bougé
    /// entre-temps, le déclencheur est annulé.
    value: String,
    since: SystemTime,
    deadline: Instant,
}

#[derive(Debug)]
pub struct Evaluator {
    automations: Vec<Automation>,
    pending: Vec<Pending>,
    sun: Option<Sun>,
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

    /// Un changement d'état : renvoie les automatisations déclenchées
    /// (leur position dans la liste).
    pub fn on_state_changed(
        &mut self,
        entity_id: &EntityId,
        old: Option<&State>,
        new: Option<&State>,
        now: Instant,
    ) -> Vec<usize> {
        let mut fired = BTreeSet::new();
        for (a, automation) in self.automations.iter().enumerate() {
            for (t, trigger) in automation.triggers.iter().enumerate() {
                match trigger {
                    Trigger::State {
                        entity_ids,
                        from,
                        to,
                        for_,
                    } if entity_ids.contains(entity_id) => {
                        // Un nouveau changement annule toujours l'attente en cours.
                        self.pending.retain(|p| {
                            !(p.automation == a && p.trigger == t && &p.entity_id == entity_id)
                        });
                        let Some(new) = new else { continue };
                        if !state_matches(from.as_deref(), to.as_deref(), old, new) {
                            continue;
                        }
                        match for_ {
                            None => {
                                fired.insert(a);
                            }
                            Some(duration) => self.pending.push(Pending {
                                automation: a,
                                trigger: t,
                                entity_id: entity_id.clone(),
                                value: new.state().to_owned(),
                                since: new.last_changed(),
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
                            fired.insert(a);
                        }
                    }
                    _ => {}
                }
            }
        }
        fired.into_iter().collect()
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
        let mut fired = BTreeSet::new();
        for (a, automation) in self.automations.iter().enumerate() {
            for trigger in &automation.triggers {
                let crossed_now = match trigger {
                    Trigger::Time { at } => at.iter().any(|time| crossed(previous, now, *time)),
                    Trigger::Sun { event, offset } => self.sun.as_ref().is_some_and(|sun| {
                        [previous.date(), now.date()].into_iter().any(|date| {
                            sun.event_at(date, *event, *offset)
                                .is_some_and(|moment| previous < moment && moment <= now)
                        })
                    }),
                    _ => false,
                };
                if crossed_now {
                    fired.insert(a);
                }
            }
        }

        let (due, waiting): (Vec<Pending>, Vec<Pending>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|p| p.deadline <= instant);
        self.pending = waiting;
        for pending in due {
            let still_true = states.get(&pending.entity_id).is_some_and(|state| {
                state.state() == pending.value && state.last_changed() == pending.since
            });
            if still_true {
                fired.insert(pending.automation);
            }
        }
        fired.into_iter().collect()
    }

    /// Toutes les conditions de l'automatisation sont-elles remplies ?
    pub fn conditions_pass(&self, automation: usize, states: &StateMachine, now: DateTime) -> bool {
        let Some(automation) = self.automations.get(automation) else {
            return false;
        };
        automation
            .conditions
            .iter()
            .all(|condition| condition_passes(condition, states, now, self.sun.as_ref()))
    }
}

fn state_matches(from: Option<&str>, to: Option<&str>, old: Option<&State>, new: &State) -> bool {
    let old_value = old.map(State::state);
    // Seul un changement de **valeur** compte (pas un simple attribut).
    if old_value == Some(new.state()) {
        return false;
    }
    from.is_none_or(|from| old_value == Some(from)) && to.is_none_or(|to| new.state() == to)
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

fn condition_passes(
    condition: &Condition,
    states: &StateMachine,
    now: DateTime,
    sun: Option<&Sun>,
) -> bool {
    match condition {
        Condition::State {
            entity_ids,
            states: expected,
        } => entity_ids.iter().all(|id| {
            states
                .get(id)
                .is_some_and(|state| expected.iter().any(|e| e == state.state()))
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
