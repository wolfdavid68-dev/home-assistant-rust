//! Noyau domotique : bus d'événements, machine à états et entités.
//!
//! Concept Rust : chaque `mod` correspond à un fichier dans `src/`. Les `pub use`
//! remettent les types principaux à la racine, pour écrire `ha_core::EntityId`
//! au lieu de `ha_core::entity_id::EntityId`.

mod entity_id;
mod error;
mod event;
pub mod integrations;
mod origin;
mod state;
mod state_machine;
pub mod task;

pub use entity_id::{EntityId, MAX_ENTITY_ID_LEN, slugify};
pub use error::CoreError;
pub use event::{DEFAULT_BUS_CAPACITY, Event, EventBus};
pub use integrations::{Health, IntegrationStatus, Integrations, supervise};
pub use origin::Origin;
pub use state::{Attributes, MAX_STATE_LEN, State};
pub use state_machine::StateMachine;
pub use task::Background;
pub use tokio_util::sync::CancellationToken;

/// Un nom simple (domaine, service, identifiant d'automatisation…) :
/// 1 à 64 caractères parmi `a-z`, `0-9` et `_`.
///
/// Ces noms viennent souvent de l'extérieur (API, fichiers) et finissent
/// dans des logs : on n'accepte rien d'autre.
#[must_use]
pub fn is_valid_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Un nom d'événement, comme `timer.finished` : 1 à 64 caractères parmi
/// `a-z`, `0-9`, `_` et `.`, sans point au début ni à la fin.
#[must_use]
pub fn is_valid_event_type(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && !name.starts_with('.')
        && !name.ends_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.')
}

/// Version du noyau, reprise de `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Le noyau complet : un bus d'événements et la machine à états qui publie dessus.
///
/// Pour le partager entre plusieurs tâches, on l'enveloppe dans un `Arc<Core>`.
#[derive(Debug)]
pub struct Core {
    pub bus: EventBus,
    pub states: StateMachine,
    /// La santé des intégrations (voir [`integrations`]).
    pub integrations: Integrations,
}

impl Core {
    #[must_use]
    pub fn new() -> Self {
        let bus = EventBus::default();
        let states = StateMachine::new(bus.clone());
        Self {
            bus,
            states,
            integrations: Integrations::default(),
        }
    }
}

impl Core {
    /// Publie un appel de service sur le bus (voir [`Event::CallService`]).
    ///
    /// Le domaine et le service sont validés comme les identifiants : ils
    /// viennent souvent de l'extérieur (API) et finissent dans des logs.
    pub fn call_service(
        &self,
        domain: &str,
        service: &str,
        entity_id: EntityId,
        data: Attributes,
    ) -> Result<(), CoreError> {
        self.call_service_from(Origin::System, domain, service, entity_id, data)
    }

    /// Comme [`Core::call_service`], en disant qui le demande.
    pub fn call_service_from(
        &self,
        origin: Origin,
        domain: &str,
        service: &str,
        entity_id: EntityId,
        data: Attributes,
    ) -> Result<(), CoreError> {
        for name in [domain, service] {
            if !is_valid_name(name) {
                return Err(CoreError::InvalidServiceName(name.to_owned()));
            }
        }
        self.bus.publish(Event::CallService {
            domain: domain.to_owned(),
            service: service.to_owned(),
            entity_id,
            data,
            origin,
        });
        Ok(())
    }

    /// Publie un événement nommé (voir [`Event::Fired`]).
    pub fn fire(&self, event_type: &str, data: Attributes) -> Result<(), CoreError> {
        if !is_valid_event_type(event_type) {
            return Err(CoreError::InvalidEventType(event_type.to_owned()));
        }
        self.bus.publish(Event::Fired {
            event_type: event_type.to_owned(),
            data,
        });
        Ok(())
    }
}

impl Default for Core {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_events_are_checked_and_published() {
        let core = Core::new();
        let mut events = core.bus.subscribe();
        for good in ["timer.finished", "mon_evenement", "a.b.c"] {
            assert!(core.fire(good, Attributes::new()).is_ok(), "{good}");
        }
        for bad in ["", "Timer.finished", ".x", "x.", "a b", &"a".repeat(65)] {
            assert!(core.fire(bad, Attributes::new()).is_err(), "{bad}");
        }
        let Ok(event) = events.try_recv() else {
            panic!("un événement est attendu");
        };
        assert_eq!(
            *event,
            Event::Fired {
                event_type: "timer.finished".into(),
                data: Attributes::new()
            }
        );
    }
}
