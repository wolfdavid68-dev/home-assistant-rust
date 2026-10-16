//! Les intégrations (Hue, Somfy, MQTT…) : leur santé, et un superviseur qui
//! les relance si elles plantent.
//!
//! Chaque intégration dit où elle en est ([`Health`]) : connectée, en train
//! de réessayer, plantée. Si elle reste en panne plus de [`GRACE`], ses
//! entités passent « indisponibles » (`unavailable`, comme dans Home
//! Assistant) : le tableau de bord ne montre plus une lampe « allumée » qu'on
//! ne peut plus joindre. Dès qu'elle revient, ce qu'elle n'a pas encore
//! republié retrouve sa dernière valeur connue.
//!
//! Le superviseur ([`supervise`]) lance l'intégration dans sa propre tâche :
//! si elle panique ou s'arrête toute seule, il la relance après une attente
//! qui double à chaque échec (1 s, 2 s, 4 s… jusqu'à 1 min). Une intégration
//! en panne n'emporte jamais le serveur.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use tokio_util::sync::CancellationToken;

use crate::Core;
use crate::entity_id::EntityId;
use crate::state::Attributes;
use crate::task::Background;

/// Combien de temps une intégration peut être en panne avant que ses
/// entités passent « indisponibles » : une coupure de quelques secondes
/// (redémarrage du pont, du broker) ne doit pas faire clignoter la maison.
pub const GRACE: Duration = Duration::from_mins(1);
/// Première attente avant de relancer une intégration plantée.
pub const FIRST_RETRY: Duration = Duration::from_secs(1);
/// Attente maximale entre deux relances.
pub const MAX_RETRY: Duration = Duration::from_mins(1);
/// Une intégration qui a tenu aussi longtemps est considérée comme saine :
/// l'attente repart de [`FIRST_RETRY`] à la panne suivante.
pub const HEALTHY_RUN: Duration = Duration::from_mins(1);
/// La valeur d'une entité injoignable (comme dans Home Assistant).
pub const UNAVAILABLE: &str = "unavailable";

/// Où en est une intégration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    /// Elle démarre (ou redémarre) : pas encore connectée.
    Starting,
    /// Connectée, elle fonctionne.
    Connected,
    /// Elle n'arrive pas à joindre son appareil et réessaie toute seule.
    Retrying { error: String, retry_at: SystemTime },
    /// Elle a planté (ou s'est arrêtée seule) : le superviseur la relance.
    Crashed { error: String, retry_at: SystemTime },
    /// Arrêtée proprement (arrêt du serveur).
    Stopped,
}

impl Health {
    /// Le nom court, pour l'API : `starting`, `connected`…
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Connected => "connected",
            Self::Retrying { .. } => "retrying",
            Self::Crashed { .. } => "crashed",
            Self::Stopped => "stopped",
        }
    }

    /// En panne : ses entités risquent de ne plus refléter la réalité.
    #[must_use]
    pub fn is_down(&self) -> bool {
        matches!(
            self,
            Self::Starting | Self::Retrying { .. } | Self::Crashed { .. }
        )
    }
}

/// Ce que l'on sait d'une intégration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationStatus {
    /// Identifiant court, propriétaire de ses entités : `hue`, `mqtt`…
    pub id: &'static str,
    /// Nom affiché : « Hue », « Somfy »…
    pub name: &'static str,
    pub health: Health,
    /// Depuis quand elle est dans cet état (connectée, ou en panne).
    pub since: SystemTime,
    /// Combien de fois le superviseur l'a relancée.
    pub restarts: u32,
    /// Ses entités ont été mises « indisponibles ».
    pub unavailable: bool,
    /// Combien d'entités elle publie.
    pub entities: usize,
}

#[derive(Debug)]
struct Entry {
    status: IntegrationStatus,
    /// La dernière valeur connue des entités mises « indisponibles ».
    held: HashMap<EntityId, (String, Attributes)>,
}

/// Le registre des intégrations, dans le [`Core`].
#[derive(Debug, Default)]
pub struct Integrations {
    entries: Mutex<BTreeMap<&'static str, Entry>>,
}

