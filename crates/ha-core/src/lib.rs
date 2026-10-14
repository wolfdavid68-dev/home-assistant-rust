//! Noyau domotique : bus d'événements, machine à états et entités.
//!
//! Concept Rust : chaque `mod` correspond à un fichier dans `src/`. Les `pub use`
//! remettent les types principaux à la racine, pour écrire `ha_core::EntityId`
//! au lieu de `ha_core::entity_id::EntityId`.

mod entity_id;
mod error;
mod event;
mod state;
mod state_machine;

pub use entity_id::{EntityId, MAX_ENTITY_ID_LEN, slugify};
pub use error::CoreError;
pub use event::{DEFAULT_BUS_CAPACITY, Event, EventBus};
pub use state::{Attributes, MAX_STATE_LEN, State};
pub use state_machine::StateMachine;

/// Version du noyau, reprise de `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Le noyau complet : un bus d'événements et la machine à états qui publie dessus.
///
/// Pour le partager entre plusieurs tâches, on l'enveloppe dans un `Arc<Core>`.
#[derive(Debug)]
pub struct Core {
    pub bus: EventBus,
    pub states: StateMachine,
}

impl Core {
    pub fn new() -> Self {
        let bus = EventBus::default();
        let states = StateMachine::new(bus.clone());
        Self { bus, states }
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
        for name in [domain, service] {
            let valid = (1..=64).contains(&name.len())
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
            if !valid {
                return Err(CoreError::InvalidServiceName(name.to_owned()));
            }
        }
        self.bus.publish(Event::CallService {
            domain: domain.to_owned(),
            service: service.to_owned(),
            entity_id,
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
