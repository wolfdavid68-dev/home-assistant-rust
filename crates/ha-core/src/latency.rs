//! La rapidité des appareils (étape 63), mesurée de bout en bout : pour
//! chaque ordre, le temps entre l'ordre (le toucher sur le tableau de bord,
//! une automatisation…) et le **premier changement d'état** de l'appareil.
//! C'est le moment où le tableau de bord voit la lampe s'allumer ou le volet
//! se mettre en marche.
//!
//! - Les mesures sont rangées **par intégration** (Hue, Somfy, MQTT…) : la
//!   page Système montre la médiane et le 95e centile de chacune.
//! - Les 3 dernières mesures de **chaque appareil** sont gardées : trois
//!   ordres de suite à plus de [`SLOW`] font une alerte « répond lentement ».
//! - Un ordre qui ne change rien (allumer une lampe déjà allumée) n'a pas de
//!   réponse : il est oublié au bout de [`ANSWER_WITHIN`], sans compter.
//! - Seules les entités d'une intégration sont mesurées : un état posé par
//!   l'API ou une entrée virtuelle n'a pas d'appareil derrière lui.
//!
//! Tout est gardé en mémoire, rien n'est écrit dans la base.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::{EntityId, Event, StateMachine};

/// Au-delà, un ordre n'a pas eu de réponse (il ne changeait rien) : il
/// n'est pas compté.
pub const ANSWER_WITHIN: Duration = Duration::from_secs(30);
/// Un appareil est lent au-delà de ce temps de réponse.
pub const SLOW: Duration = Duration::from_secs(3);
/// Mesures gardées par intégration.
const SAMPLES: usize = 200;
/// Mesures gardées par appareil (pour l'alerte).
pub const DEVICE_SAMPLES: usize = 3;
/// Ordres en attente de réponse au plus.
const MAX_PENDING: usize = 256;
/// Appareils suivis au plus.
const MAX_DEVICES: usize = 2_000;

/// La rapidité d'une intégration.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// L'intégration propriétaire : `hue`, `somfy`, `mqtt`…
    pub owner: String,
    /// Médiane et 95e centile, en millisecondes.
    pub p50_ms: f64,
    pub p95_ms: f64,
    /// Combien de mesures.
    pub samples: usize,
    /// Combien au-delà de [`SLOW`].
    pub slow: usize,
}

#[derive(Debug, Default)]
struct Inner {
    /// L'ordre en attente de chaque appareil : quand, et à quelle
    /// intégration.
    pending: HashMap<EntityId, (Instant, String)>,
    /// Les dernières mesures de chaque intégration, en millisecondes.
    owners: HashMap<String, VecDeque<u32>>,
    /// Les dernières mesures de chaque appareil.
    devices: HashMap<EntityId, VecDeque<Duration>>,
}

/// Les mesures, partagées entre le bus (qui les remplit) et l'API.
#[derive(Debug, Default)]
pub struct Latencies(Mutex<Inner>);

fn millis(duration: Duration) -> u32 {
    u32::try_from(duration.as_millis()).unwrap_or(u32::MAX)
}

impl Latencies {
    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Regarde passer un événement du bus. `now` : quand il a été reçu.
    pub fn observe(&self, event: &Event, states: &StateMachine, now: Instant) {
        match event {
            Event::CallService {
                entity_id,
                issued_at,
                ..
            } => {
                let Some(owner) = states.owner_of(entity_id) else {
                    return;
                };
                let mut inner = self.inner();
                inner
                    .pending
                    .retain(|_, (at, _)| now.saturating_duration_since(*at) <= ANSWER_WITHIN);
                if inner.pending.len() >= MAX_PENDING && !inner.pending.contains_key(entity_id) {
                    return;
                }
                // Deux ordres de suite : on mesure le dernier.
                inner.pending.insert(entity_id.clone(), (*issued_at, owner));
            }
            Event::StateChanged {
                entity_id,
                new_state: Some(_),
                ..
            } => {
                let mut inner = self.inner();
                let Some((at, owner)) = inner.pending.remove(entity_id) else {
                    return;
                };
                let took = now.saturating_duration_since(at);
                if took > ANSWER_WITHIN {
                    return;
                }
                let samples = inner.owners.entry(owner).or_default();
                if samples.len() == SAMPLES {
                    samples.pop_front();
                }
                samples.push_back(millis(took));
                if inner.devices.len() >= MAX_DEVICES && !inner.devices.contains_key(entity_id) {
                    return;
                }
                let device = inner.devices.entry(entity_id.clone()).or_default();
                if device.len() == DEVICE_SAMPLES {
                    device.pop_front();
                }
                device.push_back(took);
            }
            _ => {}
        }
    }