impl Integrations {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<&'static str, Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Core {
    /// Toutes les intégrations connues, par identifiant.
    #[must_use]
    pub fn integrations(&self) -> Vec<IntegrationStatus> {
        let entries = self.integrations.lock();
        entries
            .values()
            .map(|entry| IntegrationStatus {
                entities: self.states.owned_by(entry.status.id).len(),
                ..entry.status.clone()
            })
            .collect()
    }

    /// Une intégration dit où elle en est. Revenue « connectée », ses entités
    /// mises « indisponibles » retrouvent leur dernière valeur connue (sauf
    /// celles qu'elle a déjà republiées).
    pub fn report(&self, id: &'static str, name: &'static str, health: Health) {
        let mut entries = self.integrations.lock();
        let now = SystemTime::now();
        let entry = entries.entry(id).or_insert_with(|| Entry {
            status: IntegrationStatus {
                id,
                name,
                health: Health::Starting,
                since: now,
                restarts: 0,
                unavailable: false,
                entities: 0,
            },
            held: HashMap::new(),
        });
        let status = &mut entry.status;
        // « Depuis » ne repart pas à chaque nouvel essai : seulement quand on
        // passe de « en panne » à « connectée », ou l'inverse.
        if status.health.is_down() != health.is_down() || status.health == Health::Stopped {
            status.since = now;
        }
        if matches!(health, Health::Crashed { .. }) {
            status.restarts = status.restarts.saturating_add(1);
        }
        let back = health == Health::Connected && status.unavailable;
        status.health = health;
        if back {
            status.unavailable = false;
            for (entity_id, (value, attributes)) in std::mem::take(&mut entry.held) {
                let still_unavailable = self
                    .states
                    .get(&entity_id)
                    .is_some_and(|state| state.state() == UNAVAILABLE);
                if still_unavailable {
                    let _ = self.states.set_by(id, entity_id, value, attributes);
                }
            }
            tracing::info!("{name} : de retour, ses appareils aussi");
        }
    }

    /// À appeler régulièrement : les intégrations en panne depuis plus de
    /// [`GRACE`] voient leurs entités passer « indisponibles ».
    pub fn check_integrations(&self, now: SystemTime) {
        let mut entries = self.integrations.lock();
        for entry in entries.values_mut() {
            let status = &mut entry.status;
            let long_enough = now
                .duration_since(status.since)
                .is_ok_and(|down| down >= GRACE);
            if !status.health.is_down() || status.unavailable || !long_enough {
                continue;
            }
            status.unavailable = true;
            let mut count = 0;
            for entity_id in self.states.owned_by(status.id) {
                let Some(state) = self.states.get(&entity_id) else {
                    continue;
                };
                if state.state() == UNAVAILABLE {
                    continue;
                }
                entry.held.insert(
                    entity_id.clone(),
                    (state.state().to_owned(), state.attributes().clone()),
                );
                let _ = self.states.set_by(
                    status.id,
                    entity_id,
                    UNAVAILABLE,
                    state.attributes().clone(),
                );
                count += 1;
            }
            tracing::warn!(
                "{} : en panne depuis plus d'une minute, {count} appareils indisponibles",
                status.name
            );
        }
    }
}