    /// La rapidité de chaque intégration, dans l'ordre alphabétique.
    #[must_use]
    pub fn summaries(&self) -> Vec<Summary> {
        let inner = self.inner();
        let mut summaries: Vec<Summary> = inner
            .owners
            .iter()
            .filter(|(_, samples)| !samples.is_empty())
            .map(|(owner, samples)| {
                let mut sorted: Vec<u32> = samples.iter().copied().collect();
                sorted.sort_unstable();
                let at = |q: f64| {
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        clippy::cast_precision_loss,
                        reason = "rang dans une liste de 200"
                    )]
                    let index = ((sorted.len() - 1) as f64 * q).round() as usize;
                    f64::from(sorted.get(index).copied().unwrap_or_default())
                };
                Summary {
                    owner: owner.clone(),
                    p50_ms: at(0.5),
                    p95_ms: at(0.95),
                    samples: sorted.len(),
                    slow: sorted.iter().filter(|ms| **ms > millis(SLOW)).count(),
                }
            })
            .collect();
        summaries.sort_by(|a, b| a.owner.cmp(&b.owner));
        summaries
    }

    /// Les appareils dont les [`DEVICE_SAMPLES`] dernières réponses ont
    /// toutes dépassé [`SLOW`], avec la plus rapide des trois.
    #[must_use]
    pub fn slow_devices(&self) -> Vec<(EntityId, Duration)> {
        let inner = self.inner();
        let mut slow: Vec<(EntityId, Duration)> = inner
            .devices
            .iter()
            .filter(|(_, samples)| {
                samples.len() == DEVICE_SAMPLES && samples.iter().all(|took| *took > SLOW)
            })
            .filter_map(|(entity_id, samples)| {
                samples
                    .iter()
                    .min()
                    .map(|fastest| (entity_id.clone(), *fastest))
            })
            .collect();
        slow.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        slow
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::float_cmp, reason = "tests")]

    use super::*;
    use crate::{Attributes, Core, Origin};

    fn order(entity: &str, at: Instant) -> Event {
        Event::CallService {
            domain: "light".into(),
            service: "turn_on".into(),
            entity_id: entity.parse().unwrap(),
            data: Attributes::new(),
            origin: Origin::System,
            command: None,
            issued_at: at,
        }
    }

    fn changed(core: &Core, entity: &str) -> Event {
        let id: EntityId = entity.parse().unwrap();
        Event::StateChanged {
            entity_id: id.clone(),
            old_state: None,
            new_state: core.states.get(&id),
        }
    }

    #[test]
    fn each_order_is_timed_until_the_device_answers() {
        let core = Core::new();
        for (entity, owner) in [("light.salon", "hue"), ("cover.salon", "somfy")] {
            core.states
                .set_by(owner, entity.parse().unwrap(), "off", Attributes::new())
                .unwrap();
        }
        // Posé par l'API : pas d'appareil derrière, pas de mesure.
        core.states
            .set("light.essai".parse().unwrap(), "off", Attributes::new())
            .unwrap();
        let latencies = Latencies::default();
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        for (sent, answered) in [(0, 120), (1_000, 1_080), (2_000, 2_300)] {
            latencies.observe(&order("light.salon", at(sent)), &core.states, at(sent));
            latencies.observe(&changed(&core, "light.salon"), &core.states, at(answered));
        }
        latencies.observe(&order("cover.salon", at(0)), &core.states, at(0));
        latencies.observe(&changed(&core, "cover.salon"), &core.states, at(1_900));
        latencies.observe(&order("light.essai", at(0)), &core.states, at(0));
        latencies.observe(&changed(&core, "light.essai"), &core.states, at(10));
        // Un changement sans ordre (la lampe allumée à l'interrupteur) ne
        // compte pas.
        latencies.observe(&changed(&core, "light.salon"), &core.states, at(5_000));

        let summaries = latencies.summaries();
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].owner, "hue");
        assert_eq!(summaries[0].samples, 3);
        assert_eq!(summaries[0].p50_ms, 120.0);
        assert_eq!(summaries[0].p95_ms, 300.0);
        assert_eq!(summaries[1].owner, "somfy");
        assert_eq!(summaries[1].p50_ms, 1_900.0);
        assert_eq!(summaries[1].slow, 0);
    }

    #[test]
    fn an_order_that_changes_nothing_is_forgotten() {
        let core = Core::new();
        core.states
            .set_by(
                "hue",
                "light.salon".parse().unwrap(),
                "on",
                Attributes::new(),
            )
            .unwrap();
        let latencies = Latencies::default();
        let start = Instant::now();
        latencies.observe(&order("light.salon", start), &core.states, start);
        // Le changement arrive bien plus tard, pour une autre raison.
        let later = start + ANSWER_WITHIN + Duration::from_secs(1);
        latencies.observe(&changed(&core, "light.salon"), &core.states, later);
        assert_eq!(latencies.summaries(), Vec::new());
    }

    #[test]
    fn three_slow_answers_in_a_row_make_a_slow_device() {
        let core = Core::new();
        core.states
            .set_by(
                "mqtt",
                "switch.cave".parse().unwrap(),
                "off",
                Attributes::new(),
            )
            .unwrap();
        let latencies = Latencies::default();
        let start = Instant::now();
        let answer = |sent_s: u64, took_ms: u64| {
            let sent = start + Duration::from_secs(sent_s);
            latencies.observe(&order("switch.cave", sent), &core.states, sent);
            latencies.observe(
                &changed(&core, "switch.cave"),
                &core.states,
                sent + Duration::from_millis(took_ms),
            );
        };
        answer(0, 4_000);
        answer(10, 5_000);
        assert_eq!(latencies.slow_devices(), Vec::new(), "deux fois seulement");
        answer(20, 3_500);
        assert_eq!(
            latencies.slow_devices(),
            vec![("switch.cave".parse().unwrap(), Duration::from_millis(3_500))]
        );
        // Une réponse rapide : il n'est plus lent.
        answer(30, 200);
        assert_eq!(latencies.slow_devices(), Vec::new());
        assert_eq!(latencies.summaries()[0].slow, 3);
    }
}