/// Lance une intégration sous surveillance : si elle panique ou s'arrête
/// d'elle-même, elle est relancée après une attente croissante. `make`
/// fabrique une nouvelle exécution à chaque (re)démarrage ; elle reçoit le
/// jeton d'arrêt.
pub fn supervise<F, Fut>(
    core: Arc<Core>,
    id: &'static str,
    name: &'static str,
    mut make: F,
) -> Background
where
    F: FnMut(CancellationToken) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    core.report(id, name, Health::Starting);
    Background::spawn(name, move |token| async move {
        let mut wait = FIRST_RETRY;
        loop {
            let started = Instant::now();
            // Sa propre tâche : une panique y reste enfermée.
            let outcome = tokio::spawn(make(token.child_token())).await;
            if token.is_cancelled() {
                core.report(id, name, Health::Stopped);
                break;
            }
            let error = match outcome {
                Ok(()) => "s'est arrêtée d'elle-même".to_owned(),
                Err(error) if error.is_panic() => {
                    let panic = error.into_panic();
                    let text = panic
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_owned())
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "erreur inconnue".to_owned());
                    format!("a planté : {text}")
                }
                Err(error) => format!("a été interrompue : {error}"),
            };
            if started.elapsed() >= HEALTHY_RUN {
                wait = FIRST_RETRY;
            }
            tracing::error!("{name} {error} ; nouvel essai dans {} s", wait.as_secs());
            core.report(
                id,
                name,
                Health::Crashed {
                    error,
                    retry_at: SystemTime::now() + wait,
                },
            );
            tokio::select! {
                () = token.cancelled() => {
                    core.report(id, name, Health::Stopped);
                    break;
                }
                () = tokio::time::sleep(wait) => {}
            }
            wait = (wait * 2).min(MAX_RETRY);
            core.report(id, name, Health::Starting);
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use std::sync::atomic::{AtomicU32, Ordering};

    use serde_json::json;

    use super::*;

    fn id(text: &str) -> EntityId {
        text.parse().unwrap()
    }

    fn attrs() -> Attributes {
        json!({"friendly_name": "Salon"})
            .as_object()
            .unwrap()
            .clone()
    }

    #[test]
    fn a_long_outage_makes_its_entities_unavailable_then_restores_them() {
        let core = Core::new();
        core.report("hue", "Hue", Health::Connected);
        core.states
            .set_by("hue", id("light.salon"), "on", attrs())
            .unwrap();
        core.states
            .set_by("hue", id("light.cuisine"), "off", attrs())
            .unwrap();
        core.states
            .set(id("sensor.autre"), "12", Attributes::new())
            .unwrap();

        core.report(
            "hue",
            "Hue",
            Health::Retrying {
                error: "pont injoignable".into(),
                retry_at: SystemTime::now(),
            },
        );
        // Une coupure courte : rien ne change.
        core.check_integrations(SystemTime::now());
        assert_eq!(core.states.get(&id("light.salon")).unwrap().state(), "on");

        // Plus d'une minute : ses entités (et seulement elles) sont indisponibles.
        core.check_integrations(SystemTime::now() + GRACE + Duration::from_secs(1));
        assert_eq!(
            core.states.get(&id("light.salon")).unwrap().state(),
            UNAVAILABLE
        );
        assert_eq!(
            core.states.get(&id("light.salon")).unwrap().attributes(),
            &attrs()
        );
        assert_eq!(core.states.get(&id("sensor.autre")).unwrap().state(), "12");
        let status = &core.integrations()[0];
        assert!(status.unavailable);
        assert_eq!(status.entities, 2);

        // Elle revient : la cuisine a été republiée, le salon retrouve sa valeur.
        core.states
            .set_by("hue", id("light.cuisine"), "on", attrs())
            .unwrap();
        core.report("hue", "Hue", Health::Connected);
        assert_eq!(core.states.get(&id("light.salon")).unwrap().state(), "on");
        assert_eq!(core.states.get(&id("light.cuisine")).unwrap().state(), "on");
        assert!(!core.integrations()[0].unavailable);
    }

    #[test]
    fn a_clean_stop_never_makes_entities_unavailable() {
        let core = Core::new();
        core.states
            .set_by("mqtt", id("switch.prise"), "on", Attributes::new())
            .unwrap();
        core.report("mqtt", "MQTT", Health::Stopped);
        core.check_integrations(SystemTime::now() + GRACE * 2);
        assert_eq!(core.states.get(&id("switch.prise")).unwrap().state(), "on");
    }

    #[tokio::test(start_paused = true)]
    async fn a_panicking_integration_is_restarted_with_a_growing_wait() {
        let core = Arc::new(Core::new());
        let runs = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&runs);
        let task = supervise(Arc::clone(&core), "test", "Test", move |token| {
            let counter = Arc::clone(&counter);
            async move {
                let run = counter.fetch_add(1, Ordering::SeqCst);
                assert!(run >= 2, "panne simulée n° {run}");
                token.cancelled().await;
            }
        });
        // 1re exécution : panique ; relance après 1 s, panique ; après 2 s, tient.
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert!(matches!(
            core.integrations()[0].health,
            Health::Crashed { .. }
        ));
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 3);
        let status = &core.integrations()[0];
        assert_eq!(status.restarts, 2);
        match &status.health {
            Health::Starting => {}
            other => panic!("attendu : redémarrée, obtenu {other:?}"),
        }
        task.stop().await;
        assert_eq!(core.integrations()[0].health, Health::Stopped);
    }
}
